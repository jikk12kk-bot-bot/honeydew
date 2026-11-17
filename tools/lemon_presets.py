#!/usr/bin/env python3
"""Honeydew's own Lemon plugin presets: writes `assets/lemon-presets/<plugin>/Honeydew.../<name>.ospreset`.

Each preset starts from a factory preset of the same plugin (its `Default` unless named) and changes some of its
parameters, so every name is one the plugin knows (the script fails on a name the base file lacks). Values are the
OSPRESET format's: a stepped parameter's step count from its minimum (`file = stored - min`).

Run it from the repository root: `python3 tools/lemon_presets.py`. It rewrites the Honeydew folders only.
"""

import math
import os
import shutil

ROOT = os.path.join("assets", "lemon-presets")


def read(plugin, rel="Default"):
    path = os.path.join(ROOT, plugin, rel + ".ospreset")
    with open(path, encoding="utf-8") as f:
        lines = f.read().splitlines()
    header = lines[1]
    values = []
    for line in lines[2:]:
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        name, value = line.rsplit("=", 1)
        values.append([name.strip(), value.strip()])
    return header, values


def write(plugin, category, name, overrides, base="Default"):
    header, values = read(plugin, base)
    names = {n for n, _ in values}
    for key in overrides:
        if key not in names:
            raise SystemExit(f"{plugin} / {name}: no parameter {key!r} in {base}")
    for pair in values:
        if pair[0] in overrides:
            v = overrides[pair[0]]
            pair[1] = str(int(round(v))) if isinstance(v, (int, float)) else v
    folder = os.path.join(ROOT, plugin, category)
    os.makedirs(folder, exist_ok=True)
    body = ["OSPRESET 1", header, "# Honeydew's own preset (tools/lemon_presets.py)."]
    body += [f"{n} = {v}" for n, v in values]
    with open(os.path.join(folder, name + ".ospreset"), "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(body) + "\n")


# ---- Lemon Trio ---------------------------------------------------------------------------------------------------

SHAPES = {"sine": 0, "triangle": 1, "square": 2, "saw": 3, "rounded": 4, "noise": 5, "custom": 6}
FILTERS = {"fast lp": 0, "lp": 1, "bp": 2, "hp": 3, "bs": 4, "lpx2": 5, "svf lp": 6, "svf lpx2": 7}
LFO_SHAPES = {"sine": 0, "triangle": 1, "pulse": 2}


def rise(seconds):
    """An envelope's attack / hold / delay knob for `seconds` (`os_engine::envelope::knob_seconds`)."""
    if seconds <= 0:
        return 100
    return min(65536, round(6553.6 * math.log2(seconds / 0.063922 + 2 ** (100 / 6553.6))))


def fall(seconds):
    """A decay / release knob for `seconds` (the rising law plus 9 ms)."""
    return rise(max(seconds - 0.009, 0.0))


def lfo_speed(hz):
    """An LFO speed knob for `hz` (`lemon_3x_osc::dsp::lfo_hz`: higher is slower)."""
    ms = 1000.0 / hz
    return min(65536, round(65536 * math.log(1 + ms / 64) / math.log(1001)))


def osc(n, shape="sine", coarse=None, fine=0, pan=0, detune=0, phase=0, mix=None):
    o = f"Osc {n} "
    out = {
        o + "shape": SHAPES[shape],
        o + "fine pitch": fine + 100,
        o + "panning": pan + 64,
        o + "stereo detune": detune + 50,
        o + "stereo phase offset": phase + 64,
    }
    if coarse is not None:
        out[o + "coarse pitch"] = coarse + 24
    if mix is not None:
        out[o + "mix level"] = mix
    return out


def env(target, a=0.0, h=0.0, d=0.0, s=128, r=0.05, amt=0, delay=0.0, tension=None):
    """`target`'s envelope on: times in seconds, sustain 0..128, amount (±128, pitch in cents)."""
    e = f"{target} envelope - "
    centre = 2400 if target == "Pitch" else 128
    out = {
        e + "Use envelope": 1,
        e + "Predelay time": rise(delay),
        e + "Attack time": rise(a),
        e + "Hold time": rise(h),
        e + "Decay time": fall(d),
        e + "Sustain level": s,
        e + "Release time": fall(r),
        e + "Modulation amount": amt + centre,
    }
    if tension:
        for stage, t in zip(("Attack", "Decay", "Release"), tension):
            out[e + f"{stage} tension"] = t + 128
    return out


def lfo(target, hz, amt, shape="sine", delay=0.0, attack=0.0):
    l = f"{target} LFO - "
    centre = 2400 if target == "Pitch" else 128
    return {
        l + "Speed": lfo_speed(hz),
        l + "Amount": amt + centre,
        l + "Shape": LFO_SHAPES[shape],
        l + "Predelay time": rise(delay),
        l + "Attack time": rise(attack),
    }


def filt(kind="lp", cutoff=256, res=0):
    return {
        "Filter type": FILTERS[kind],
        "Filter frequency (modulation X)": cutoff,
        "Filter bandwidth (modulation Y)": res,
    }


def trio(category, name, *parts):
    values = {}
    for p in parts:
        values.update(p)
    write("Lemon Trio", "Honeydew " + category, name, values)


def trio_presets():
    off = lambda n: {f"Osc {n} mix level": 0}
    # Bass
    trio("bass", "Sub floor", osc(1, "sine", 0), osc(2, "sine", -12, mix=40), off(3),
         env("Volume", a=0.002, d=0.3, s=110, r=0.08))
    trio("bass", "Rubber pluck", osc(1, "saw", 0), osc(2, "square", -12, mix=70), off(3), filt("lp", 40, 60),
         env("Filter cutoff", d=0.25, s=0, r=0.1, amt=110), env("Volume", d=0.6, s=90, r=0.08))
    trio("bass", "Acid squelch", osc(1, "saw", 0), off(2), off(3), filt("svf lp", 30, 200),
         env("Filter cutoff", d=0.18, s=0, r=0.05, amt=120), env("Volume", s=128, r=0.05))
    trio("bass", "Reese drift", osc(1, "saw", 0, fine=-12), osc(2, "saw", 0, fine=12, mix=100),
         osc(3, "saw", -12, mix=40), filt("lp", 120, 40), lfo("Filter cutoff", 0.25, 30),
         env("Volume", a=0.005, s=128, r=0.15))
    trio("bass", "Wobble", osc(1, "saw", 0), osc(2, "square", -12, mix=90), off(3), filt("lpx2", 50, 120),
         lfo("Filter cutoff", 3.0, 90), env("Volume", s=128, r=0.08))
    trio("bass", "Square thump", osc(1, "square", 0), osc(2, "sine", -12, mix=80), off(3), filt("lp", 70, 20),
         env("Filter cutoff", d=0.12, s=0, amt=80), env("Volume", d=0.4, s=70, r=0.06))
    trio("bass", "Growler", osc(1, "saw", 0), osc(2, "rounded", 0, fine=7, mix=100), osc(3, "noise", 0, mix=10),
         filt("bp", 90, 150), lfo("Filter cutoff", 6.0, 40, "triangle"), env("Volume", s=128, r=0.08))
    trio("bass", "Octave bounce", osc(1, "square", 0), osc(2, "square", 12, mix=60), osc(3, "sine", -12, mix=64),
         filt("lp", 80, 80), env("Filter cutoff", d=0.09, s=0, amt=100), env("Volume", d=0.25, s=60, r=0.06))
    # Leads
    trio("leads", "Neon saw", osc(1, "saw", 0, fine=-8), osc(2, "saw", 0, fine=8, mix=110),
         osc(3, "saw", 12, mix=30), filt("lp", 170, 40), lfo("Pitch", 5.5, 15, delay=0.35, attack=0.4),
         env("Volume", a=0.005, s=128, r=0.2))
    trio("leads", "Chip hero", osc(1, "square", 0), osc(2, "square", 12, mix=30), off(3),
         lfo("Pitch", 6.0, 20, delay=0.25), env("Volume", d=0.2, s=100, r=0.05))
    trio("leads", "Hollow flute", osc(1, "triangle", 0), osc(2, "sine", 12, mix=40), osc(3, "noise", 0, mix=6),
         filt("bp", 150, 40), lfo("Pitch", 5.0, 10, delay=0.4, attack=0.3), env("Volume", a=0.06, s=128, r=0.15))
    trio("leads", "Screamer", osc(1, "saw", 0), osc(2, "square", 0, fine=5, mix=90), off(3), filt("svf lp", 110, 210),
         env("Filter cutoff", a=0.01, d=0.4, s=40, amt=60), env("Volume", s=128, r=0.1))
    trio("leads", "Glass sine", osc(1, "sine", 0), osc(2, "triangle", 12, mix=50), osc(3, "sine", 24, mix=18),
         lfo("Pitch", 4.5, 12, delay=0.5, attack=0.4), env("Volume", a=0.01, s=120, r=0.3))
    trio("leads", "Fifth buzz", osc(1, "rounded", 0), osc(2, "saw", 7, mix=60), osc(3, "saw", -12, mix=50),
         filt("lp", 140, 90), env("Filter cutoff", d=0.3, s=30, amt=70), env("Volume", s=128, r=0.1))
    trio("leads", "Vowel lead", osc(1, "saw", 0), osc(2, "square", 0, fine=4, mix=60), off(3), filt("bp", 120, 120),
         lfo("Filter cutoff", 1.5, 35, "triangle"), env("Volume", s=128, r=0.12))
    # Pads
    trio("pads", "Warm blanket", osc(1, "saw", 0, fine=-10, detune=8), osc(2, "saw", 0, fine=10, detune=-8, mix=100),
         osc(3, "triangle", -12, mix=50), filt("lp", 90, 30),
         env("Filter cutoff", a=1.5, d=2.0, s=60, r=1.5, amt=50), env("Volume", a=0.8, d=1.0, s=128, r=1.5))
    trio("pads", "Glass cathedral", osc(1, "triangle", 0), osc(2, "sine", 12, detune=6, mix=80),
         osc(3, "sine", 24, mix=30), filt("lp", 200, 0), env("Volume", a=1.2, s=128, r=2.5))
    trio("pads", "Dark drone", osc(1, "saw", -12), osc(2, "saw", -12, fine=6, mix=110), osc(3, "square", -24, mix=60),
         filt("lp", 45, 80), lfo("Filter cutoff", 0.08, 35), env("Volume", a=2.0, s=128, r=3.0))
    trio("pads", "Breathing strings", osc(1, "saw", 0, detune=5), osc(2, "saw", 0, fine=6, detune=-5, mix=100),
         osc(3, "saw", -12, fine=-6, mix=60), filt("lp", 100, 20), lfo("Filter cutoff", 0.2, 25),
         env("Volume", a=0.5, s=128, r=1.2))
    trio("pads", "Choir haze", osc(1, "rounded", 0), osc(2, "rounded", 0, fine=9, mix=100),
         osc(3, "triangle", 12, mix=30), filt("bp", 130, 60), lfo("Pitch", 5.0, 6, attack=0.6),
         env("Volume", a=0.7, s=128, r=1.4))
    trio("pads", "Rising sun", osc(1, "saw", 0), osc(2, "square", 0, fine=7, mix=70), osc(3, "saw", 12, mix=40),
         filt("lp", 30, 70), env("Filter cutoff", a=4.0, d=3.0, s=100, r=2.0, amt=110),
         env("Volume", a=1.0, s=128, r=2.0))
    trio("pads", "Tremolo glow", osc(1, "triangle", 0), osc(2, "saw", 0, fine=5, mix=60), off(3), filt("lp", 120, 10),
         lfo("Volume", 4.0, 50), env("Volume", a=0.4, s=128, r=1.0))
    # Keys and plucks
    trio("keys and plucks", "Soft e-piano", osc(1, "sine", 0), osc(2, "sine", 12, mix=40),
         osc(3, "triangle", 0, fine=4, mix=30), env("Volume", a=0.002, d=1.6, s=20, r=0.4))
    trio("keys and plucks", "Bright pluck", osc(1, "saw", 0), osc(2, "saw", 0, fine=8, mix=90),
         osc(3, "square", 12, mix=30), filt("lp", 60, 60), env("Filter cutoff", d=0.15, s=0, amt=128),
         env("Volume", d=0.5, s=0, r=0.3))
    trio("keys and plucks", "Marimba wood", osc(1, "sine", 0), osc(2, "triangle", 24, mix=20), off(3),
         env("Pitch", d=0.03, s=0, r=0.0, amt=30), env("Volume", d=0.35, s=0, r=0.2))
    trio("keys and plucks", "Harp drops", osc(1, "triangle", 0), osc(2, "sine", 12, mix=50), off(3),
         filt("lp", 140, 0), env("Filter cutoff", d=0.4, s=0, amt=60), env("Volume", d=1.2, s=0, r=0.8))
    trio("keys and plucks", "Drawbar organ", osc(1, "sine", 0), osc(2, "sine", 12, mix=90), osc(3, "sine", -12, mix=70),
         lfo("Pitch", 6.0, 4), env("Volume", a=0.004, s=128, r=0.05))
    trio("keys and plucks", "Clavi funk", osc(1, "square", 0), osc(2, "saw", 0, fine=3, mix=50), off(3),
         filt("bp", 160, 70), env("Filter cutoff", d=0.12, s=0, amt=60), env("Volume", d=0.3, s=20, r=0.05))
    trio("keys and plucks", "Kalimba", osc(1, "sine", 0), osc(2, "sine", 24, detune=4, mix=25),
         osc(3, "triangle", 12, mix=15), env("Pitch", d=0.01, s=0, r=0.0, amt=100), env("Volume", d=0.6, s=0, r=0.4))
    trio("keys and plucks", "Music box", osc(1, "sine", 12), osc(2, "triangle", 24, mix=30), off(3), filt("lp", 220, 0),
         env("Volume", d=1.0, s=0, r=1.0))
    trio("keys and plucks", "Chord stab", osc(1, "saw", 0), osc(2, "saw", 0, fine=12, mix=110),
         osc(3, "square", -12, mix=50), filt("lp", 80, 40), env("Filter cutoff", d=0.2, s=10, amt=100),
         env("Volume", d=0.25, s=0, r=0.15))
    # Effects
    trio("FX", "Riser", osc(1, "noise", 0), osc(2, "saw", 0, mix=40), off(3), filt("lp", 20, 120),
         env("Filter cutoff", a=6.0, s=128, r=1.0, amt=128), env("Pitch", a=6.0, s=128, r=1.0, amt=1200),
         env("Volume", a=2.0, s=128, r=1.0))
    trio("FX", "Laser drop", osc(1, "square", 0), osc(2, "saw", 12, mix=40), off(3),
         env("Pitch", d=0.25, s=0, r=0.0, amt=2400), env("Volume", d=0.4, s=0, r=0.1))
    trio("FX", "Ocean sweep", osc(1, "noise", 0), off(2), off(3), filt("lp", 60, 100), lfo("Filter cutoff", 0.15, 90),
         env("Volume", a=1.5, s=128, r=2.0))
    trio("FX", "Siren", osc(1, "triangle", 0), osc(2, "square", 12, mix=30), off(3),
         lfo("Pitch", 0.6, 700, "triangle"), env("Volume", s=128, r=0.2))
    trio("FX", "Water drip", osc(1, "sine", 12), off(2), off(3), env("Pitch", d=0.06, s=0, r=0.0, amt=-1200),
         env("Volume", d=0.15, s=0, r=0.1))
    trio("FX", "Robot chatter", osc(1, "square", 0), osc(2, "saw", 7, mix=60), off(3), filt("bp", 140, 100),
         lfo("Pitch", 8.0, 500, "pulse"), env("Volume", s=128, r=0.1))


# ---- Effects ------------------------------------------------------------------------------------------------------


def fx(plugin, name, values, base="Default"):
    write(plugin, "Honeydew", name, values, base)


def effect_presets():
    # Fast Distortion: Pre amp 64..192, Threshold 1..10 (file: from the minimum), Type A/B, Mix and Post gain 0..128.
    fd = lambda pre, thr, kind, mix, post: {
        "Pre amp": pre - 64, "Threshold": thr - 1, "Type": kind, "Mix": mix, "Post gain": post}
    fx("Lemon Fast Distortion", "Gentle crunch", fd(154, 8, 0, 70, 110))
    fx("Lemon Fast Distortion", "Tape saturation", fd(144, 9, 1, 90, 115))
    fx("Lemon Fast Distortion", "Broken speaker", fd(192, 2, 1, 128, 70))
    fx("Lemon Fast Distortion", "Parallel grit", fd(184, 4, 0, 50, 120))
    fx("Lemon Fast Distortion", "Bass warmer", fd(139, 7, 1, 60, 124))
    # Soft Clipper: Threshold 1..127, Post gain 0..160.
    sc = lambda thr, post: {"Threshold": thr - 1, "Post gain": post}
    fx("Lemon Soft Clipper", "Gentle glue", sc(110, 128))
    fx("Lemon Soft Clipper", "Loud and proud", sc(61, 128))
    fx("Lemon Soft Clipper", "Hard squash", sc(31, 120))
    fx("Lemon Soft Clipper", "Drum smash", sc(46, 124))
    # Chorus.
    ch = lambda delay, depth, stereo, s1, s2, s3, w1=0, w2=0, w3=0, cross=0, cutoff=2500, wet=0: {
        "Delay": delay, "Depth": depth, "Stereo": stereo, "LFO 1 speed": s1, "LFO 2 speed": s2, "LFO 3 speed": s3,
        "LFO 1 wave": w1, "LFO 2 wave": w2, "LFO 3 wave": w3, "Cross type": cross, "Cross cutoff": cutoff,
        "Wet only": wet}
    fx("Lemon Chorus", "Wide shimmer", ch(300, 3000, 900, 1200, 2200, 3100))
    fx("Lemon Chorus", "Slow swirl", ch(600, 4200, 700, 500, 800, 1100))
    fx("Lemon Chorus", "Subtle doubler", ch(60, 800, 1024, 2000, 0, 0))
    fx("Lemon Chorus", "Seasick", ch(400, 5000, 300, 3800, 4200, 4600, 1024, 1024, 1024))
    fx("Lemon Chorus", "Bass-safe chorus", ch(200, 2000, 600, 1500, 2500, 0, cross=65, cutoff=3500))
    # Flanger.
    fx("Lemon Flanger", "Jet takeoff", {"Delay": 100, "Depth": 4500, "Rate": 900, "Feedback": 80})
    fx("Lemon Flanger", "Metallic comb", {"Delay": 300, "Depth": 1000, "Rate": 400, "Feedback": 90, "Damp": 1000})
    fx("Lemon Flanger", "Gentle sweep", {"Depth": 2500, "Rate": 1500, "Feedback": 30, "Wet": 2000})
    fx("Lemon Flanger", "Inside out", {"Depth": 3500, "Rate": 1200, "Feedback": 50, "Invert wet": 0})
    # Phaser.
    fx("Lemon Phaser", "Liquid phase", {"Sweep frequency": 1800, "Minimum depth": 50, "Maximum depth": 900,
                                        "Stages": 9, "Feedback": 600, "Dry - wet": 512})
    fx("Lemon Phaser", "Slow drift", {"Sweep frequency": 900, "Stages": 6, "Feedback": 300, "Stereo": 900,
                                      "Dry - wet": 512})
    fx("Lemon Phaser", "Funky phase", {"Sweep frequency": 3500, "Minimum depth": 200, "Maximum depth": 700,
                                       "Stages": 4, "Feedback": 750, "Dry - wet": 512})
    fx("Lemon Phaser", "Deep space", {"Sweep frequency": 600, "Stages": 15, "Feedback": 850, "Dry - wet": 600})
    # Filter.
    fl = lambda cutoff, res, lp, bp, hp, x2: {
        "Cutoff frequency": cutoff, "Resonance": res, "Low pass": lp, "Band pass": bp, "High pass": hp, "X 2": x2}
    fx("Lemon Filter", "Telephone", fl(600, 300, 0, 1024, 300, 1))
    fx("Lemon Filter", "Muffled club", fl(250, 400, 1024, 0, 0, 1))
    fx("Lemon Filter", "Thin air", fl(700, 200, 0, 0, 1024, 0))
    fx("Lemon Filter", "Resonant peak", fl(520, 900, 1024, 0, 0, 0))
    # Transient Processor.
    tp = lambda att, drive, rel, gain, ashape=1, rshape=1, split=80: {
        "Attack": att, "Drive": drive, "Release": rel, "Gain": gain, "Attack shape": ashape,
        "Release shape": rshape, "Split frequency": split}
    fx("Lemon Transient Processor", "Punchy drums", tp(130, 20, 70, 80, 0, 1, 0))
    fx("Lemon Transient Processor", "Tight room", tp(90, 0, 30, 84))
    fx("Lemon Transient Processor", "Long tails", tp(70, 0, 140, 76, 1, 2))
    fx("Lemon Transient Processor", "Soft pluck", tp(30, 0, 90, 84, 2, 1))
    # Stereo Enhancer: Stereo separation −96..96 and Phase offset −512..512 from their minimums.
    se = lambda sep, phase: {"Stereo separation": sep + 96, "Phase offset": phase + 512}
    fx("Lemon Stereo Enhancer", "Extra wide", se(-80, 60))
    fx("Lemon Stereo Enhancer", "Gentle width", se(-25, 0))
    fx("Lemon Stereo Enhancer", "Narrow focus", se(50, 0))
    # Lemon Reverb (stored values; the file counts from each knob's minimum).
    lows = {"Low cut": 19, "High cut": 5, "Room size": 1, "Decay time": 1, "High damping": 5,
            "Bass multiplier": 20, "Crossover": 25, "Stereo separation": -64}

    def reverb(name, **stored):
        names = {"low": "Low cut", "high": "High cut", "predelay": "Predelay", "room": "Room size",
                 "diffusion": "Diffusion", "decay": "Decay time", "damping": "High damping",
                 "bass": "Bass multiplier", "crossover": "Crossover", "wet": "Wet level", "early": "Early reflection level",
                 "mod_speed": "Mod Speed", "mod_depth": "Mod Depth"}
        values = {names[k]: v - lows.get(names[k], 0) for k, v in stored.items()}
        fx("Lemon Reverb", name, values)

    reverb("Vocal plate", low=150, room=30, decay=18, predelay=20, wet=95, mod_depth=300)
    reverb("Huge ambience", room=100, decay=120, predelay=40, wet=90, mod_speed=600, mod_depth=500)
    reverb("Tight booth", low=200, room=8, decay=3, wet=85)
    reverb("Endless wash", room=80, decay=180, wet=100, mod_speed=1200, mod_depth=1200)
    reverb("Dark chamber", high=21, damping=31, room=55, decay=30, wet=95)
    reverb("Snare room", low=120, room=25, decay=8, predelay=5, wet=105)
    # Lemon Limiter's compressor (file values as the factory presets have them).
    lim = lambda gain, thr, ratio, knee, att, rel, curve=1: {
        "Gain": gain, "Comp threshold": thr, "Comp ratio": ratio, "Comp knee": knee, "Comp attack time": att,
        "Comp release time": rel, "Comp curve": curve}
    fx("Lemon Limiter", "Vocal leveler", lim(1500, 300, 1400, 2000, 5000, 7000))
    fx("Lemon Limiter", "Bus glue", lim(1300, 500, 1250, 2000, 6500, 7600))
    fx("Lemon Limiter", "Bass tamer", lim(1450, 250, 1500, 1500, 4500, 6800))
    fx("Lemon Limiter", "Gentle master", {"Gain": 1400, "Limiter peak window": 1115})


if __name__ == "__main__":
    for plugin in os.listdir(ROOT):
        for entry in os.listdir(os.path.join(ROOT, plugin)):
            if entry.startswith("Honeydew"):
                shutil.rmtree(os.path.join(ROOT, plugin, entry))
    trio_presets()
    effect_presets()
