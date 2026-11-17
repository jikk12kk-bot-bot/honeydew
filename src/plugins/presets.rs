//! Plugin presets (`.ospreset`): the built-in Lemon plugins' (Lemon Studio's factory presets and our own, built into
//! the app from `assets/lemon-presets/<plugin>/[<category>/]<name>.ospreset` by `build.rs`, and Citrus's, made in code
//! by [`super::citrus_presets`]), which the factory lists as
//! `Presets/<plugin>/…` ([`crate::content`]), and the ones a plugin module's `Save preset` writes to
//! `My Presets/<plugin>` ([`write_state`]). A plugin module's preset picker lists both; a pick loads into the running
//! plugin ([`super::load_preset`]). Lemon Reverb and Lemon Limiter, Honeydew's own modules, take their Lemon plugins'
//! presets too ([`native_params`]).
//!
//! The format is Lemon Studio's (its `factory-presets/README.md`):
//! - line 1 `OSPRESET 1`;
//! - line 2 tab-separated `key=value` fields: `plugin=<shown name>`, `id=clap:<CLAP id>`, `format=clap`, and
//!   `body=params` for a parameter list;
//! - with `body=params`, one `<param name> = <value>` per line (`#` comments and blank lines skipped): a stepped
//!   parameter's value counts its steps from its minimum, any other's is normalized (0..1). A load first puts every
//!   settable parameter back at its default, then sets the listed ones; an unknown name is skipped.
//! - without it, the bytes after line 2 are the plugin's own saved state.

use std::sync::OnceLock;

/// One preset.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    /// The plugin's id in its format (a CLAP id), without the format.
    pub plugin: String,
    /// Its folder below the plugin's (`""` for none).
    pub category: String,
    pub name: String,
    pub body: Body,
}

/// What a preset sets.
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    /// Parameter values by name ([module docs](self)).
    Params(Vec<(String, f64)>),
    /// The plugin's saved state.
    State(Vec<u8>),
}

/// The file type.
pub const EXT: &str = "ospreset";

/// The built-in files: their paths below `assets/lemon-presets` and their bytes.
pub(crate) static FILES: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/lemon_presets.rs"));

/// The preset file at `path` (named by its stem, in its parent folder's category) of `bytes`.
pub fn read(path: &std::path::Path, bytes: &[u8]) -> Option<Preset> {
    let name = path.file_stem()?.to_string_lossy();
    let category = (path.parent().and_then(|p| p.file_name())).map(|c| c.to_string_lossy());
    let rel = format!("x/{}/{name}.{EXT}", category.unwrap_or_default());
    let mut preset = parse(&rel, bytes)?;
    preset.category.clear();
    Some(preset)
}

/// The folder a plugin's presets are in, below `Presets` in the factory and `My Presets` in the user folder: a built-in
/// plugin's name, else the plugin's own name without the characters a file name cannot hold.
pub fn folder(plugin: &crate::song::plugin::PluginRef) -> String {
    if let Some(b) = super::builtin::of(plugin) {
        return b.name.to_owned();
    }
    let clean: String = (plugin.name.chars())
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect();
    match clean.trim() {
        "" => "Plugin".to_owned(),
        name => name.to_owned(),
    }
}

/// A preset file holding `plugin`'s saved state (`state`): what a plugin module's `Save preset` writes.
pub fn write_state(plugin: &crate::song::plugin::PluginRef, state: &[u8]) -> Vec<u8> {
    let name = plugin.name.replace(['\t', '\n', '\r'], " ");
    let header = format!(
        "OSPRESET 1\nplugin={name}\tid={}:{}\tformat={}\n",
        plugin.format, plugin.id, plugin.format
    );
    let mut bytes = header.into_bytes();
    bytes.extend_from_slice(state);
    bytes
}

/// `preset` is for `plugin` (its format's id, without case).
pub fn is_for(preset: &Preset, plugin: &crate::song::plugin::PluginRef) -> bool {
    preset.plugin.eq_ignore_ascii_case(&plugin.id)
}

/// The preset in file `rel` (its path below the presets folder: `<plugin>/[<category>/]<name>.ospreset`) of `bytes`;
/// `None` for anything that is not one.
pub fn parse(rel: &str, bytes: &[u8]) -> Option<Preset> {
    let (first, rest) = split_line(bytes)?;
    if trim(first) != b"OSPRESET 1" {
        return None;
    }
    let (header, body) = split_line(rest)?;
    let header = std::str::from_utf8(trim(header)).ok()?;
    let (mut plugin, mut params) = (None, false);
    for field in header.split('\t') {
        match field.split_once('=') {
            Some(("id", id)) => {
                plugin = Some(id.split_once(':').map_or(id, |(_, id)| id).to_owned())
            }
            Some(("body", "params")) => params = true,
            _ => {}
        }
    }
    let parts: Vec<&str> = rel.split('/').collect();
    let name = parts.last()?.strip_suffix(".ospreset")?.to_owned();
    let category = match parts.len() {
        0..=2 => String::new(),
        n => parts[1..n - 1].join("/"),
    };
    let body = if params {
        let text = std::str::from_utf8(body).ok()?;
        let values = text.lines().filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (name, value) = line.rsplit_once('=')?;
            Some((name.trim().to_owned(), value.trim().parse().ok()?))
        });
        Body::Params(values.collect())
    } else {
        Body::State(body.to_vec())
    };
    Some(Preset {
        plugin: plugin?,
        category,
        name,
        body,
    })
}

fn split_line(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let at = bytes.iter().position(|&b| b == b'\n')?;
    Some((&bytes[..at], &bytes[at + 1..]))
}

fn trim(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// Every built-in preset, read at the first call.
pub fn all() -> &'static [Preset] {
    static ALL: OnceLock<Vec<Preset>> = OnceLock::new();
    ALL.get_or_init(|| {
        // Citrus's are made in code ([`super::citrus_presets`]).
        let citrus = (super::citrus_presets::files().iter())
            .map(|(rel, bytes)| (rel.as_str(), bytes.as_slice()));
        let mut all: Vec<Preset> = (FILES.iter().copied())
            .chain(citrus)
            .filter_map(|(rel, bytes)| parse(rel, bytes))
            .collect();
        all.sort_by_cached_key(|p| order(p));
        all
    })
}

/// The list order: `Default` first, then the presets outside a category, then each category, by name (case aside).
fn order(p: &Preset) -> (bool, bool, String, String) {
    (
        !(p.category.is_empty() && p.name == "Default"),
        !p.category.is_empty(),
        p.category.to_lowercase(),
        p.name.to_lowercase(),
    )
}

/// The presets of the plugin with CLAP id `plugin`, in list order.
pub fn of(plugin: &str) -> Vec<&'static Preset> {
    (all().iter())
        .filter(|p| p.plugin.eq_ignore_ascii_case(plugin))
        .collect()
}

/// The categories of `plugin`'s presets, in list order (`""` first when some have none).
pub fn categories(plugin: &str) -> Vec<&'static str> {
    let mut categories: Vec<&'static str> = Vec::new();
    for p in of(plugin) {
        if !categories.contains(&p.category.as_str()) {
            categories.push(&p.category);
        }
    }
    categories
}

/// Lemon Reverb and Lemon Limiter are Honeydew's own module types, not plugins ([`honeydew_audio::lemon`]); their
/// presets are Lemon Studio's plugins' all the same, by their CLAP ids, and set the module's params by name: Lemon's
/// knobs in Lemon's order (each normalized over its range, which is what a preset's step count from the minimum
/// divided by the range is), then the switches.
const REVERB_ID: &str = "org.openstudio.lemon-reverb";
const LIMITER_ID: &str = "org.openstudio.lemon-limiter";

/// Lemon Reverb's knobs and switches, as its presets name them, in its params' order (after Enable).
const REVERB_NAMES: [&str; 17] = [
    "Low cut",
    "High cut",
    "Predelay",
    "Room size",
    "Diffusion",
    "Decay time",
    "High damping",
    "Bass multiplier",
    "Crossover",
    "Stereo separation",
    "Dry level",
    "Early reflection level",
    "Wet level",
    "Mod Speed",
    "Mod Depth",
    "Tempo-based predelay",
    "Mid / side input",
];

/// Lemon Limiter's knobs, as its presets name them, in its params' order (after Enable).
const LIMITER_NAMES: [&str; 18] = [
    "Gain",
    "Soft saturation threshold",
    "Limiter ceiling",
    "Limiter attack time",
    "Limiter attack curve",
    "Limiter release time",
    "Limiter release curve",
    "Limiter peak window",
    "Comp threshold",
    "Comp ratio",
    "Comp knee",
    "Comp attack time",
    "Comp release time",
    "Comp curve",
    "Comp RMS window",
    "Noise gain",
    "Noise threshold",
    "Noise release time",
];

/// The CLAP id whose presets a native module of type `type_id` takes: Lemon Reverb's and Lemon Limiter's.
pub fn native_plugin(type_id: u16) -> Option<&'static str> {
    use honeydew_audio::lemon::{LIMITER, REVERB};
    match type_id {
        REVERB => Some(REVERB_ID),
        LIMITER => Some(LIMITER_ID),
        _ => None,
    }
}

/// The params of a native module of type `type_id` ([`native_plugin`]) that `preset` makes, from Enable on: a param
/// the preset leaves out at its default, Enable on. `None` for another type or a state preset.
pub fn native_params(type_id: u16, preset: &Preset) -> Option<Vec<f32>> {
    use honeydew_audio::lemon::{self, LIMITER, REVERB, dynamics, reverb};
    let Body::Params(list) = &preset.body else {
        return None;
    };
    let (names, ranges): (&[&str], &[(i32, i32)]) = match type_id {
        REVERB => (&REVERB_NAMES, &reverb::RANGES),
        LIMITER => (&LIMITER_NAMES, &dynamics::RANGES),
        _ => return None,
    };
    let mut params = lemon::defaults(type_id);
    if let Some(enable) = params.first_mut() {
        *enable = 1.0;
    }
    for (k, name) in names.iter().enumerate() {
        let Some(&(_, value)) = list.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) else {
            continue;
        };
        // A knob over its range; a switch (past the knobs) is 0 or 1.
        let span = ranges.get(k).map_or(1, |&(min, max)| (max - min).max(1));
        if let Some(param) = params.get_mut(1 + k) {
            *param = (value / f64::from(span)).clamp(0.0, 1.0) as f32;
        }
    }
    Some(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parameter_list_and_a_state_parse() {
        let text = "OSPRESET 1\nplugin=Lemon Filter\tid=clap:org.openstudio.lemon-filter\tformat=clap\tbody=params\n\
                    # a comment\n\nCutoff frequency = 160\nResonance = 0.5\n";
        let p = parse("Lemon Filter/Party/Next door.ospreset", text.as_bytes()).expect("a preset");
        assert_eq!(p.plugin, "org.openstudio.lemon-filter");
        assert_eq!(
            (p.category.as_str(), p.name.as_str()),
            ("Party", "Next door")
        );
        assert_eq!(
            p.body,
            Body::Params(vec![
                ("Cutoff frequency".into(), 160.0),
                ("Resonance".into(), 0.5)
            ])
        );
        let state =
            b"OSPRESET 1\nplugin=Press\tid=clap:org.openstudio.press\tformat=clap\n\x07\0\n{}";
        let p = parse("Press/Default.ospreset", state).expect("a preset");
        assert_eq!(p.category, "");
        assert_eq!(p.body, Body::State(b"\x07\0\n{}".to_vec()));
        assert!(parse("x/y.ospreset", b"not a preset\n").is_none());
    }

    #[test]
    fn every_built_in_file_is_a_preset_of_a_built_in_plugin() {
        assert_eq!(
            all().len(),
            FILES.len() + super::super::citrus_presets::PATCHES.len()
        );
        for p in all() {
            assert!(
                p.plugin.starts_with("org.openstudio."),
                "{} / {}",
                p.category,
                p.name
            );
        }
        assert_eq!(of("org.openstudio.press")[0].name, "Default");
        assert_eq!(of(super::super::citrus_presets::ID)[0].name, "Default");
        assert!(categories(super::super::citrus_presets::ID).contains(&"Bass"));
    }

    /// A native module's preset value counts steps from its knob's minimum: Lemon Reverb's Default preset's Room size
    /// 49 is stored 50 (range 1..100), its Stereo separation 64 the centre 0 (−64..64), its Low cut 56 is 75 (19..3000).
    #[test]
    fn a_native_preset_counts_from_the_knob_minimum() {
        use honeydew_audio::lemon::{self, LIMITER, REVERB, dynamics, reverb};
        let find = |type_id, name| {
            of(native_plugin(type_id).expect("a native preset plugin"))
                .into_iter()
                .find(|p| p.name == name)
                .expect("the preset")
        };
        let params = native_params(REVERB, find(REVERB, "Default")).expect("its params");
        assert_eq!(params.len(), lemon::defaults(REVERB).len());
        let stored = |k: usize| lemon::stored(reverb::RANGES[k], params[1 + k]);
        assert_eq!(stored(reverb::ROOM), 50);
        assert_eq!(stored(reverb::SEPARATION), 0);
        assert_eq!(stored(reverb::LOW_CUT), 75);
        let params = native_params(LIMITER, find(LIMITER, "Default")).expect("its params");
        assert_eq!(params.len(), lemon::defaults(LIMITER).len());
        assert_eq!(lemon::stored(dynamics::RANGES[0], params[1]), 1000);
    }
}
