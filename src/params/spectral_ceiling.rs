//! Spectral Ceiling ([`honeydew_audio::spectral_ceiling`]), Honeydew's own effect: its params and value texts, in the
//! engine's order and laws.

use honeydew_audio::spectral_ceiling::{
    self as d, ATTACK, CLIP, DEFAULTS, DELTA, DEPTH, HIGH, LOW, PARAMS, RATIO, RELEASE, THRESHOLD,
    TILT,
};

use super::{At, Display, Fmt, ModuleSpec, ParamSpec as P, Sign};

const FREQ: Display = Display::Freq {
    min: 20.0,
    max: 20_000.0,
    hz_decimals: 0,
    khz_decimals: 1,
};

/// Clip: dB over the line, `Off` at the top.
const CLIP_TEXT: Display = Display::Special {
    inner: &Display::linear(0.0, 24.0, 1, " dB"),
    at: At::Max,
    text: "Off",
};

static PARAM_SPECS: [P; PARAMS] = {
    let mut p = [P::ENABLE; PARAMS];
    p[THRESHOLD] = P::cont(
        "Threshold",
        DEFAULTS[THRESHOLD],
        Display::linear(-60.0, 0.0, 1, " dB"),
    );
    p[TILT] = P::cont(
        "Tilt",
        DEFAULTS[TILT],
        Display::Linear {
            min: -12.0,
            max: 12.0,
            fmt: Fmt::new(1, " dB/oct").sign(Sign::Positive),
        },
    );
    p[RATIO] = P::cont("Ratio", DEFAULTS[RATIO], Display::exp(1.0, 20.0, 1, ":1"));
    p[CLIP] = P::cont(
        "Clip",
        DEFAULTS[CLIP],
        CLIP_TEXT,
    );
    p[DEPTH] = P::cont(
        "Depth",
        DEFAULTS[DEPTH],
        Display::linear(0.0, 30.0, 1, " dB"),
    );
    p[ATTACK] = P::cont(
        "Attack",
        DEFAULTS[ATTACK],
        Display::exp(1.0, 100.0, 1, " ms"),
    );
    p[RELEASE] = P::cont(
        "Release",
        DEFAULTS[RELEASE],
        Display::exp(10.0, 1000.0, 0, " ms"),
    );
    p[LOW] = P::cont("Low", DEFAULTS[LOW], FREQ);
    p[HIGH] = P::cont("High", DEFAULTS[HIGH], FREQ);
    p[DELTA] = P::toggle("Delta", DEFAULTS[DELTA]);
    p
};

/// The module. Not in [`super::modules`]: that is the song format's effects.
pub static MODULE: ModuleSpec = ModuleSpec::effect(
    d::SPECTRAL_CEILING,
    "Spectral Ceiling",
    "Spectral Ceiling",
    false,
    &PARAM_SPECS,
    &["CEILING", "TIME", "RANGE"],
);
