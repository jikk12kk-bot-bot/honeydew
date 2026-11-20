//! The Displayer's video sound: listen-only, added to the output stream after the whole mix (the master, its effects
//! and its limiter), so that it is heard while the song plays and never reaches a render, which runs the renderer on its
//! own and never this stream's callback.
//!
//! One producer (the Displayer's sound thread) and one consumer (the output callback) share [`FEED`], a lock-free ring
//! of interleaved stereo frames:
//! - The producer [`ListenFeed::flush`]es when the song starts or jumps, waits for the callback to take the flush
//!   ([`ListenFeed::flushed`]), and then writes from the song second it saw. The callback counts every frame it plays
//!   from there, or would have played had the sound been there ([`ListenFeed::consumed`]): sound that comes late is
//!   skipped by as much, so the video's sound keeps to the song's clock.
//! - Only while the transport moves does the callback take frames; stopped (or holding for a count-in) it takes none.
//! - The callback publishes the transport tick its block ended at ([`ListenFeed::tick`]) and the one it took the last
//!   flush at ([`ListenFeed::flushed_tick`]): the producer keeps to the song's clock from them, not from the app's
//!   frames, which stop while the app is in the background (the sound looped on its last second there).
//!
//! Part of Honeydew (the Displayer, our own addition).

use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// The ring's room, in stereo frames: 0.68 s at 48 kHz, 0.17 s more than the producer keeps queued.
const CAP: usize = 1 << 15;

/// The feed.
pub struct ListenFeed {
    buf: [AtomicU32; CAP * 2],
    /// Frames written and read so far (wrapping): the producer's and the callback's.
    write: AtomicUsize,
    read: AtomicUsize,
    /// The producer's flushes, and the last the callback took.
    generation: AtomicU32,
    acked: AtomicU32,
    /// The level, an `f32`'s bits: 0 is silent.
    gain: AtomicU32,
    /// The output's rate as the callback last ran, 0 before any.
    rate: AtomicU32,
    /// Frames the callback has played since the last flush, the missing ones counted.
    consumed: AtomicU64,
    /// Frames the callback owes: played as silence while missing, skipped when they come.
    debt: AtomicUsize,
    /// The transport tick the callback's last block ended at, and the one it took the last flush at; [`NO_TICK`]
    /// before any.
    tick: AtomicU64,
    flushed_tick: AtomicU64,
}

/// No tick published yet.
const NO_TICK: u64 = u64::MAX;

/// The one feed: the Displayer writes, the output callback mixes.
pub static FEED: ListenFeed = ListenFeed::new();

impl ListenFeed {
    const fn new() -> Self {
        Self {
            buf: [const { AtomicU32::new(0) }; CAP * 2],
            write: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
            generation: AtomicU32::new(0),
            acked: AtomicU32::new(0),
            gain: AtomicU32::new(0),
            rate: AtomicU32::new(0),
            consumed: AtomicU64::new(0),
            debt: AtomicUsize::new(0),
            tick: AtomicU64::new(NO_TICK),
            flushed_tick: AtomicU64::new(NO_TICK),
        }
    }

    /// The transport tick the callback's last block ended at: the song's clock as the output plays it, which moves on
    /// while the app runs no frame. `None` before a callback ran.
    pub fn tick(&self) -> Option<u64> {
        Some(self.tick.load(Ordering::Acquire)).filter(|&t| t != NO_TICK)
    }

    /// The transport tick the callback took the last flush at: [`Self::consumed`] counts from there.
    pub fn flushed_tick(&self) -> Option<u64> {
        Some(self.flushed_tick.load(Ordering::Acquire)).filter(|&t| t != NO_TICK)
    }

    /// The output's rate as the callback last ran; 0 while no output has run.
    pub fn rate(&self) -> u32 {
        self.rate.load(Ordering::Acquire)
    }

    /// Sets the level (0 silent, 1 as decoded).
    pub fn set_gain(&self, gain: f32) {
        let gain = if gain.is_finite() {
            gain.clamp(0.0, 2.0)
        } else {
            0.0
        };
        self.gain.store(gain.to_bits(), Ordering::Release);
    }

    /// Drops what is queued, once the callback takes it: returns the flush, for [`Self::flushed`].
    pub fn flush(&self) -> u32 {
        self.generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
    }

    /// The callback has taken flush `generation`: what is written from now on plays from where it was then.
    pub fn flushed(&self, generation: u32) -> bool {
        self.acked.load(Ordering::Acquire) == generation
    }

    /// Frames the callback has played since the last flush it took, missing ones counted.
    pub fn consumed(&self) -> u64 {
        self.consumed.load(Ordering::Acquire)
    }

    /// Frames written and not yet played.
    pub fn queued(&self) -> usize {
        let (w, r) = (
            self.write.load(Ordering::Acquire),
            self.read.load(Ordering::Acquire),
        );
        w.wrapping_sub(r).min(CAP)
    }

    /// Writes interleaved stereo `frames`, as many as there is room for; returns how many frames went in.
    pub fn push(&self, frames: &[f32]) -> usize {
        let w = self.write.load(Ordering::Relaxed);
        let r = self.read.load(Ordering::Acquire);
        let room = CAP - w.wrapping_sub(r).min(CAP);
        let n = (frames.len() / 2).min(room);
        for i in 0..n {
            let slot = (w.wrapping_add(i) % CAP) * 2;
            self.buf[slot].store(frames[2 * i].to_bits(), Ordering::Relaxed);
            self.buf[slot + 1].store(frames[2 * i + 1].to_bits(), Ordering::Relaxed);
        }
        self.write.store(w.wrapping_add(n), Ordering::Release);
        n
    }

    /// The callback's part: adds the queued sound into `out` (interleaved, `channels` wide) while the transport
    /// `moving`, at the stream's `rate`; `tick` is the transport tick the block ended at. Allocates nothing and never
    /// blocks.
    pub fn mix(&self, out: &mut [f32], channels: usize, rate: u32, moving: bool, tick: u64) {
        self.rate.store(rate, Ordering::Release);
        self.tick.store(tick, Ordering::Release);
        let generation = self.generation.load(Ordering::Acquire);
        if generation != self.acked.load(Ordering::Relaxed) {
            self.flushed_tick.store(tick, Ordering::Release);
            self.read
                .store(self.write.load(Ordering::Acquire), Ordering::Release);
            self.debt.store(0, Ordering::Relaxed);
            self.consumed.store(0, Ordering::Release);
            self.acked.store(generation, Ordering::Release);
            return;
        }
        if !moving || channels == 0 {
            return;
        }
        let frames = out.len() / channels;
        let w = self.write.load(Ordering::Acquire);
        let mut r = self.read.load(Ordering::Relaxed);
        let mut avail = w.wrapping_sub(r).min(CAP);
        let mut debt = self.debt.load(Ordering::Relaxed);
        let skip = debt.min(avail);
        r = r.wrapping_add(skip);
        avail -= skip;
        debt -= skip;
        let n = frames.min(avail);
        let gain = f32::from_bits(self.gain.load(Ordering::Acquire));
        if gain > 0.0 {
            for i in 0..n {
                let slot = (r.wrapping_add(i) % CAP) * 2;
                let l = f32::from_bits(self.buf[slot].load(Ordering::Relaxed)) * gain;
                let rr = f32::from_bits(self.buf[slot + 1].load(Ordering::Relaxed)) * gain;
                let frame = &mut out[i * channels..(i + 1) * channels];
                if channels == 1 {
                    frame[0] += 0.5 * (l + rr);
                } else {
                    frame[0] += l;
                    frame[1] += rr;
                }
            }
        }
        self.read.store(r.wrapping_add(n), Ordering::Release);
        self.debt.store(debt + (frames - n), Ordering::Relaxed);
        self.consumed.fetch_add(frames as u64, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_sound_is_skipped_so_it_keeps_to_the_clock() {
        let feed = Box::new(ListenFeed::new());
        feed.set_gain(1.0);
        let g = feed.flush();
        let mut out = vec![0.0f32; 8];
        feed.mix(&mut out, 2, 48_000, true, 0);
        assert!(feed.flushed(g));
        // 4 frames played with nothing queued: the first 4 frames written are skipped.
        feed.mix(&mut out, 2, 48_000, true, 0);
        assert_eq!(feed.consumed(), 4);
        let frames: Vec<f32> = (0..16).map(|i| (i / 2) as f32).collect();
        assert_eq!(feed.push(&frames), 8);
        let mut out = vec![0.0f32; 4];
        feed.mix(&mut out, 2, 48_000, true, 0);
        assert_eq!(out, [4.0, 4.0, 5.0, 5.0]);
        // Stopped: nothing taken.
        let mut out = vec![0.0f32; 4];
        feed.mix(&mut out, 2, 48_000, false, 0);
        assert_eq!(out, [0.0; 4]);
        assert_eq!(feed.queued(), 2);
    }
}
