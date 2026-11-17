//! Citrus's factory presets: our own sound design, made in code from Citrus's own state type
//! ([`citrus::state::State`]), so every preset is a state Citrus wrote itself. They list as
//! `Presets/Citrus/[<category>/]<name>.ospreset` beside the other built-in plugins' presets ([`super::presets`]), each a
//! state preset (Citrus's plugin state: the Sytrus state in nih-plug's JSON, [`citrus::plugin_state_from_fl`]); a file
//! is made when it is first read.
//!
//! **What a patch sets.** A preset starts from [`base`]: a fresh state with the main page's master envelope knobs,
//! unison knobs and mod X / Y at their reset values ([`citrus::ctl::default`]). A fresh state leaves those at 0, which
//! squashes every envelope to no time at all. Then:
//! - an operator ([`op`]) has a volume (of 256), a frequency ratio and an octave of pitch range, its pre-filter open
//!   (shape int 4 at 128: at 0 the wave keeps its first four harmonics only) and band limiting on;
//! - the matrix's rows are destinations and its columns sources: [`fm`] puts operator `from` into `to`'s phase,
//!   [`out`] a row into the output, [`send`] into the effects (the delays, the chorus and the reverb take only the send);
//! - an envelope's distances are half seconds (the curves' free unit); the helpers take seconds;
//! - a filter ([`filter`]) is the state variable one (type 0, one stage, its low output unless [`filter_mix`] says
//!   otherwise); its cutoff `c` (of 256) runs from about 70 Hz to 7 kHz, and its cutoff envelope adds up to
//!   `amount / 256` of the range;
//! - levels: a matrix cell's gain is `(11^(v/256) - 1) / 10` (128 is about -13 dB), the master level's the same of
//!   `v/128`. One note peaks around -12 dBFS, so a chord stays clear of clipping.

use std::sync::OnceLock;

use citrus::curve::{Curve, Point, ENVELOPE};
use citrus::state::{
    Delay, State, BAND_LIMIT, COL_OUT, COL_PAN, COL_SEND, F_CUTOFF, OPERATORS, OP_PITCH, OP_VOLUME,
};

/// Citrus's CLAP id, without the format.
pub const ID: &str = "org.openstudio.citrus";
/// Its presets' folder: its name ([`super::presets::folder`]).
pub const FOLDER: &str = "Citrus";
/// The preset a fresh instance plays, the picker's `Default` row.
const DEFAULT: &str = "Default";

/// One factory preset.
pub struct Patch {
    /// Its folder below Citrus's (`""` for none).
    pub category: &'static str,
    pub name: &'static str,
    make: fn(&mut State),
}

impl Patch {
    /// Its path below the presets folder.
    pub fn rel(&self) -> String {
        match self.category {
            "" => format!("{FOLDER}/{}.{}", self.name, super::presets::EXT),
            category => format!("{FOLDER}/{category}/{}.{}", self.name, super::presets::EXT),
        }
    }

    /// Whether it is the `Default` preset (the picker's own row, not listed in the factory).
    pub fn is_default(&self) -> bool {
        self.category.is_empty() && self.name == DEFAULT
    }

    /// Its Sytrus state.
    pub fn state(&self) -> State {
        let mut st = base();
        (self.make)(&mut st);
        st.name = self.name.to_owned();
        st
    }

    /// Its preset file: the header, then Citrus's plugin state.
    pub fn file(&self) -> Vec<u8> {
        let fl = self.state().to_bytes(None);
        let mut bytes = format!("OSPRESET 1\nplugin={FOLDER}\tid=clap:{ID}\tformat=clap\n").into_bytes();
        bytes.extend_from_slice(&citrus::plugin_state_from_fl(&fl));
        bytes
    }
}

/// Every preset's path and file, made at the first call.
pub fn files() -> &'static [(String, Vec<u8>)] {
    static FILES: OnceLock<Vec<(String, Vec<u8>)>> = OnceLock::new();
    FILES.get_or_init(|| PATCHES.iter().map(|p| (p.rel(), p.file())).collect())
}

const fn p(category: &'static str, name: &'static str, make: fn(&mut State)) -> Patch {
    Patch {
        category,
        name,
        make,
    }
}

const BASS: &str = "Bass";
const LEADS: &str = "Leads";
const PADS: &str = "Strings and pads";
const KEYS: &str = "Keys and bells";
const PLUCKS: &str = "Plucks";
const BRASS: &str = "Brass and winds";
const DRUMS: &str = "Drums";
const FX: &str = "Effects";

/// The presets, in no particular order (the picker sorts them).
pub static PATCHES: &[Patch] = &[
    p("", DEFAULT, default),
    // Bass
    p(BASS, "Sub Bass", sub_bass),
    p(BASS, "FM Slap", fm_slap),
    p(BASS, "Reese", reese),
    p(BASS, "Acid Squelch", acid),
    p(BASS, "Rubber Bass", rubber_bass),
    p(BASS, "Deep FM", deep_fm),
    p(BASS, "Pluck Bass", pluck_bass),
    p(BASS, "808 Drop", drop_808),
    // Leads
    p(LEADS, "Supersaw", supersaw),
    p(LEADS, "Square Lead", square_lead),
    p(LEADS, "FM Lead", fm_lead),
    p(LEADS, "Scream", scream),
    p(LEADS, "Chip Lead", chip_lead),
    p(LEADS, "Whistle", whistle),
    p(LEADS, "Soft Saw", soft_saw),
    // Strings and pads
    p(PADS, "Warm Pad", warm_pad),
    p(PADS, "Glass Pad", glass_pad),
    p(PADS, "Choir", choir),
    p(PADS, "Dark Drone", dark_drone),
    p(PADS, "Shimmer", shimmer),
    p(PADS, "Evolving Pad", evolving_pad),
    p(PADS, "String Ensemble", strings),
    // Keys and bells
    p(KEYS, "Electric Piano", electric_piano),
    p(KEYS, "Bright EP", bright_ep),
    p(KEYS, "FM Bell", fm_bell),
    p(KEYS, "Tubular Bell", tubular_bell),
    p(KEYS, "Drawbar Organ", organ),
    p(KEYS, "Music Box", music_box),
    p(KEYS, "Marimba", marimba),
    p(KEYS, "Clav", clav),
    p(KEYS, "Vibraphone", vibraphone),
    // Plucks
    p(PLUCKS, "Harp", harp),
    p(PLUCKS, "Synth Pluck", synth_pluck),
    p(PLUCKS, "Kalimba", kalimba),
    p(PLUCKS, "Nylon", nylon),
    p(PLUCKS, "Pizzicato", pizzicato),
    // Brass and winds
    p(BRASS, "Brass Section", brass),
    p(BRASS, "FM Brass", fm_brass),
    p(BRASS, "Flute", flute),
    // Drums
    p(DRUMS, "Kick", kick),
    p(DRUMS, "Snare", snare),
    p(DRUMS, "Clap", clap),
    p(DRUMS, "Closed Hat", closed_hat),
    p(DRUMS, "Open Hat", open_hat),
    p(DRUMS, "Tom", tom),
    // Effects
    p(FX, "Laser", laser),
    p(FX, "Noise Riser", riser),
    p(FX, "Metal Hit", metal_hit),
    p(FX, "Blip", blip),
    p(FX, "Wind", wind),
];

// ---- Building blocks ----

/// A fresh state with the main page's knobs at their reset values and nothing playing.
fn base() -> State {
    let mut st = State::default();
    for k in [0, 1, 3, 4, 5, 7] {
        st.s[k] = 128;
    }
    st.s[8] = 1;
    st.s[10] = 128;
    st.s[11] = 128;
    st.s[25] = 256;
    st.s[26] = 256;
    st
}

/// The master level (of 128).
fn volume(st: &mut State, v: i32) {
    st.main_volume = v;
}

/// Operator `o` at volume `level` (of 256), `ratio` times the note's frequency, an octave of pitch range, a sine.
fn op(st: &mut State, o: usize, level: i32, ratio: f32) {
    let m = &mut st.modules[o];
    m.header[1] = level;
    m.header[2] = 1200;
    m.op.ratio = (ratio * 10_000.0).round() as i32;
    m.op.shape[4] = 128;
    m.op.flags[BAND_LIMIT] = 1;
    m.op.levels.iter_mut().for_each(|l| *l = 0.0);
    m.op.levels[0] = 1.0;
}

/// Operator `o`'s harmonics: harmonic `n` (1 the fundamental) at `level(n)`, the first `count`.
fn harmonics(st: &mut State, o: usize, count: usize, level: impl Fn(usize) -> f32) {
    let levels = &mut st.modules[o].op.levels;
    for (h, l) in levels.iter_mut().enumerate() {
        *l = if h < count { level(h + 1) } else { 0.0 };
    }
}

fn saw(st: &mut State, o: usize) {
    harmonics(st, o, 128, |n| 1.0 / n as f32);
}

fn square(st: &mut State, o: usize) {
    harmonics(st, o, 128, |n| if n % 2 == 1 { 1.0 / n as f32 } else { 0.0 });
}

/// Operator `o`'s harmonics as listed, from the fundamental.
fn levels(st: &mut State, o: usize, list: &[f32]) {
    harmonics(st, o, list.len(), |n| list[n - 1]);
}

/// Operator `o`'s noise (of 128; 128 is noise only).
fn noise(st: &mut State, o: usize, v: i32) {
    st.modules[o].op.noise = v;
}

/// Operator `o`'s frequency offset, in Hz.
fn detune(st: &mut State, o: usize, hz: f32) {
    st.modules[o].op.offset = (hz * 100.0).round() as i32;
}

fn cell(st: &mut State, row: usize, col: usize, v: i32) {
    st.matrix[row * 9 + col] = v;
}

/// Row `row` (an operator, or filter `f` at `OPERATORS + f`) into the output.
fn out(st: &mut State, row: usize, v: i32) {
    cell(st, row, COL_OUT, v);
}

/// Row `row` into the effects.
fn send(st: &mut State, row: usize, v: i32) {
    cell(st, row, COL_SEND, v);
}

/// Row `row`'s pan (-256 left .. 256 right).
fn pan(st: &mut State, row: usize, v: i32) {
    cell(st, row, COL_PAN, v);
}

/// Operator `from` into operator `to`'s phase.
fn fm(st: &mut State, from: usize, to: usize, v: i32) {
    cell(st, to, from, v);
}

/// Operator `o` into its own phase.
fn feedback(st: &mut State, o: usize, v: i32) {
    cell(st, o, o, v);
}

/// An envelope through `points` (seconds to the point, level, tension: above 0 a fast start), its decay point and
/// sustain point (`-1` for none).
fn curve(points: &[(f32, f32, f32)], decay: i32, sustain: i32) -> Curve {
    let mut c = Curve::default_of(ENVELOPE);
    c.points = (points.iter())
        .map(|&(secs, level, tension)| Point {
            dist: secs * 2.0,
            level,
            tension,
            ..Point::default()
        })
        .collect();
    c.enabled = true;
    c.decay = decay;
    c.loop_start = -1;
    c.sustain = sustain;
    c.prepare();
    c
}

/// Attack, decay to the sustain level, release (seconds; the sustain 0..1).
fn adsr(a: f32, d: f32, s: f32, r: f32) -> Curve {
    curve(&[(0.0, 0.0, 0.0), (a, 1.0, 0.0), (d, s, 0.3), (r, 0.0, 0.3)], 1, 2)
}

/// Operator `o`'s volume envelope.
fn amp(st: &mut State, o: usize, a: f32, d: f32, s: f32, r: f32) {
    let art = &mut st.modules[o].arts[OP_VOLUME];
    art.env = adsr(a, d, s, r);
    art.base = 0.0;
    art.env_amount = 1.0;
}

/// Operator `o`'s pitch envelope: from `from` (1 the range up, 0.5 the note, 0 the range down) to the note in `secs`,
/// a range of `cents`.
fn pitch(st: &mut State, o: usize, cents: i32, secs: f32, from: f32) {
    let m = &mut st.modules[o];
    m.header[2] = cents;
    let art = &mut m.arts[OP_PITCH];
    art.env = curve(&[(0.0, from, 0.0), (secs, 0.5, 0.3)], -1, -1);
    art.base = 0.0;
    art.env_amount = 1.0;
}

/// Filter `f` on, at cutoff `cutoff` and resonance `res` (of 256), its low output, into the output.
fn filter(st: &mut State, f: usize, cutoff: i32, res: i32) {
    let row = OPERATORS + f;
    let m = &mut st.modules[row];
    m.filter.kind = 0;
    m.header[0] = 1;
    m.header[2] = cutoff;
    m.header[3] = res;
    m.header[4] = 128;
    out(st, row, 256);
}

/// Filter `f`'s low, band and high outputs (of 128).
fn filter_mix(st: &mut State, f: usize, low: i32, band: i32, high: i32) {
    let h = &mut st.modules[OPERATORS + f].header;
    (h[4], h[5], h[6]) = (low, band, high);
}

/// Operator `o` into filter `f`.
fn to_filter(st: &mut State, o: usize, f: usize, v: i32) {
    cell(st, OPERATORS + f, o, v);
}

/// Filter `f`'s cutoff envelope: up by `amount` (of 256) at its peak, `s` of it held.
fn filter_env(st: &mut State, f: usize, amount: i32, a: f32, d: f32, s: f32, r: f32) {
    let m = &mut st.modules[OPERATORS + f];
    m.header[1] = amount;
    let top = 0.5 + 0.5 * s;
    m.arts[F_CUTOFF].env = curve(&[(0.0, 0.5, 0.0), (a, 1.0, 0.0), (d, top, 0.3), (r, 0.5, 0.3)], 1, 2);
}

/// `order` unison voices, detuned by `detune` (of 256) and spread by `spread` (of 128), their phases apart.
fn unison(st: &mut State, order: i32, detune: i32, spread: i32) {
    st.s[8] = order;
    st.s[9] = spread;
    st.s[11] = detune;
    st.s[13] = 128;
}

/// The reverb on: `decay` in tenths of a second, wet level `wet` (of 128).
fn reverb(st: &mut State, decay: i32, wet: i32) {
    st.reverb.on = true;
    st.reverb.knobs[3] = 60;
    st.reverb.knobs[5] = decay;
    st.reverb.knobs[8] = wet;
}

/// The first delay on, tempo-synced: `time` in 1/192 beats, feedback (of 127), level (of 128), the sides apart.
fn delay(st: &mut State, time: i32, feedback: i32, level: i32) {
    st.delays[0] = Delay {
        feedback,
        time,
        stereo: 300,
        level,
        mode: 2,
        synced: true,
        serial: false,
        on: true,
    };
}

/// The chorus on (of the send), at `volume` (of 128).
fn chorus(st: &mut State, volume: i32) {
    st.s[16] = 3;
    st.s[17] = 380;
    st.s[18] = 320;
    st.s[19] = 40;
    st.s[20] = 90;
    st.s[21] = 0;
    st.s[22] = volume;
}

// ---- The presets ----

/// What a fresh instance plays.
fn default(st: &mut State) {
    *st = citrus::preset::default_patch();
}

fn sub_bass(st: &mut State) {
    op(st, 0, 256, 1.0);
    amp(st, 0, 0.003, 0.3, 0.85, 0.08);
    out(st, 0, 256);
    op(st, 1, 200, 1.0);
    amp(st, 1, 0.002, 0.25, 0.3, 0.08);
    fm(st, 1, 0, 40);
    volume(st, 100);
}

fn fm_slap(st: &mut State) {
    op(st, 0, 256, 1.0);
    amp(st, 0, 0.002, 0.6, 0.7, 0.08);
    out(st, 0, 256);
    op(st, 1, 256, 1.0);
    amp(st, 1, 0.001, 0.12, 0.1, 0.1);
    fm(st, 1, 0, 110);
    volume(st, 95);
}

fn reese(st: &mut State) {
    for (o, hz) in [(0, 0.5), (1, -0.5)] {
        op(st, o, 256, 1.0);
        saw(st, o);
        detune(st, o, hz);
        amp(st, o, 0.005, 0.5, 1.0, 0.1);
        to_filter(st, o, 0, 200);
    }
    filter(st, 0, 120, 60);
    op(st, 2, 256, 0.5);
    amp(st, 2, 0.005, 0.5, 1.0, 0.1);
    out(st, 2, 160);
    volume(st, 95);
}

fn acid(st: &mut State) {
    op(st, 0, 256, 1.0);
    saw(st, 0);
    amp(st, 0, 0.002, 0.3, 0.75, 0.05);
    to_filter(st, 0, 0, 256);
    filter(st, 0, 50, 220);
    filter_env(st, 0, 200, 0.002, 0.22, 0.15, 0.1);
    volume(st, 95);
}

fn rubber_bass(st: &mut State) {
    op(st, 0, 256, 1.0);
    square(st, 0);
    amp(st, 0, 0.002, 0.4, 0.8, 0.06);
    to_filter(st, 0, 0, 256);
    filter(st, 0, 70, 110);
    filter_env(st, 0, 150, 0.002, 0.3, 0.2, 0.1);
    op(st, 1, 256, 0.5);
    amp(st, 1, 0.002, 0.4, 0.8, 0.06);
    out(st, 1, 140);
    volume(st, 100);
}

fn deep_fm(st: &mut State) {
    op(st, 0, 256, 1.0);
    amp(st, 0, 0.002, 0.8, 0.9, 0.08);
    out(st, 0, 256);
    op(st, 1, 256, 2.0);
    amp(st, 1, 0.002, 0.5, 0.35, 0.1);
    fm(st, 1, 0, 80);
    volume(st, 100);
}

fn pluck_bass(st: &mut State) {
    op(st, 0, 256, 1.0);
    saw(st, 0);
    amp(st, 0, 0.001, 0.5, 0.3, 0.08);
    to_filter(st, 0, 0, 256);
    filter(st, 0, 40, 90);
    filter_env(st, 0, 210, 0.001, 0.16, 0.0, 0.08);
    op(st, 1, 256, 0.5);
    amp(st, 1, 0.001, 0.4, 0.3, 0.08);
    out(st, 1, 120);
    volume(st, 100);
}

fn drop_808(st: &mut State) {
    op(st, 0, 256, 1.0);
    pitch(st, 0, 1200, 0.06, 1.0);
    amp(st, 0, 0.001, 1.8, 0.0, 0.15);
    feedback(st, 0, 140);
    out(st, 0, 256);
    volume(st, 110);
}

fn supersaw(st: &mut State) {
    op(st, 0, 256, 1.0);
    saw(st, 0);
    amp(st, 0, 0.004, 0.4, 0.85, 0.25);
    out(st, 0, 256);
    send(st, 0, 170);
    unison(st, 7, 180, 110);
    reverb(st, 25, 50);
    delay(st, 144, 50, 60);
    volume(st, 85);
}

fn square_lead(st: &mut State) {
    op(st, 0, 256, 1.0);
    square(st, 0);
    amp(st, 0, 0.003, 0.2, 0.9, 0.12);
    out(st, 0, 256);
    send(st, 0, 140);
    op(st, 1, 90, 2.0);
    square(st, 1);
    amp(st, 1, 0.003, 0.2, 0.9, 0.12);
    out(st, 1, 256);
    unison(st, 2, 60, 60);
    delay(st, 144, 45, 55);
    volume(st, 80);
}

fn fm_lead(st: &mut State) {
    op(st, 0, 256, 1.0);
    amp(st, 0, 0.003, 0.3, 0.9, 0.15);
    out(st, 0, 256);
    send(st, 0, 120);
    op(st, 1, 256, 1.0);
    amp(st, 1, 0.003, 0.4, 0.6, 0.15);
    feedback(st, 1, 160);
    fm(st, 1, 0, 120);
    reverb(st, 20, 45);
    volume(st, 85);
}

fn scream(st: &mut State) {
    op(st, 0, 256, 1.0);
    amp(st, 0, 0.004, 0.3, 0.9, 0.15);
    to_filter(st, 0, 0, 256);
    filter(st, 0, 200, 120);
    op(st, 1, 256, 3.0);
    amp(st, 1, 0.01, 0.6, 0.7, 0.15);
    fm(st, 1, 0, 150);
    unison(st, 3, 90, 80);
    volume(st, 80);
}

fn chip_lead(st: &mut State) {
    op(st, 0, 256, 1.0);
    square(st, 0);
    amp(st, 0, 0.001, 0.05, 0.8, 0.04);
    out(st, 0, 256);
    send(st, 0, 140);
    delay(st, 96, 40, 50);
    volume(st, 70);
}

fn whistle(st: &mut State) {
    op(st, 0, 256, 1.0);
    noise(st, 0, 12);
    amp(st, 0, 0.08, 0.3, 0.9, 0.25);
    out(st, 0, 256);
    send(st, 0, 160);
    reverb(st, 30, 55);
    volume(st, 85);
}

fn soft_saw(st: &mut State) {
    for (o, hz) in [(0, 0.0), (1, 1.2)] {
        op(st, o, 256, 1.0);
        saw(st, o);
        detune(st, o, hz);
        amp(st, o, 0.01, 0.5, 0.85, 0.2);
        to_filter(st, o, 0, 220);
    }
    filter(st, 0, 170, 60);
    send(st, OPERATORS, 120);
    reverb(st, 20, 45);
    volume(st, 85);
}

fn warm_pad(st: &mut State) {
    op(st, 0, 256, 1.0);
    saw(st, 0);
    amp(st, 0, 0.7, 1.0, 0.9, 1.6);
    to_filter(st, 0, 0, 256);
    filter(st, 0, 110, 50);
    filter_env(st, 0, 60, 1.0, 1.5, 0.5, 2.0);
    send(st, OPERATORS, 200);
    unison(st, 5, 120, 100);
    reverb(st, 40, 60);
    chorus(st, 90);
    volume(st, 75);
}

fn glass_pad(st: &mut State) {
    op(st, 0, 256, 1.0);
    amp(st, 0, 0.5, 1.0, 0.9, 2.0);
    out(st, 0, 256);
    send(st, 0, 200);
    op(st, 1, 256, 3.0);
    amp(st, 1, 1.2, 2.0, 0.5, 2.0);
    fm(st, 1, 0, 70);
    op(st, 2, 90, 2.0);
    detune(st, 2, 0.3);
    amp(st, 2, 0.5, 1.0, 0.9, 2.0);
    out(st, 2, 256);
    send(st, 2, 200);
    reverb(st, 45, 60);
    volume(st, 80);
}

fn choir(st: &mut State) {
    op(st, 0, 256, 1.0);
    levels(st, 0, &[1.0, 0.6, 0.9, 0.7, 0.35, 0.25, 0.4, 0.2, 0.1, 0.08, 0.05, 0.04]);
    amp(st, 0, 0.4, 1.0, 0.9, 1.2);
    out(st, 0, 256);
    send(st, 0, 200);
    unison(st, 4, 70, 128);
    reverb(st, 50, 60);
    chorus(st, 80);
    volume(st, 80);
}

fn dark_drone(st: &mut State) {
    for (o, hz) in [(0, 0.0), (1, 0.4)] {
        op(st, o, 256, 0.5);
        saw(st, o);
        detune(st, o, hz);
        amp(st, o, 1.5, 2.0, 1.0, 3.0);
        to_filter(st, o, 0, 220);
    }
    filter(st, 0, 70, 140);
    filter_env(st, 0, 50, 3.0, 3.0, 0.6, 3.0);
    send(st, OPERATORS, 180);
    reverb(st, 60, 60);
    volume(st, 85);
}

fn shimmer(st: &mut State) {
    for (o, level, ratio, side) in [(0, 256, 1.0, 0), (1, 120, 2.0, -128), (2, 70, 4.0, 128), (3, 40, 8.0, 0)] {
        op(st, o, level, ratio);
        amp(st, o, 1.0, 2.0, 0.8, 2.5);
        out(st, o, 256);
        send(st, o, 230);
        pan(st, o, side);
    }
    reverb(st, 70, 70);
    delay(st, 192, 70, 70);
    volume(st, 70);
}

fn evolving_pad(st: &mut State) {
    op(st, 0, 256, 1.0);
    saw(st, 0);
    amp(st, 0, 1.2, 2.0, 1.0, 2.5);
    to_filter(st, 0, 0, 230);
    op(st, 1, 90, 2.0);
    square(st, 1);
    amp(st, 1, 1.2, 2.0, 1.0, 2.5);
    to_filter(st, 1, 0, 230);
    filter(st, 0, 60, 120);
    filter_env(st, 0, 140, 4.0, 4.0, 0.7, 3.0);
    send(st, OPERATORS, 190);
    unison(st, 3, 100, 90);
    reverb(st, 50, 60);
    chorus(st, 80);
    volume(st, 75);
}

fn strings(st: &mut State) {
    op(st, 0, 256, 1.0);
    saw(st, 0);
    amp(st, 0, 0.35, 0.8, 0.9, 0.8);
    to_filter(st, 0, 0, 256);
    filter(st, 0, 150, 30);
    send(st, OPERATORS, 180);
    unison(st, 5, 100, 110);
    reverb(st, 40, 55);
    chorus(st, 80);
    volume(st, 75);
}

fn electric_piano(st: &mut State) {
    op(st, 0, 256, 1.0);
    amp(st, 0, 0.002, 3.0, 0.0, 0.4);
    out(st, 0, 256);
    send(st, 0, 100);
    op(st, 1, 256, 1.0);
    amp(st, 1, 0.001, 1.0, 0.05, 0.3);
    fm(st, 1, 0, 80);
    op(st, 2, 120, 1.0);
    amp(st, 2, 0.001, 0.8, 0.0, 0.3);
    out(st, 2, 160);
    op(st, 3, 256, 14.0);
    amp(st, 3, 0.001, 0.12, 0.0, 0.1);
    fm(st, 3, 2, 70);
    chorus(st, 70);
    volume(st, 80);
}

fn bright_ep(st: &mut State) {
    electric_piano(st);
    fm(st, 1, 0, 120);
    feedback(st, 1, 80);
    reverb(st, 15, 40);
}

fn fm_bell(st: &mut State) {
    op(st, 0, 256, 1.0);
    amp(st, 0, 0.001, 4.0, 0.0, 2.0);
    out(st, 0, 256);
    send(st, 0, 180);
    op(st, 1, 256, 3.5);
    amp(st, 1, 0.001, 2.5, 0.0, 1.5);
    fm(st, 1, 0, 120);
    op(st, 2, 100, 2.0);
    amp(st, 2, 0.001, 2.0, 0.0, 1.5);
    out(st, 2, 256);
    send(st, 2, 180);
    op(st, 3, 256, 7.07);
    amp(st, 3, 0.001, 1.0, 0.0, 1.0);
    fm(st, 3, 2, 70);
    reverb(st, 40, 55);
    volume(st, 80);
}

fn tubular_bell(st: &mut State) {
    op(st, 0, 256, 1.0);
    amp(st, 0, 0.001, 5.0, 0.0, 2.5);
    out(st, 0, 256);
    send(st, 0, 170);
    op(st, 1, 256, 1.4);
    amp(st, 1, 0.001, 3.0, 0.0, 2.0);
    fm(st, 1, 0, 140);
    reverb(st, 45, 55);
    volume(st, 80);
}

fn organ(st: &mut State) {
    // Drawbars 16', 8', 5 1/3', 4', 2 2/3', 2', 1 3/5', 1 1/3', 1' as harmonics of the 16' (an octave down).
    op(st, 0, 256, 0.5);
    let bars = [(1, 0.8), (2, 1.0), (3, 0.8), (4, 0.5), (6, 0.3), (8, 0.4), (12, 0.2), (16, 0.2)];
    harmonics(st, 0, 16, |n| bars.iter().find(|&&(h, _)| h == n).map_or(0.0, |&(_, l)| l));
    amp(st, 0, 0.005, 0.1, 1.0, 0.06);
    out(st, 0, 256);
    send(st, 0, 200);
    chorus(st, 100);
    st.s[18] = 600;
    volume(st, 70);
}

fn music_box(st: &mut State) {
    op(st, 0, 256, 2.0);
    amp(st, 0, 0.001, 1.5, 0.0, 1.2);
    out(st, 0, 256);
    send(st, 0, 160);
    op(st, 1, 256, 5.0);
    amp(st, 1, 0.001, 0.2, 0.0, 0.1);
    fm(st, 1, 0, 60);
    reverb(st, 35, 55);
    delay(st, 192, 35, 40);
    volume(st, 80);
}

fn marimba(st: &mut State) {
    op(st, 0, 256, 1.0);
    amp(st, 0, 0.001, 0.5, 0.0, 0.3);
    out(st, 0, 256);
    op(st, 1, 256, 4.0);
    amp(st, 1, 0.001, 0.06, 0.0, 0.05);
    fm(st, 1, 0, 90);
    volume(st, 90);
}

fn clav(st: &mut State) {
    op(st, 0, 256, 1.0);
    amp(st, 0, 0.001, 0.6, 0.4, 0.06);
    out(st, 0, 256);
    op(st, 1, 256, 3.0);
    amp(st, 1, 0.001, 0.4, 0.25, 0.05);
    feedback(st, 1, 170);
    fm(st, 1, 0, 120);
    volume(st, 85);
}

fn vibraphone(st: &mut State) {
    op(st, 0, 256, 1.0);
    amp(st, 0, 0.001, 2.5, 0.0, 1.0);
    out(st, 0, 256);
    send(st, 0, 150);
    op(st, 1, 256, 4.0);
    amp(st, 1, 0.001, 0.3, 0.0, 0.2);
    fm(st, 1, 0, 50);
    chorus(st, 80);
    reverb(st, 30, 45);
    volume(st, 85);
}

fn harp(st: &mut State) {
    op(st, 0, 256, 1.0);
    amp(st, 0, 0.001, 2.0, 0.0, 0.8);
    out(st, 0, 256);
    send(st, 0, 160);
    op(st, 1, 256, 2.0);
    amp(st, 1, 0.001, 0.4, 0.0, 0.3);
    fm(st, 1, 0, 70);
    reverb(st, 35, 55);
    volume(st, 85);
}

fn synth_pluck(st: &mut State) {
    op(st, 0, 256, 1.0);
    saw(st, 0);
    amp(st, 0, 0.001, 0.45, 0.0, 0.25);
    to_filter(st, 0, 0, 256);
    filter(st, 0, 60, 80);
    filter_env(st, 0, 220, 0.001, 0.2, 0.0, 0.15);
    send(st, OPERATORS, 180);
    unison(st, 3, 100, 90);
    delay(st, 144, 55, 70);
    reverb(st, 30, 50);
    volume(st, 80);
}

fn kalimba(st: &mut State) {
    op(st, 0, 256, 1.0);
    amp(st, 0, 0.001, 1.2, 0.0, 0.6);
    out(st, 0, 256);
    send(st, 0, 150);
    op(st, 1, 256, 3.0);
    amp(st, 1, 0.001, 0.03, 0.0, 0.03);
    fm(st, 1, 0, 100);
    op(st, 2, 60, 6.3);
    amp(st, 2, 0.001, 0.15, 0.0, 0.1);
    out(st, 2, 256);
    reverb(st, 25, 45);
    volume(st, 85);
}

fn nylon(st: &mut State) {
    op(st, 0, 256, 1.0);
    amp(st, 0, 0.002, 1.6, 0.0, 0.4);
    out(st, 0, 256);
    send(st, 0, 120);
    op(st, 1, 256, 1.0);
    amp(st, 1, 0.001, 0.35, 0.05, 0.2);
    fm(st, 1, 0, 90);
    op(st, 2, 70, 2.0);
    amp(st, 2, 0.001, 0.8, 0.0, 0.3);
    out(st, 2, 256);
    reverb(st, 20, 40);
    volume(st, 85);
}

fn pizzicato(st: &mut State) {
    op(st, 0, 256, 1.0);
    saw(st, 0);
    amp(st, 0, 0.002, 0.3, 0.0, 0.15);
    to_filter(st, 0, 0, 256);
    filter(st, 0, 80, 40);
    filter_env(st, 0, 110, 0.001, 0.12, 0.0, 0.1);
    send(st, OPERATORS, 170);
    reverb(st, 30, 55);
    volume(st, 85);
}

fn brass(st: &mut State) {
    op(st, 0, 256, 1.0);
    saw(st, 0);
    amp(st, 0, 0.04, 0.3, 0.9, 0.2);
    to_filter(st, 0, 0, 256);
    filter(st, 0, 70, 40);
    filter_env(st, 0, 130, 0.06, 0.4, 0.55, 0.2);
    send(st, OPERATORS, 140);
    unison(st, 3, 70, 80);
    reverb(st, 25, 45);
    volume(st, 80);
}

fn fm_brass(st: &mut State) {
    op(st, 0, 256, 1.0);
    amp(st, 0, 0.04, 0.2, 0.95, 0.15);
    out(st, 0, 256);
    send(st, 0, 130);
    op(st, 1, 256, 1.0);
    amp(st, 1, 0.07, 0.3, 0.6, 0.15);
    fm(st, 1, 0, 140);
    reverb(st, 25, 45);
    volume(st, 85);
}

fn flute(st: &mut State) {
    op(st, 0, 256, 1.0);
    noise(st, 0, 18);
    amp(st, 0, 0.09, 0.3, 0.85, 0.2);
    out(st, 0, 256);
    send(st, 0, 150);
    op(st, 1, 50, 2.0);
    amp(st, 1, 0.09, 0.3, 0.85, 0.2);
    out(st, 1, 256);
    reverb(st, 30, 50);
    volume(st, 85);
}

fn kick(st: &mut State) {
    op(st, 0, 256, 0.25);
    pitch(st, 0, 3600, 0.08, 1.0);
    amp(st, 0, 0.001, 0.5, 0.0, 0.1);
    feedback(st, 0, 60);
    out(st, 0, 256);
    volume(st, 120);
}

fn snare(st: &mut State) {
    op(st, 0, 256, 0.7);
    pitch(st, 0, 700, 0.03, 1.0);
    amp(st, 0, 0.001, 0.12, 0.0, 0.08);
    out(st, 0, 200);
    op(st, 1, 256, 1.0);
    noise(st, 1, 128);
    amp(st, 1, 0.001, 0.2, 0.0, 0.1);
    to_filter(st, 1, 0, 256);
    filter(st, 0, 200, 30);
    filter_mix(st, 0, 0, 128, 128);
    volume(st, 110);
}

fn clap(st: &mut State) {
    op(st, 0, 256, 1.0);
    noise(st, 0, 128);
    // Three quick bursts, then the tail.
    let bursts = [
        (0.0, 0.0, 0.0),
        (0.001, 1.0, 0.0),
        (0.009, 0.15, 0.3),
        (0.001, 1.0, 0.0),
        (0.009, 0.15, 0.3),
        (0.001, 1.0, 0.0),
        (0.25, 0.0, 0.35),
    ];
    let art = &mut st.modules[0].arts[OP_VOLUME];
    art.env = curve(&bursts, -1, -1);
    art.base = 0.0;
    art.env_amount = 1.0;
    to_filter(st, 0, 0, 256);
    filter(st, 0, 190, 100);
    filter_mix(st, 0, 0, 128, 40);
    send(st, OPERATORS, 120);
    reverb(st, 10, 40);
    volume(st, 110);
}

fn closed_hat(st: &mut State) {
    op(st, 0, 256, 1.0);
    noise(st, 0, 128);
    amp(st, 0, 0.001, 0.05, 0.0, 0.04);
    to_filter(st, 0, 0, 256);
    filter(st, 0, 230, 60);
    filter_mix(st, 0, 0, 0, 128);
    volume(st, 100);
}

fn open_hat(st: &mut State) {
    closed_hat(st);
    amp(st, 0, 0.001, 0.4, 0.0, 0.3);
    volume(st, 95);
}

fn tom(st: &mut State) {
    op(st, 0, 256, 0.5);
    pitch(st, 0, 700, 0.15, 1.0);
    amp(st, 0, 0.001, 0.45, 0.0, 0.15);
    out(st, 0, 256);
    op(st, 1, 60, 1.0);
    noise(st, 1, 128);
    amp(st, 1, 0.001, 0.05, 0.0, 0.05);
    out(st, 1, 60);
    volume(st, 115);
}

fn laser(st: &mut State) {
    op(st, 0, 256, 1.0);
    square(st, 0);
    pitch(st, 0, 4800, 0.25, 1.0);
    amp(st, 0, 0.001, 0.35, 0.0, 0.1);
    out(st, 0, 256);
    send(st, 0, 160);
    delay(st, 96, 60, 70);
    volume(st, 75);
}

fn riser(st: &mut State) {
    op(st, 0, 256, 1.0);
    noise(st, 0, 128);
    amp(st, 0, 2.0, 1.0, 1.0, 1.5);
    to_filter(st, 0, 0, 256);
    filter(st, 0, 30, 150);
    filter_env(st, 0, 230, 4.0, 0.5, 1.0, 1.0);
    send(st, OPERATORS, 180);
    reverb(st, 40, 55);
    volume(st, 90);
}

fn metal_hit(st: &mut State) {
    op(st, 0, 256, 1.0);
    amp(st, 0, 0.001, 1.5, 0.0, 1.0);
    out(st, 0, 256);
    send(st, 0, 170);
    op(st, 1, 256, 1.41);
    amp(st, 1, 0.001, 0.8, 0.0, 0.5);
    fm(st, 1, 0, 150);
    op(st, 2, 256, 3.73);
    amp(st, 2, 0.001, 0.4, 0.0, 0.3);
    fm(st, 2, 0, 110);
    reverb(st, 35, 55);
    volume(st, 85);
}

fn blip(st: &mut State) {
    op(st, 0, 256, 2.0);
    pitch(st, 0, 1200, 0.04, 0.0);
    amp(st, 0, 0.001, 0.12, 0.0, 0.08);
    out(st, 0, 256);
    send(st, 0, 150);
    delay(st, 96, 50, 60);
    volume(st, 80);
}

fn wind(st: &mut State) {
    op(st, 0, 256, 1.0);
    noise(st, 0, 128);
    amp(st, 0, 1.5, 1.0, 1.0, 2.5);
    to_filter(st, 0, 0, 256);
    filter(st, 0, 120, 200);
    filter_mix(st, 0, 0, 128, 0);
    filter_env(st, 0, 60, 2.0, 3.0, 0.7, 2.0);
    send(st, OPERATORS, 200);
    reverb(st, 50, 60);
    volume(st, 95);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::presets::{self, Body};

    /// Every preset is a Citrus state preset that reads back as the state it was made of, by a name of its own, and
    /// plays something.
    #[test]
    fn every_preset_reads_back_as_its_state() {
        let mut seen = std::collections::HashSet::new();
        for patch in PATCHES {
            assert!(seen.insert((patch.category, patch.name)), "{} twice", patch.name);
            let rel = patch.rel();
            let preset = presets::parse(&rel, &patch.file()).expect("a preset");
            assert_eq!(preset.plugin, ID);
            assert_eq!((preset.category.as_str(), preset.name.as_str()), (patch.category, patch.name));
            let Body::State(blob) = preset.body else {
                panic!("{rel} is no state preset");
            };
            let fl = citrus::fl_from_plugin_state(&blob).expect("a Citrus state");
            let st = State::from_bytes(&fl).expect("a Sytrus state");
            assert_eq!(st.name, patch.name);
            assert_eq!(st.matrix, patch.state().matrix, "{rel}");
            let (used, _) = citrus::patch::usage(&st);
            assert!(used.iter().any(|&u| u), "{rel} plays nothing");
        }
        assert!(PATCHES.iter().any(Patch::is_default));
    }

    /// The `Default` preset is a fresh instance's patch.
    #[test]
    fn the_default_is_a_fresh_instance() {
        let default = PATCHES.iter().find(|p| p.is_default()).expect("a Default");
        let st = default.state();
        let fresh = citrus::preset::default_patch();
        assert_eq!((st.matrix, st.s, st.main_volume), (fresh.matrix, fresh.s, fresh.main_volume));
        assert_eq!(st.modules[0].header, fresh.modules[0].header);
    }

    /// A preset's envelopes have time: the master envelope knobs are not left at 0.
    #[test]
    fn the_master_envelope_knobs_are_set() {
        let st = PATCHES.iter().find(|p| p.name == "Warm Pad").expect("Warm Pad").state();
        assert_eq!(&st.s[..8], &[128, 128, 0, 128, 128, 128, 0, 128]);
        assert!(st.modules[0].arts[OP_VOLUME].env.enabled);
    }
}
