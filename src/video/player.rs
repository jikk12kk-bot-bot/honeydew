//! The Displayer's video player: a picture thread that keeps the frame for the time the app asks for, decoding ahead
//! and seeking when the time jumps, and a sound thread that feeds the listen-only output
//! ([`honeydew_audio::listen::FEED`]) in step with the song.
//!
//! The app sets a [`Target`] each frame and takes the newest [`Frame`] when its serial changes. Both threads end when
//! the player drops. The file opens on the picture thread (an Android document's descriptor comes through JNI, which
//! can take a while), and only once the player before has closed its decoders: one video's decoders at a time, so a
//! change of video never runs two MediaCodec sessions, or two writers of the listen feed, side by side.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use honeydew_audio::listen::FEED;

use super::{Decoder, MediaFile};
use crate::displayer::Trim;

/// What the app wants this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    /// The picture at this video second; `None`: black (the trimmed part over, unlooped).
    pub picture: Option<f64>,
    /// The song's second (from its start) and whether it plays: the sound follows them.
    pub song_secs: f64,
    pub playing: bool,
    pub trim: Trim,
    /// The sound's level, 0 when muted.
    pub gain: f32,
}

impl Default for Target {
    fn default() -> Self {
        Self {
            picture: Some(0.0),
            song_secs: 0.0,
            playing: false,
            trim: Trim::default(),
            gain: 0.0,
        }
    }
}

/// The newest picture.
pub struct Frame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Changes with every new picture (and with black).
    pub serial: u64,
    /// Black: the trimmed part is over.
    pub black: bool,
}

/// How the opening went.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Opening,
    /// The file is not where it was.
    NotFound,
    /// Its length in seconds, its display size, and whether MediaCodec decodes it.
    Ready {
        duration: f64,
        size: (u32, u32),
        hardware: bool,
    },
    Failed(String),
}

struct Shared {
    target: Mutex<Target>,
    frame: Mutex<Frame>,
    status: Mutex<Status>,
    stop: AtomicBool,
    ctx: eframe::egui::Context,
}

/// A playing video.
pub struct Player {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

/// The picture threads of the players dropped, still closing: the next player waits for them before it opens.
static ENDING: Mutex<Vec<JoinHandle<()>>> = Mutex::new(Vec::new());

/// The longest side of the pictures the player decodes to: the window and the background never need more.
#[cfg(any(target_os = "android", target_os = "ios"))]
const MAX_SIDE: u32 = 1280;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const MAX_SIDE: u32 = 1920;

impl Player {
    /// Starts playing the file at `location` (a path, or an Android document URI); `ctx` repaints for each new
    /// picture.
    pub fn start(location: String, ctx: eframe::egui::Context) -> Self {
        let shared = Arc::new(Shared {
            target: Mutex::new(Target::default()),
            frame: Mutex::new(Frame {
                rgba: Vec::new(),
                width: 0,
                height: 0,
                serial: 0,
                black: true,
            }),
            status: Mutex::new(Status::Opening),
            stop: AtomicBool::new(false),
            ctx,
        });
        let s = shared.clone();
        let thread = std::thread::Builder::new()
            .name("displayer picture".into())
            .spawn(move || run(&s, &location));
        let thread = match thread {
            Ok(t) => Some(t),
            Err(err) => {
                *lock(&shared.status) = Status::Failed(err.to_string());
                None
            }
        };
        Self { shared, thread }
    }

    pub fn set_target(&self, target: Target) {
        *lock(&self.shared.target) = target;
    }

    pub fn status(&self) -> Status {
        lock(&self.shared.status).clone()
    }

    /// Calls `f` with the newest picture when its serial is not `seen`.
    pub fn with_frame(&self, seen: u64, f: impl FnOnce(&Frame)) {
        let frame = lock(&self.shared.frame);
        if frame.serial != seen {
            f(&frame);
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        // The thread sees the stop within a few ms; the next player joins it (`ENDING`), never the UI thread.
        if let Some(t) = self.thread.take() {
            let mut ending = lock(&ENDING);
            ending.retain(|t| !t.is_finished());
            ending.push(t);
        }
    }
}

/// A lock, through a panic that poisoned it: the data are plain values, whole at every step.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The picture thread: waits for the players before to close, opens the file, starts the sound thread and shows
/// pictures until the stop; the sound thread is joined before it ends.
fn run(shared: &Arc<Shared>, location: &str) {
    let before = std::mem::take(&mut *lock(&ENDING));
    let me = std::thread::current().id();
    // This player's own thread is there when it was dropped before it got here.
    for t in before.into_iter().filter(|t| t.thread().id() != me) {
        let _ = t.join();
    }
    if shared.stop.load(Ordering::Acquire) {
        return;
    }
    let Some(file) = MediaFile::open(location) else {
        *lock(&shared.status) = Status::NotFound;
        shared.ctx.request_repaint();
        return;
    };
    let s = shared.clone();
    let f = file.clone();
    let sound = std::thread::Builder::new()
        .name("displayer sound".into())
        .spawn(move || sound_thread(&s, &f))
        .ok();
    picture_thread(shared, &file);
    shared.stop.store(true, Ordering::Release);
    if let Some(t) = sound {
        let _ = t.join();
    }
}

/// Fits `size` within [`MAX_SIDE`], even sides.
fn decode_size((w, h): (u32, u32)) -> (u32, u32) {
    let (w, h) = (w.max(2), h.max(2));
    let k = (MAX_SIDE as f64 / w.max(h) as f64).min(1.0);
    let even = |v: f64| ((v / 2.0).round() as u32 * 2).max(2);
    (even(w as f64 * k), even(h as f64 * k))
}

fn picture_thread(shared: &Shared, file: &Arc<MediaFile>) {
    let opened =
        Decoder::open(file, false, true, 0).or_else(|_| Decoder::open(file, false, false, 0));
    let mut dec = match opened {
        Ok(d) => d,
        Err(err) => {
            *lock(&shared.status) = Status::Failed(err);
            shared.ctx.request_repaint();
            return;
        }
    };
    let size = decode_size(dec.size);
    dec.set_size(size.0, size.1);
    let ready = Status::Ready {
        duration: dec.duration,
        size: dec.size,
        hardware: dec.hardware,
    };
    *lock(&shared.status) = ready;
    let mut p = Pictures {
        dec,
        size,
        shown: None,
        next: None,
        last: None,
        step: 1.0 / 30.0,
        end: false,
        failures: 0,
        file: file.clone(),
    };
    let mut black_sent = false;
    while !shared.stop.load(Ordering::Acquire) {
        let target = *lock(&shared.target);
        let Some(want) = target.picture else {
            if !black_sent {
                publish(shared, None);
                black_sent = true;
                // Nothing decoded so far counts any more: the next picture asked for seeks afresh. A picture kept
                // ahead (`next`, past the part's end) would otherwise hold back every earlier one for good, as a jump
                // back to before it was never seen as one.
                (p.shown, p.next) = (None, None);
            }
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        black_sent = false;
        match p.advance(want, &shared.stop) {
            Some(Some(frame)) => publish(shared, Some(frame)),
            Some(None) => {}
            None => std::thread::sleep(Duration::from_millis(4)),
        }
    }
}

fn publish(shared: &Shared, frame: Option<(Vec<u8>, u32, u32)>) {
    {
        let mut f = lock(&shared.frame);
        f.serial += 1;
        match frame {
            Some((rgba, w, h)) => {
                (f.rgba, f.width, f.height, f.black) = (rgba, w, h, false);
            }
            None => f.black = true,
        }
    }
    shared.ctx.request_repaint();
}

/// The picture thread's decoder and what it has decoded.
struct Pictures {
    dec: Decoder,
    size: (u32, u32),
    /// The time of the picture published last.
    shown: Option<f64>,
    /// A decoded picture not yet due: its pixels and time.
    next: Option<(Vec<u8>, f64)>,
    /// The time of the picture decoded last, and the step between pictures as measured.
    last: Option<f64>,
    step: f64,
    end: bool,
    failures: u32,
    file: Arc<MediaFile>,
}

impl Pictures {
    /// Decodes up to the picture for video second `want`. `None`: nothing to do (wait); `Some(None)`: worked, nothing
    /// new to show; `Some(Some(_))`: the picture to show. Stops early at `stop`.
    fn advance(&mut self, want: f64, stop: &AtomicBool) -> Option<Option<(Vec<u8>, u32, u32)>> {
        let ahead = self.next.as_ref().map(|(_, t)| *t).or(self.shown);
        let jump = match (self.shown, ahead) {
            (None, None) => true,
            (Some(shown), _) if want < shown - 0.25 * self.step - 0.01 => true,
            (_, Some(at)) => want > at + 1.5,
            _ => false,
        };
        if jump {
            self.dec.seek(want);
            (self.next, self.last, self.end) = (None, None, false);
            self.shown = None;
        }
        // As the step was measured on entry: the loop below refines it.
        let slack = 0.5 * self.step;
        let due = move |t: f64| t <= want + slack;
        if self.next.as_ref().is_some_and(|(_, t)| !due(*t)) {
            return None;
        }
        if self.end && self.next.is_none() {
            return None;
        }
        // Decode until a picture is past `want`; show the last one due.
        let mut show: Option<(Vec<u8>, f64)> = self.next.take().filter(|(_, t)| due(*t));
        let mut budget = 240;
        while budget > 0 && !stop.load(Ordering::Acquire) {
            budget -= 1;
            // Skip converting a picture another surely follows before `want`.
            let skip = self.last.is_some_and(|l| l + 2.5 * self.step < want);
            match self.dec.picture(skip) {
                Ok(Some(pic)) => {
                    self.failures = 0;
                    let t = if pic.secs >= 0.0 {
                        pic.secs
                    } else {
                        self.last.map_or(0.0, |l| l + self.step)
                    };
                    if let Some(l) = self.last {
                        let d = t - l;
                        if d > 0.001 && d < 0.5 {
                            self.step = 0.9 * self.step + 0.1 * d;
                        }
                    }
                    self.last = Some(t);
                    if skip {
                        continue;
                    }
                    let expected = (self.size.0 as usize) * (self.size.1 as usize) * 4;
                    if pic.rgba.len() != expected {
                        // Not the size asked for: nothing to show of it.
                        continue;
                    }
                    let pixels = pic.rgba.to_vec();
                    if due(t) {
                        show = Some((pixels, t));
                    } else {
                        self.next = Some((pixels, t));
                        break;
                    }
                }
                Ok(None) => {
                    self.end = true;
                    break;
                }
                Err(_) => {
                    self.failures += 1;
                    // MediaCodec that opened but cannot decode this file: FFmpeg's own decoder instead.
                    if self.failures >= 8 && self.dec.hardware {
                        if let Ok(mut sw) = Decoder::open(&self.file, false, false, 0) {
                            sw.set_size(self.size.0, self.size.1);
                            self.dec = sw;
                            self.dec.seek(want);
                            (self.next, self.last, self.shown, self.failures) =
                                (None, None, None, 0);
                            continue;
                        }
                    }
                    if self.failures >= 32 {
                        self.end = true;
                        break;
                    }
                }
            }
        }
        match show {
            Some((rgba, t)) => {
                self.shown = Some(t);
                Some(Some((rgba, self.size.0, self.size.1)))
            }
            None => Some(None),
        }
    }
}

/// How far ahead the sound thread keeps the feed, in seconds.
const LEAD: f64 = 0.3;

fn sound_thread(shared: &Shared, file: &Arc<MediaFile>) {
    let mut rate = 0;
    let mut dec: Option<Decoder> = None;
    // Since the last flush: the song second it started at and the frames written from there.
    let mut from: Option<(f64, u64)> = None;
    let mut silence = Vec::new();
    let mut reseeks = 0;
    // Silent frames owed before the decoder's next sound: a looped file whose sound ends before its trimmed end (the
    // container's length often runs past the sound's) is silent up to that end, then wraps to the trim's start, where
    // the decoder already is.
    let mut owed = 0usize;
    while !shared.stop.load(Ordering::Acquire) {
        let target = *lock(&shared.target);
        FEED.set_gain(target.gain);
        let now_rate = FEED.rate();
        if now_rate == 0 {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        if now_rate != rate || dec.is_none() {
            rate = now_rate;
            dec = match Decoder::open(file, true, false, rate) {
                Ok(d) => Some(d),
                // No sound in the file (or none we can play): nothing to feed, ever.
                Err(_) => return,
            };
            from = None;
        }
        let Some(d) = dec.as_mut() else { continue };
        if !target.playing || target.gain <= 0.0 {
            if from.take().is_some() {
                FEED.flush();
            }
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        let r = f64::from(rate);
        // Where the callback is, since the flush, against the song's clock: off by more than a moment (a seek, a loop,
        // a stop and play) starts again from the song's second.
        let drifted = from.is_some_and(|(start, _)| {
            let played = start + FEED.consumed() as f64 / r;
            (target.song_secs - played).abs() > 0.35
        });
        if from.is_none() || drifted {
            let generation = FEED.flush();
            let mut waited = 0;
            while !FEED.flushed(generation) && waited < 200 && !shared.stop.load(Ordering::Acquire)
            {
                std::thread::sleep(Duration::from_millis(2));
                waited += 1;
            }
            let start = lock(&shared.target).song_secs;
            from = Some((start, 0));
            owed = 0;
            if let Some(v) = target.trim.at(start) {
                d.seek(v);
            }
        }
        let Some((start, written)) = from.as_mut() else {
            continue;
        };
        let mut wrote = false;
        while (FEED.queued() as f64) < LEAD * r && !shared.stop.load(Ordering::Acquire) {
            let song = *start + *written as f64 / r;
            let Some(v) = target.trim.at(song) else {
                // The trimmed part is over: silence, which keeps the count.
                silence.resize(1024, 0.0);
                *written += FEED.push(&silence) as u64;
                wrote = true;
                continue;
            };
            if owed > 0 {
                silence.resize(2 * owed.min(1024), 0.0);
                let n = FEED.push(&silence);
                owed -= n.min(owed);
                *written += n as u64;
                wrote = true;
                if n == 0 {
                    std::thread::sleep(Duration::from_millis(5));
                }
                continue;
            }
            match d.sound() {
                Ok(Some((pcm, secs))) => {
                    let frames = pcm.len() / 2;
                    // A loop's wrap: the decoder is past where the song wants it, or far before it (rather than
                    // decoding the whole file to drop it). Twice at most in a row: a file whose sound starts late
                    // lands past `v` after every seek.
                    if (secs > v + 0.25 || secs + 1.0 < v) && reseeks < 2 {
                        reseeks += 1;
                        d.seek(v);
                        continue;
                    }
                    reseeks = 0;
                    // Before `v` (after a seek): dropped.
                    let skip = (((v - secs) * r).round().max(0.0) as usize).min(frames);
                    // Not past the trimmed end.
                    let left = ((target.trim.end - v) * r).ceil().max(0.0) as usize;
                    let take = (frames - skip).min(left.max(1));
                    let chunk = &pcm[2 * skip..2 * (skip + take)];
                    let mut off = 0;
                    while off < chunk.len() && !shared.stop.load(Ordering::Acquire) {
                        let n = FEED.push(&chunk[off..]);
                        off += 2 * n;
                        *written += n as u64;
                        if n == 0 {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                    }
                    wrote = true;
                    if take < frames - skip && target.trim.looped {
                        d.seek(target.trim.start);
                    }
                }
                Ok(None) => {
                    if target.trim.looped && target.trim.end.is_finite() {
                        d.seek(target.trim.start);
                        // A file whose sound ends before its trimmed end: silence up to the end, all of it (the
                        // decoder is at the start already: a part short of it dropped the whole file's sound again).
                        owed = ((target.trim.end - v) * r).max(0.0) as usize;
                    } else {
                        silence.resize(1024, 0.0);
                        *written += FEED.push(&silence) as u64;
                    }
                    wrote = true;
                }
                Err(_) => {
                    silence.resize(1024, 0.0);
                    *written += FEED.push(&silence) as u64;
                }
            }
        }
        if !wrote {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    FEED.flush();
}
