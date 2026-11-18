//! Spectral Ceiling (module type [`SPECTRAL_CEILING`]), Honeydew's own effect: a resonance suppressor. Whatever pokes
//! above a tilted threshold line, as the Analyzer draws the spectrum, is turned down; everything under the line is
//! left exactly as it was.
//!
//! **Params**, normalized, in this order (the saved one): 0 Enable, [`THRESHOLD`], [`TILT`], [`RATIO`], [`CLIP`],
//! [`DEPTH`], [`ATTACK`], [`RELEASE`], [`LOW`], [`HIGH`], [`DELTA`]. Post Gain, Mix and Wet Pan are the rack's
//! (`CommonModule`). The laws from `v`:
//! - Threshold `−60 + 60v` dB, the line.
//! - Tilt `−12 + 24v` dB/octave around 1 kHz, the Analyzer's convention: a bin reads `level + tilt·log2(f/1000)`, so
//!   the Analyzer's `Default` display is +4.5 (white noise reads flat at 0, pink at +3, brown at +6).
//! - Ratio `20^v`:1, how far what is over the line comes down: an excess `e` dB leaves `e/ratio`.
//! - Clip `24v` dB over the line, the most that is left over it whatever the ratio (a hard ceiling); `Off` at 1.
//! - Depth `30v` dB, the most a band is lowered.
//! - Attack `100^v` ms (1–100), Release `10·100^v` ms (10–1000), each band's reduction's ballistics.
//! - Low and High `20·1000^v` Hz, the range it works in; outside it nothing is touched.
//! - Delta: on from 0.5, the output is what is taken away (input − output), to hear what it removes.
//!
//! **Detection.** The mid `(L + R)/2` (the Analyzer's `L + R`) through a periodic-Hann [`FRAME`]-point FFT every
//! [`HOP`] samples, normalised as the Analyzer's (`2/Σw`, a full-scale sine reads 0 dB), so the threshold is the line
//! the user reads off the Analyzer at the same tilt. Each of [`BANDS`] bands, 1/10 octave apart from 20 Hz, takes the
//! loudest tilted bin under it.
//!
//! **No latency.** The audio itself never goes through the FFT: it runs through a cascade of minimum-phase peaking
//! filters, one per band (RBJ bells 1/10 octave wide, so neighbours meet at half their cut and sum to about the cut
//! between them), each at minus its band's reduction. Neighbouring bells add up where they overlap, so each band's cut
//! gives back half of what its neighbours already cut there ([`SPREAD`]): a lone resonance gets its whole cut, a broad
//! region about what was asked, not half again as much. A band at 0 dB is the identity, exactly, and skipped; so the
//! output is not delayed, live or rendered, and frequencies under the line are untouched. Only the detector looks back
//! (its window is centred [`FRAME`]/2 samples ago), which makes the reduction answer a few tens of ms late at most.
//!
//! Nothing allocates while it runs: [`SpectralCeilingState::new`] makes every buffer.

use crate::biquad::flush;
use crate::slicer::fft;

/// The module type: Honeydew's own id, which a song file keeps apart from the song format's modules.
pub const SPECTRAL_CEILING: u16 = 1009;

pub const THRESHOLD: usize = 1;
pub const TILT: usize = 2;
pub const RATIO: usize = 3;
pub const CLIP: usize = 4;
pub const DEPTH: usize = 5;
pub const ATTACK: usize = 6;
pub const RELEASE: usize = 7;
pub const LOW: usize = 8;
pub const HIGH: usize = 9;
pub const DELTA: usize = 10;
/// Enable and the ten above.
pub const PARAMS: usize = 11;

/// A new module's params: Threshold −30 dB, Tilt +4.5 dB/oct (the Analyzer's default), Ratio 4:1 (`ln 4/ln 20`), Clip
/// off, Depth 12 dB, Attack 5 ms (`ln 5/ln 100`), Release 100 ms, Low 20 Hz, High 20 kHz, Delta off.
pub const DEFAULTS: [f32; PARAMS] = [
    1.0, 0.5, 0.6875, 0.462_756, 1.0, 0.4, 0.349_485, 0.5, 0.0, 1.0, 0.0,
];

/// The detector's FFT size and hop, samples (the Analyzer's size).
pub const FRAME: usize = 2048;
pub const HOP: usize = 256;
const BINS: usize = FRAME / 2 + 1;
/// The bands: 1/10 octave apart from [`LOWEST_HZ`], up to 20.48 kHz.
pub const BANDS: usize = 101;
pub const LOWEST_HZ: f32 = 20.0;
pub const BANDS_PER_OCTAVE: f32 = 10.0;
/// The reductions glide toward the detector's every this many samples, and the filters follow.
const TICK: usize = 32;
/// The floor of a level, dB.
pub const FLOOR_DB: f32 = -120.0;
/// A band's filter state under this is silence: the band rests.
const QUIET: f64 = 1e-18;
/// The display's levels fall this fast, dB/s.
const FALL_DB_S: f32 = 40.0;
/// A bell's cut 1, 2 and 3 bands away, as a share of its cut at its centre (dB), and how much of the neighbours' share
/// a band's cut gives back.
const SPREAD: [f64; 3] = [0.2277, 0.0743, 0.0349];
const GIVE_BACK: f64 = 0.5;

/// Band `k`'s centre, Hz.
pub fn band_hz(k: usize) -> f32 {
    LOWEST_HZ * 2f32.powf(k as f32 / BANDS_PER_OCTAVE)
}

/// One band: its bins, its filter and its reduction.
#[derive(Clone, Copy)]
struct Band {
    /// The detector bins it reads, inclusive.
    lo: usize,
    hi: usize,
    /// Below Nyquist's margin and inside Low…High.
    live: bool,
    cos: f64,
    alpha: f64,
    /// The reduction the detector asks for, and the one now (both dB, positive).
    target: f64,
    env: f64,
    /// The bell's normalized coefficients (`b1 = a1`).
    b0: f64,
    b1: f64,
    b2: f64,
    a2: f64,
    /// The transposed direct form II states, per channel.
    s: [[f64; 2]; 2],
}

impl Band {
    const REST: Self = Self {
        lo: 1,
        hi: 1,
        live: false,
        cos: 1.0,
        alpha: 0.0,
        target: 0.0,
        env: 0.0,
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a2: 0.0,
        s: [[0.0; 2]; 2],
    };

    /// The bell at minus the reduction now: identity at 0 dB, exactly.
    fn set_gain(&mut self) {
        let a = 10f64.powf(-self.env / 40.0);
        let a0 = 1.0 + self.alpha / a;
        self.b0 = (1.0 + self.alpha * a) / a0;
        self.b1 = -2.0 * self.cos / a0;
        self.b2 = (1.0 - self.alpha * a) / a0;
        self.a2 = (1.0 - self.alpha / a) / a0;
    }

    fn quiet(&self) -> bool {
        self.s.iter().flatten().all(|s| s.abs() < QUIET)
    }

    #[inline]
    fn run(&mut self, c: usize, x: f64) -> f64 {
        let s = &mut self.s[c];
        let y = self.b0.mul_add(x, s[0]);
        s[0] = self.b1.mul_add(x - y, s[1]);
        s[1] = self.b2 * x - self.a2 * y;
        y
    }
}

/// Spectral Ceiling's state.
pub struct SpectralCeilingState {
    rate: f32,
    /// The last [`FRAME`] mids, a ring whose oldest is at `at`.
    input: Box<[f32]>,
    at: usize,
    /// Samples since the last detector frame, and since the last tick.
    hop_count: usize,
    tick_count: usize,
    window: Box<[f64]>,
    re: Box<[f64]>,
    im: Box<[f64]>,
    /// Each bin's `log2(f/1000)`.
    octaves: Box<[f32]>,
    bands: Box<[Band]>,
    /// Each band's cut as the detector asks for it, before the neighbours' share comes off.
    asked: Box<[f64]>,
    /// The bands that run this tick, in order, and how many.
    active: Box<[usize]>,
    active_len: usize,
    /// Each band's tilted level for the display, dB, falling at [`FALL_DB_S`].
    shown: Box<[f32]>,
    norm: f64,
    threshold: f32,
    tilt: f32,
    ratio: f32,
    clip: Option<f32>,
    depth: f64,
    attack: f64,
    release: f64,
    low: f32,
    high: f32,
    delta: bool,
}

impl SpectralCeilingState {
    pub fn new(params: &[f32], sample_rate: f32) -> Self {
        let n = FRAME as f64;
        let window: Box<[f64]> = (0..FRAME)
            .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n).cos())
            .collect();
        let norm = 2.0 / window.iter().sum::<f64>();
        let mut state = Self {
            rate: 0.0,
            input: vec![0.0; FRAME].into(),
            at: 0,
            hop_count: 0,
            tick_count: 0,
            window,
            re: vec![0.0; FRAME].into(),
            im: vec![0.0; FRAME].into(),
            octaves: vec![0.0; BINS].into(),
            bands: vec![Band::REST; BANDS].into(),
            asked: vec![0.0; BANDS].into(),
            active: vec![0; BANDS].into(),
            active_len: 0,
            shown: vec![FLOOR_DB; BANDS].into(),
            norm,
            threshold: -30.0,
            tilt: 4.5,
            ratio: 4.0,
            clip: None,
            depth: 12.0,
            attack: 1.0,
            release: 1.0,
            low: 20.0,
            high: 20_000.0,
            delta: false,
        };
        state.refresh(params, sample_rate);
        state
    }

    /// Takes the params (and, when it changed, the rate). Running reductions and filters carry on.
    pub fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        let v = |i: usize| {
            let x = params.get(i).copied().unwrap_or(DEFAULTS[i]);
            if x.is_finite() {
                x.clamp(0.0, 1.0)
            } else {
                DEFAULTS[i]
            }
        };
        self.threshold = 60.0f32.mul_add(v(THRESHOLD), -60.0);
        self.tilt = 24.0f32.mul_add(v(TILT), -12.0);
        self.ratio = 20f32.powf(v(RATIO));
        self.clip = (v(CLIP) < 1.0).then(|| 24.0 * v(CLIP));
        self.depth = f64::from(30.0 * v(DEPTH));
        let ms = |ms: f32| f64::from(ms) * 1e-3 * f64::from(sample_rate);
        let coefficient = |samples: f64| 1.0 - (-(TICK as f64) / samples.max(1.0)).exp();
        self.attack = coefficient(ms(100f32.powf(v(ATTACK))));
        self.release = coefficient(ms(10.0 * 100f32.powf(v(RELEASE))));
        self.low = 20.0 * 1000f32.powf(v(LOW));
        self.high = 20.0 * 1000f32.powf(v(HIGH));
        self.delta = v(DELTA) >= 0.5;
        if sample_rate != self.rate {
            self.rate = sample_rate;
            self.place_bands();
        }
        let (low, high) = (self.low.min(self.high), self.low.max(self.high));
        let top = 0.45 * sample_rate;
        let edge = 2f32.powf(0.5 / BANDS_PER_OCTAVE);
        for (k, band) in self.bands.iter_mut().enumerate() {
            let hz = band_hz(k);
            band.live = hz * edge < top && (low * 0.999..=high * 1.001).contains(&hz);
            if !band.live {
                band.target = 0.0;
            }
        }
    }

    /// The bands' bins and filter constants at [`Self::rate`].
    fn place_bands(&mut self) {
        let rate = f64::from(self.rate);
        let bin_hz = rate / FRAME as f64;
        for (k, octave) in self.octaves.iter_mut().enumerate() {
            *octave = if k == 0 {
                0.0
            } else {
                ((k as f64 * bin_hz) / 1000.0).log2() as f32
            };
        }
        let half = 2f64.powf(0.5 / f64::from(BANDS_PER_OCTAVE));
        let width = 1.0 / f64::from(BANDS_PER_OCTAVE);
        for (k, band) in self.bands.iter_mut().enumerate() {
            let hz = f64::from(band_hz(k));
            let nearest = ((hz / bin_hz).round() as usize).clamp(1, BINS - 1);
            let lo = ((hz / half / bin_hz).ceil() as usize).clamp(1, BINS - 1);
            let hi = ((hz * half / bin_hz).floor() as usize).clamp(1, BINS - 1);
            (band.lo, band.hi) = if lo <= hi { (lo, hi) } else { (nearest, nearest) };
            let w0 = (std::f64::consts::TAU * hz / rate).min(0.99 * std::f64::consts::PI);
            let sin = w0.sin();
            band.cos = w0.cos();
            band.alpha = sin * (std::f64::consts::LN_2 / 2.0 * width * w0 / sin).sinh();
            band.set_gain();
        }
    }

    /// Silence: no reduction, no filter memory.
    pub fn reset(&mut self) {
        self.input.fill(0.0);
        (self.at, self.hop_count, self.tick_count) = (0, 0, 0);
        for band in self.bands.iter_mut() {
            (band.target, band.env, band.s) = (0.0, 0.0, [[0.0; 2]; 2]);
            band.set_gain();
        }
        self.active_len = 0;
        self.shown.fill(FLOOR_DB);
    }

    /// The deepest band's reduction now, dB (positive), for the meter.
    pub fn reduction_db(&self) -> f32 {
        self.bands.iter().fold(0.0f64, |a, b| a.max(b.env)) as f32
    }

    /// For the display: each band's tilted level (dB, what the threshold is compared with), then each band's
    /// reduction now (dB, positive). `out` holds `2·BANDS`.
    pub fn view(&self, out: &mut [f32]) {
        let (levels, reductions) = out.split_at_mut(BANDS);
        levels.copy_from_slice(&self.shown);
        for (r, band) in reductions.iter_mut().zip(self.bands.iter()) {
            *r = band.env as f32;
        }
    }

    /// One frame of `l` and `r` in; the same frame out, with what pokes over the line turned down.
    pub fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.input[self.at] = 0.5 * (l + r);
        self.at = (self.at + 1) % FRAME;
        self.hop_count += 1;
        if self.hop_count == HOP {
            self.hop_count = 0;
            self.detect();
        }
        if self.tick_count == 0 {
            self.tick();
        }
        self.tick_count = (self.tick_count + 1) % TICK;
        let (mut y_l, mut y_r) = (f64::from(l), f64::from(r));
        for &k in &self.active[..self.active_len] {
            let band = &mut self.bands[k];
            y_l = band.run(0, y_l);
            y_r = band.run(1, y_r);
        }
        let (y_l, y_r) = (flush(y_l as f32), flush(y_r as f32));
        if self.delta {
            (l - y_l, r - y_r)
        } else {
            (y_l, y_r)
        }
    }

    /// The reductions glide toward the detector's; the bands that cut, or still ring, run.
    fn tick(&mut self) {
        self.active_len = 0;
        for (k, band) in self.bands.iter_mut().enumerate() {
            let before = band.env;
            let speed = if band.target > band.env {
                self.attack
            } else {
                self.release
            };
            band.env += (band.target - band.env) * speed;
            if band.target == 0.0 && band.env < 1e-4 {
                band.env = 0.0;
            }
            if band.env != before {
                band.set_gain();
            }
            if band.env > 0.0 || !band.quiet() {
                self.active[self.active_len] = k;
                self.active_len += 1;
            } else {
                band.s = [[0.0; 2]; 2];
            }
        }
    }

    /// The last [`FRAME`] mids through the FFT: each band's level against the line, and the reduction it asks for.
    fn detect(&mut self) {
        for i in 0..FRAME {
            self.re[i] = f64::from(self.input[(self.at + i) % FRAME]) * self.window[i];
        }
        self.im.fill(0.0);
        fft(&mut self.re, &mut self.im);
        // Each bin's tilted level, dB, kept in `re`.
        let bins = self.re[1..BINS].iter_mut().zip(&self.im[1..BINS]);
        for ((re, im), &octave) in bins.zip(&self.octaves[1..]) {
            let magnitude = (re.hypot(*im) * self.norm).max(1e-6);
            let db = (20.0 * magnitude.log10()) as f32;
            *re = f64::from(db.max(FLOOR_DB) + self.tilt * octave);
        }
        let fall = FALL_DB_S * HOP as f32 / self.rate;
        let bands = self.bands.iter().zip(self.shown.iter_mut());
        for ((band, shown), asked) in bands.zip(self.asked.iter_mut()) {
            let level = self.re[band.lo..=band.hi]
                .iter()
                .fold(f64::NEG_INFINITY, |a, &b| a.max(b)) as f32;
            *shown = level.max(*shown - fall).max(FLOOR_DB);
            let excess = level - self.threshold;
            *asked = if band.live && excess > 0.0 {
                let mut left = excess / self.ratio;
                if let Some(clip) = self.clip {
                    left = left.min(clip);
                }
                f64::from(excess - left).min(self.depth)
            } else {
                0.0
            };
        }
        for (k, band) in self.bands.iter_mut().enumerate() {
            let mut shared = 0.0;
            for (d, &share) in SPREAD.iter().enumerate() {
                let d = d + 1;
                let below = k.checked_sub(d).map_or(0.0, |j| self.asked[j]);
                let above = self.asked.get(k + d).copied().unwrap_or(0.0);
                shared += share * (below + above);
            }
            band.target = GIVE_BACK.mul_add(-shared, self.asked[k]).max(0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    fn sine(hz: f32, amplitude: f32, i: usize) -> f32 {
        amplitude * (std::f32::consts::TAU * hz * i as f32 / RATE).sin()
    }

    /// Runs `input` through a module at `params`; the output.
    fn run(params: &[f32], input: &[f32]) -> Vec<f32> {
        let mut state = SpectralCeilingState::new(params, RATE);
        input.iter().map(|&x| state.process_frame(x, x).0).collect()
    }

    /// `x`'s RMS over its last half second.
    fn rms(x: &[f32]) -> f32 {
        let tail = &x[x.len() - RATE as usize / 2..];
        (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt()
    }

    #[test]
    fn under_the_line_nothing_changes_at_all() {
        // −40 dB at 1 kHz under a −30 dB line (tilt 0 reads it as is): bit for bit the input, no delay.
        let mut params = DEFAULTS;
        params[TILT] = 0.5;
        let input: Vec<f32> = (0..RATE as usize).map(|i| sine(1000.0, 0.01, i)).collect();
        assert_eq!(run(&params, &input), input);
    }

    #[test]
    fn a_tone_over_the_line_comes_down_to_it_with_the_clip() {
        // −10 dB at 1420 Hz, the line at −30 dB, Clip 0 dB, Depth 30 dB: about 20 dB over it. 1420 Hz falls halfway
        // between two bands, where their bells meet, so it comes down a little less (about 16.5 dB).
        let mut params = DEFAULTS;
        params[TILT] = 0.5;
        params[CLIP] = 0.0;
        params[DEPTH] = 1.0;
        let amplitude = 10f32.powf(-10.0 / 20.0);
        let input: Vec<f32> = (0..2 * RATE as usize)
            .map(|i| sine(1420.0, amplitude, i))
            .collect();
        let output = run(&params, &input);
        let change = 20.0 * (rms(&output) / rms(&input)).log10();
        assert!((-21.0..-14.0).contains(&change), "{change} dB");
    }

    #[test]
    fn only_the_resonance_comes_down() {
        // A loud 4630 Hz resonance over the line and a quiet 300 Hz tone under it: the quiet tone keeps its level.
        let mut params = DEFAULTS;
        params[TILT] = 0.5;
        params[CLIP] = 0.0;
        let low: Vec<f32> = (0..2 * RATE as usize).map(|i| sine(300.0, 0.01, i)).collect();
        let loud: Vec<f32> = (0..2 * RATE as usize).map(|i| sine(4630.0, 0.3, i)).collect();
        let input: Vec<f32> = low.iter().zip(&loud).map(|(a, b)| a + b).collect();
        let output = run(&params, &input);
        // Take the low tone back out with the same filters' view: a 300 Hz correlation.
        let at = output.len() - RATE as usize / 2;
        let corr = |x: &[f32]| {
            let (mut c, mut s) = (0.0f64, 0.0f64);
            for (i, &v) in x[at..].iter().enumerate() {
                let t = std::f64::consts::TAU * 300.0 * (at + i) as f64 / f64::from(RATE);
                c += f64::from(v) * t.cos();
                s += f64::from(v) * t.sin();
            }
            c.hypot(s)
        };
        let kept = 20.0 * (corr(&output) / corr(&input)).log10();
        assert!(kept.abs() < 0.2, "300 Hz changed by {kept} dB");
        let resonance = 20.0 * (rms(&output) / rms(&input)).log10();
        assert!(resonance < -6.0, "the resonance only came down {resonance} dB");
    }

    #[test]
    fn the_module_has_no_latency_and_cuts_in_place() {
        use crate::{AudioEffect, ProcessContext};
        let mut effect = AudioEffect::new(SPECTRAL_CEILING, RATE).unwrap();
        assert_eq!(effect.latency_samples(), 0);
        effect.set_parameter(CLIP, 0.0).unwrap();
        let context = ProcessContext {
            sample_rate: RATE,
            tempo_bpm: 120.0,
            start_beat: 0.0,
            playing: true,
        };
        let input: Vec<f32> = (0..RATE as usize).map(|i| sine(4630.0, 0.5, i)).collect();
        let (mut left, mut right) = (input.clone(), input.clone());
        for (l, r) in left.chunks_mut(480).zip(right.chunks_mut(480)) {
            effect.process(l, r, context).unwrap();
        }
        assert!(rms(&left) < 0.5 * rms(&input));
        assert!(effect.meter().gain_reduction_db > 6.0);
    }

    #[test]
    fn delta_is_what_was_taken_away() {
        let mut params = DEFAULTS;
        params[CLIP] = 0.0;
        let input: Vec<f32> = (0..RATE as usize).map(|i| sine(2000.0, 0.5, i)).collect();
        let wet = run(&params, &input);
        params[DELTA] = 1.0;
        let delta = run(&params, &input);
        for ((x, w), d) in input.iter().zip(&wet).zip(&delta) {
            assert!((x - w - d).abs() < 1e-6);
        }
    }
}
