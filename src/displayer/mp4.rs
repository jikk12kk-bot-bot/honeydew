//! The MP4 render (the Save dialog's MP4): the song's sound, rendered as an audio render renders it, with the
//! Displayer's picture: its image, its video (trimmed, looped or black after its end, from the song's start as it
//! plays), or black without either. No title, nothing else on it. With `Include video sound` the video's sound joins
//! the song's at the Displayer's volume; otherwise the file holds the song's sound alone.
//!
//! The export job ([`crate::project_files::export::Job`]) hands each block of the song's processed sound to
//! [`Video::push`], which writes the pictures due by then; [`Video::finish`] ends the file. Without the `video`
//! feature (the web) there is no encoder, and no `Video` can be made.

#[cfg(feature = "video")]
pub use real::Video;

#[cfg(not(feature = "video"))]
pub use none::Video;

/// Where `src` (`sw × sh`) goes in a `w × h` frame by `fit`: its drawn size and its top-left corner (negative when a
/// Fill overflows).
pub fn layout(src: (u32, u32), frame: (u32, u32), fit: super::Fit) -> ((u32, u32), (i64, i64)) {
    let room = eframe::egui::Rect::from_min_size(
        eframe::egui::Pos2::ZERO,
        eframe::egui::vec2(frame.0 as f32, frame.1 as f32),
    );
    let r = super::placed((src.0 as f32, src.1 as f32), room, fit);
    let even = |v: f32| ((v / 2.0).round() as u32 * 2).max(2);
    let size = (even(r.width()), even(r.height()));
    let at = (
        (i64::from(frame.0) - i64::from(size.0)) / 2,
        (i64::from(frame.1) - i64::from(size.1)) / 2,
    );
    (size, at)
}

/// Copies `src` (`sw × sh` RGBA) into `canvas` (`w × h` RGBA) at `at`, clipped.
pub fn blit(
    canvas: &mut [u8],
    (w, h): (u32, u32),
    src: &[u8],
    (sw, sh): (u32, u32),
    at: (i64, i64),
) {
    let (w, h, sw, sh) = (i64::from(w), i64::from(h), i64::from(sw), i64::from(sh));
    if src.len() < (sw * sh * 4) as usize || canvas.len() < (w * h * 4) as usize {
        return;
    }
    let x0 = at.0.max(0);
    let x1 = (at.0 + sw).min(w);
    if x1 <= x0 {
        return;
    }
    for y in at.1.max(0)..(at.1 + sh).min(h) {
        let sy = y - at.1;
        let from = ((sy * sw + (x0 - at.0)) * 4) as usize;
        let to = ((y * w + x0) * 4) as usize;
        let len = ((x1 - x0) * 4) as usize;
        canvas[to..to + len].copy_from_slice(&src[from..from + len]);
    }
}

/// A black RGBA frame.
pub fn black((w, h): (u32, u32)) -> Vec<u8> {
    let mut v = vec![0; (w * h * 4) as usize];
    v.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    v
}

#[cfg(not(feature = "video"))]
mod none {
    use std::path::Path;

    use super::super::DisplayerSettings;

    /// No encoder in this build: never made.
    pub struct Video(std::convert::Infallible);

    impl Video {
        pub fn new(_path: &Path, _s: &DisplayerSettings, _rate: u32) -> Result<Self, String> {
            Err("This version cannot write video".into())
        }

        pub fn push(&mut self, _left: &[f32], _right: &[f32]) {
            match self.0 {}
        }

        pub fn finish(self) -> Result<(), String> {
            match self.0 {}
        }
    }
}

#[cfg(feature = "video")]
mod real {
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};

    use super::super::{DisplayerSettings, Fit, Source, Trim};
    use super::{black, blit, layout};
    use crate::video::{Decoder, Encoder, MediaFile};

    /// An MP4 being written.
    pub struct Video {
        /// Taken when the file is finished; a video dropped unfinished (cancelled, failed) deletes its file.
        encoder: Option<Encoder>,
        path: PathBuf,
        size: (u32, u32),
        fps: u32,
        rate: u32,
        /// Sound frames and pictures written.
        sound_frames: u64,
        pictures: u64,
        picture: Pictures,
        sound: Option<Sound>,
        /// Mixed sound, the channels apart.
        left: Vec<f32>,
        right: Vec<f32>,
        error: Option<String>,
    }

    impl Video {
        /// The MP4 at `path` of what `s` shows, its sound at `rate` Hz. The folder is made if needed.
        pub fn new(path: &Path, s: &DisplayerSettings, rate: u32) -> Result<Self, String> {
            let o = s.mp4;
            let size = o.size();
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            let picture = match (s.source, s.file()) {
                (Source::Image, Some(file)) => {
                    match super::super::media::decode_image(file, 4096) {
                        Ok(img) => Pictures::still(&img, size, o.fit),
                        Err(_) => Pictures::Black(black(size)),
                    }
                }
                (Source::Video, Some(file)) => match MediaFile::open(file) {
                    Some(f) => Pictures::clip(&f, s, size, o.fit)
                        .unwrap_or_else(|| Pictures::Black(black(size))),
                    None => Pictures::Black(black(size)),
                },
                _ => Pictures::Black(black(size)),
            };
            let sound = match (&picture, o.video_sound, s.source, s.file()) {
                (Pictures::Clip(clip), true, Source::Video, Some(file)) => MediaFile::open(file)
                    .and_then(|f| Decoder::open(&f, true, false, rate).ok())
                    .map(|dec| Sound {
                        dec,
                        trim: clip.trim,
                        gain: s.volume,
                        queue: VecDeque::new(),
                        expected: None,
                        end: false,
                        rate,
                    }),
                _ => None,
            };
            let encoder = Encoder::create(path, size.0, size.1, o.fps(), rate)?;
            Ok(Self {
                encoder: Some(encoder),
                path: path.to_owned(),
                size,
                fps: o.fps(),
                rate,
                sound_frames: 0,
                pictures: 0,
                picture,
                sound,
                left: Vec::new(),
                right: Vec::new(),
                error: None,
            })
        }

        /// The next block of the song's sound: mixed with the video's when it is included, encoded, and the pictures
        /// due by its end written.
        pub fn push(&mut self, left: &[f32], right: &[f32]) {
            if self.error.is_some() {
                return;
            }
            let n = left.len().min(right.len());
            self.left.clear();
            self.right.clear();
            self.left.extend_from_slice(&left[..n]);
            self.right.extend_from_slice(&right[..n]);
            if let Some(sound) = &mut self.sound {
                for i in 0..n {
                    let secs = (self.sound_frames + i as u64) as f64 / f64::from(self.rate);
                    let [l, r] = sound.at(secs);
                    self.left[i] += l * sound.gain;
                    self.right[i] += r * sound.gain;
                }
            }
            let Some(encoder) = &mut self.encoder else {
                return;
            };
            if let Err(e) = encoder.sound(&self.left, &self.right) {
                self.error = Some(e);
                return;
            }
            self.sound_frames += n as u64;
            self.pictures_due();
        }

        /// Pictures up to the sound written so far.
        fn pictures_due(&mut self) {
            let (fps, rate) = (u64::from(self.fps), u64::from(self.rate));
            while self.error.is_none() && self.pictures * rate < self.sound_frames * fps {
                let secs = self.pictures as f64 / fps as f64;
                let frame = self.picture.at(secs, self.size);
                let Some(encoder) = &mut self.encoder else {
                    return;
                };
                if let Err(e) = encoder.picture(frame) {
                    self.error = Some(e);
                }
                self.pictures += 1;
            }
        }

        /// The last pictures, and the file's end.
        pub fn finish(mut self) -> Result<(), String> {
            self.pictures_due();
            if let Some(e) = self.error.take() {
                return Err(e);
            }
            if self.pictures == 0 {
                return Err("The render has no length".into());
            }
            let encoder = self.encoder.take().ok_or("The video was already written")?;
            let written = encoder.finish();
            if written.is_ok() {
                // Kept.
                self.path.clear();
            }
            written
        }
    }

    impl Drop for Video {
        fn drop(&mut self) {
            // The encoder closes the file first.
            drop(self.encoder.take());
            if !self.path.as_os_str().is_empty() {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }

    /// Where the pictures come from.
    enum Pictures {
        /// No image or video (or one that cannot be read): black.
        Black(Vec<u8>),
        /// The image, placed in its frame once.
        Still(Vec<u8>),
        Clip(Box<Clip>),
    }

    impl Pictures {
        fn still(img: &image::RgbaImage, size: (u32, u32), fit: Fit) -> Self {
            let ((w, h), at) = layout(img.dimensions(), size, fit);
            let scaled = image::imageops::resize(img, w, h, image::imageops::FilterType::Triangle);
            let mut canvas = black(size);
            blit(&mut canvas, size, scaled.as_raw(), (w, h), at);
            Self::Still(canvas)
        }

        fn clip(
            file: &std::sync::Arc<MediaFile>,
            s: &DisplayerSettings,
            size: (u32, u32),
            fit: Fit,
        ) -> Option<Self> {
            let mut dec = Decoder::open(file, false, true, 0)
                .or_else(|_| Decoder::open(file, false, false, 0))
                .ok()?;
            let (drawn, at) = layout(dec.size, size, fit);
            dec.set_size(drawn.0, drawn.1);
            let trim = s.trim(dec.duration);
            Some(Self::Clip(Box::new(Clip {
                dec,
                file: file.clone(),
                trim,
                drawn,
                at,
                canvas: black(size),
                black: black(size),
                shown: None,
                next: None,
                last: None,
                end: false,
                failures: 0,
            })))
        }

        /// The frame at song second `secs`.
        fn at(&mut self, secs: f64, size: (u32, u32)) -> &[u8] {
            match self {
                Self::Black(b) | Self::Still(b) => b.as_slice(),
                Self::Clip(clip) => clip.at(secs, size),
            }
        }
    }

    /// The video's pictures, decoded in order, sought again when the trim loops.
    struct Clip {
        dec: Decoder,
        file: std::sync::Arc<MediaFile>,
        trim: Trim,
        drawn: (u32, u32),
        at: (i64, i64),
        /// The frame with the picture shown, and a black one.
        canvas: Vec<u8>,
        black: Vec<u8>,
        /// The video second of the picture in `canvas`.
        shown: Option<f64>,
        /// A picture decoded that was not due yet.
        next: Option<(Vec<u8>, f64)>,
        last: Option<f64>,
        end: bool,
        failures: u32,
    }

    impl Clip {
        fn at(&mut self, secs: f64, size: (u32, u32)) -> &[u8] {
            let Some(want) = self.trim.at(secs) else {
                return &self.black;
            };
            // Back in time (the loop came round): from the key frame before.
            if self.shown.is_some_and(|s| want + 0.001 < s)
                || self.shown.is_none() && self.last.is_none()
            {
                self.dec.seek(want);
                (self.shown, self.next, self.last, self.end) = (None, None, None, false);
            }
            let eps = 0.0005;
            let mut newest: Option<Vec<u8>> = None;
            if let Some((pixels, t)) = self.next.take() {
                if t <= want + eps {
                    self.shown = Some(t);
                    newest = Some(pixels);
                } else {
                    self.next = Some((pixels, t));
                }
            }
            while self.next.is_none() && !self.end {
                match self.dec.picture(false) {
                    Ok(Some(pic)) => {
                        self.failures = 0;
                        let t = if pic.secs >= 0.0 {
                            pic.secs
                        } else {
                            self.last.map_or(0.0, |l| l + 1.0 / 30.0)
                        };
                        self.last = Some(t);
                        let pixels = pic.rgba.to_vec();
                        if t <= want + eps {
                            self.shown = Some(t);
                            newest = Some(pixels);
                        } else {
                            // Before any picture is due (just sought), the first one shows.
                            if self.shown.is_none() && newest.is_none() {
                                self.shown = Some(want);
                                newest = Some(pixels.clone());
                            }
                            self.next = Some((pixels, t));
                        }
                    }
                    Ok(None) => self.end = true,
                    Err(_) => {
                        self.failures += 1;
                        if self.failures >= 8 && self.dec.hardware {
                            if let Ok(mut sw) = Decoder::open(&self.file, false, false, 0) {
                                sw.set_size(self.drawn.0, self.drawn.1);
                                sw.seek(want);
                                self.dec = sw;
                                (self.shown, self.next, self.last, self.failures) =
                                    (None, None, None, 0);
                                continue;
                            }
                        }
                        if self.failures >= 32 {
                            self.end = true;
                        }
                    }
                }
            }
            if let Some(pixels) = newest {
                if pixels.len() == (self.drawn.0 * self.drawn.1 * 4) as usize {
                    self.canvas.copy_from_slice(&self.black);
                    blit(&mut self.canvas, size, &pixels, self.drawn, self.at);
                }
            }
            &self.canvas
        }
    }

    /// The video's sound for the file, sample by sample in song time.
    struct Sound {
        dec: Decoder,
        trim: Trim,
        gain: f32,
        /// Decoded stereo frames from `expected` on.
        queue: VecDeque<[f32; 2]>,
        /// The video second of the queue's first frame.
        expected: Option<f64>,
        end: bool,
        rate: u32,
    }

    impl Sound {
        /// The video's sound at song second `secs`.
        fn at(&mut self, secs: f64) -> [f32; 2] {
            let Some(want) = self.trim.at(secs) else {
                return [0.0; 2];
            };
            let frame = 1.0 / f64::from(self.rate);
            if self.expected.is_none_or(|e| (e - want).abs() > 1.5 * frame) {
                self.resync(want);
            }
            if self.queue.is_empty() {
                self.fill();
            }
            self.expected = Some(want + frame);
            self.queue.pop_front().unwrap_or([0.0; 2])
        }

        fn resync(&mut self, want: f64) {
            self.dec.seek(want);
            self.queue.clear();
            self.end = false;
            self.expected = Some(want);
            let rate = f64::from(self.rate);
            let mut budget = 64;
            while self.queue.is_empty() && !self.end && budget > 0 {
                budget -= 1;
                match self.dec.sound() {
                    Ok(Some((pcm, at))) => {
                        let frames = pcm.len() / 2;
                        let at = if at.is_finite() { at } else { want };
                        let first = ((want - at) * rate).round();
                        if first >= frames as f64 {
                            continue;
                        }
                        if first < 0.0 {
                            let gap = (-first) as usize;
                            self.queue
                                .extend(std::iter::repeat_n([0.0; 2], gap.min(self.rate as usize)));
                        }
                        let skip = first.max(0.0) as usize;
                        self.queue
                            .extend(pcm.chunks_exact(2).skip(skip).map(|c| [c[0], c[1]]));
                    }
                    Ok(None) | Err(_) => self.end = true,
                }
            }
        }

        fn fill(&mut self) {
            let mut budget = 16;
            while self.queue.is_empty() && !self.end && budget > 0 {
                budget -= 1;
                match self.dec.sound() {
                    Ok(Some((pcm, _))) => {
                        self.queue.extend(pcm.chunks_exact(2).map(|c| [c[0], c[1]]))
                    }
                    Ok(None) | Err(_) => self.end = true,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::displayer::Fit;

    #[test]
    fn a_wide_picture_fits_a_tall_frame_with_bars() {
        let (size, at) = layout((1920, 1080), (1080, 1920), Fit::Fit);
        assert_eq!(size, (1080, 608));
        assert_eq!(at, (0, (1920 - 608) / 2));
        let (size, at) = layout((1920, 1080), (1080, 1920), Fit::Fill);
        assert_eq!(size.1, 1920);
        assert!(at.0 < 0);
    }

    #[test]
    fn blit_clips() {
        let mut canvas = black((4, 4));
        let src = vec![9u8; 6 * 2 * 4];
        blit(&mut canvas, (4, 4), &src, (6, 2), (-1, 1));
        assert_eq!(canvas[(4 * 4) as usize], 9);
        assert_eq!(canvas[0], 0);
    }
}
