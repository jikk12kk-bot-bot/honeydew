//! The sample files of the built-in Lemon samplers (#168): Lemon Sampler, Lemon Drum Sampler and Lemon Slicer open
//! their samples themselves, so a module of one names its files inside its plugin's state, by the path they had on
//! the machine that saved the song, not by a song shorthand. A project archive takes those files too, and opening it
//! leads the paths to the files it brought.
//!
//! The state is nih-plug's JSON, after an 8-byte little-endian length when the plugin wrote it (as
//! `os_hostlink::state::json` reads it); its `fields` are JSON in strings:
//! - Lemon Sampler: `sample-path`, the file;
//! - Lemon Slicer: `sample-path`, the file to open (\"\" = the one in `state`), and `state`, the original's saved state
//!   in hex, which names the loop;
//! - Lemon Drum Sampler: `state`, the original's saved state in hex, whose layers name their files, and
//!   `sample-paths`, files given for layers. Each path is changed where it is, by value: what a given file's index
//!   counts differs inside the plugin (every layer, or only those that name a file).

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use lemon_drum_sampler::state::{L_PATH, Node, P_LAYER, PAD};
use serde_json::{Map, Value};

use crate::song::Song;
use crate::song::plugin::{PluginRef, is_plugin};

const SAMPLER: &str = "org.openstudio.open-sampler";
const DRUM_SAMPLER: &str = "org.openstudio.lemon-drum-sampler";
const SLICER: &str = "org.openstudio.lemon-slicer";

/// The sample fields.
const SAMPLE_FIELD: &str = "sample-path";
const PATHS_FIELD: &str = "sample-paths";
const STATE_FIELD: &str = "state";

#[derive(Clone, Copy)]
enum Kind {
    Sampler,
    DrumSampler,
    Slicer,
}

/// Which Lemon sampler `plugin` is: only the built-in CLAP plugins, whose state this reads.
fn kind(plugin: &PluginRef) -> Option<Kind> {
    if !plugin.format.eq_ignore_ascii_case("clap") {
        return None;
    }
    let kinds = [
        (SAMPLER, Kind::Sampler),
        (DRUM_SAMPLER, Kind::DrumSampler),
        (SLICER, Kind::Slicer),
    ];
    (kinds.into_iter())
        .find(|(id, _)| plugin.id.eq_ignore_ascii_case(id))
        .map(|(_, kind)| kind)
}

/// The Lemon sampler plugins of `song`'s racks.
fn plugins(song: &Song) -> impl Iterator<Item = &PluginRef> {
    (song.channels.iter())
        .flat_map(|c| &c.rack.modules)
        .filter(|m| is_plugin(m.type_id))
        .filter_map(|m| m.file.plugin.as_deref())
        .filter(|p| kind(p).is_some())
}

/// Every sample file the Lemon samplers of `song` name, as named (a path on the machine that saved it), each once.
pub(crate) fn sample_files(song: &Song) -> Vec<String> {
    let mut files = Vec::new();
    for plugin in plugins(song) {
        files.extend(files_of(plugin));
    }
    files.sort();
    files.dedup();
    files
}

/// The sample files `plugin`'s state names; none for a plugin that is no Lemon sampler.
pub(crate) fn files_of(plugin: &PluginRef) -> Vec<String> {
    let mut files = Vec::new();
    if let Some(kind) = kind(plugin) {
        visit(kind, &plugin.state, &mut |path| {
            files.push(path.to_owned());
            None
        });
    }
    files
}

/// Each sample path the Lemon samplers of `song` name that `map` maps changed to what it maps to.
pub(crate) fn relink(song: &mut Song, map: &dyn Fn(&str) -> Option<String>) {
    let modules = (song.channels.iter_mut()).flat_map(|c| &mut c.rack.modules);
    for module in modules.filter(|m| is_plugin(m.type_id)) {
        let Some(plugin) = module.file.plugin.as_deref_mut() else {
            continue;
        };
        let Some(kind) = kind(plugin) else { continue };
        if let Some(state) = visit(kind, &plugin.state, &mut |path| map(path)) {
            plugin.state = Arc::from(state);
        }
    }
}

/// Every sample path (not \"\") of a `kind` plugin's state `blob` goes through `each`; one it returns takes its place.
/// The state with them, `None` when none changed or it is no state this reads.
fn visit(kind: Kind, blob: &[u8], each: &mut dyn FnMut(&str) -> Option<String>) -> Option<Vec<u8>> {
    let (prefixed, json) = split_length(blob);
    let mut state: Value = serde_json::from_slice(json).ok()?;
    let fields = state.get_mut("fields")?.as_object_mut()?;
    let mut changed = false;
    if matches!(kind, Kind::Sampler | Kind::Slicer) {
        changed |= visit_string(fields, each);
    }
    match kind {
        Kind::Sampler => {}
        Kind::Slicer => {
            changed |= visit_raw(fields, &mut |raw| {
                let mut slicer = lemon_slicer::state::State::parse(raw)?;
                let path = slicer.path();
                let new = (!path.is_empty()).then(|| each(&path)).flatten()?;
                slicer.set_path(&new);
                Some(slicer.to_bytes())
            });
        }
        Kind::DrumSampler => {
            changed |= visit_raw(fields, &mut |raw| drum_layers(raw, each));
            changed |= visit_list(fields, each);
        }
    }
    if !changed {
        return None;
    }
    let json = serde_json::to_vec(&state).ok()?;
    Some(match prefixed {
        true => [&(json.len() as u64).to_le_bytes()[..], &json].concat(),
        false => json,
    })
}

/// The JSON of state `blob`, and whether its length came before it: told by the length's value, as a low byte of it can
/// be `{` itself.
fn split_length(blob: &[u8]) -> (bool, &[u8]) {
    match blob.split_first_chunk::<8>() {
        Some((len, json)) if u64::from_le_bytes(*len) == json.len() as u64 => (true, json),
        _ => (false, blob),
    }
}

/// `sample-path`, a JSON string in the field's string.
fn visit_string(
    fields: &mut Map<String, Value>,
    each: &mut dyn FnMut(&str) -> Option<String>,
) -> bool {
    let Some(field) = fields.get_mut(SAMPLE_FIELD) else {
        return false;
    };
    let Some(path) = (field.as_str()).and_then(|s| serde_json::from_str::<String>(s).ok()) else {
        return false;
    };
    let new = (!path.is_empty()).then(|| each(&path)).flatten();
    match new.and_then(|new| serde_json::to_string(&new).ok()) {
        Some(new) => {
            *field = Value::String(new);
            true
        }
        None => false,
    }
}

/// `sample-paths`, a JSON list of strings in the field's string.
fn visit_list(
    fields: &mut Map<String, Value>,
    each: &mut dyn FnMut(&str) -> Option<String>,
) -> bool {
    let Some(field) = fields.get_mut(PATHS_FIELD) else {
        return false;
    };
    let Some(mut paths) =
        (field.as_str()).and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
    else {
        return false;
    };
    let mut changed = false;
    for path in paths.iter_mut().filter(|p| !p.is_empty()) {
        if let Some(new) = each(path) {
            *path = new;
            changed = true;
        }
    }
    match (changed, serde_json::to_string(&paths)) {
        (true, Ok(list)) => {
            *field = Value::String(list);
            true
        }
        _ => false,
    }
}

/// `state`, the original's saved state as hex in quotes, in the field's string, through `edit`.
fn visit_raw(
    fields: &mut Map<String, Value>,
    edit: &mut dyn FnMut(&[u8]) -> Option<Vec<u8>>,
) -> bool {
    let Some(field) = fields.get_mut(STATE_FIELD) else {
        return false;
    };
    let new = (field.as_str())
        .and_then(|s| from_hex(s.trim_matches('"')))
        .and_then(|raw| edit(&raw));
    match new {
        Some(raw) => {
            *field = Value::String(format!("\"{}\"", hex(&raw)));
            true
        }
        None => false,
    }
}

/// A Lemon Drum Sampler's saved state `raw` with each of its layers' paths through `each`; `None` when none changed.
fn drum_layers(raw: &[u8], each: &mut dyn FnMut(&str) -> Option<String>) -> Option<Vec<u8>> {
    let mut kit = lemon_drum_sampler::state::State::parse(raw)?;
    let mut changed = false;
    let pads = kit.nodes.iter_mut().filter_map(|n| match n {
        Node::Tree(PAD, kids) => Some(kids),
        _ => None,
    });
    let layers = pads.flatten().filter_map(|n| match n {
        Node::Tree(P_LAYER, kids) => Some(kids),
        _ => None,
    });
    for node in layers.flatten() {
        if let Node::Leaf(L_PATH, bytes) = node {
            let path = lemon_drum_sampler::state::text(bytes);
            if let Some(new) = (!path.is_empty()).then(|| each(&path)).flatten() {
                *bytes = new.encode_utf16().flat_map(u16::to_le_bytes).collect();
                changed = true;
            }
        }
    }
    changed.then(|| kit.to_bytes())
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    (s.len() % 2 == 0).then_some(())?;
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The ends of sample path `path` (`/` or `\` separated, of any machine), longest first: `a/b/c.wav`, `b/c.wav`,
/// `c.wav`. Only from the last part that could leave a folder (a root, a drive, `..`) on.
pub(crate) fn tails(path: &str) -> Vec<PathBuf> {
    let parts: Vec<&str> = path.split(['/', '\\']).collect();
    let fine = |part: &&str| {
        !part.contains(':')
            && matches!(
                Path::new(part).components().collect::<Vec<_>>()[..],
                [Component::Normal(_)]
            )
    };
    let start = (parts.iter().rposition(|p| !fine(p))).map_or(0, |at| at + 1);
    (start..parts.len())
        .map(|from| parts[from..].iter().collect())
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::song::plugin::PLUGIN_INSTRUMENT;
    use crate::song::{ChannelKind, SynthKind};

    /// nih-plug's state of `fields`, after its length.
    fn blob(fields: &[(&str, String)]) -> Vec<u8> {
        let fields: Map<String, Value> = (fields.iter())
            .map(|(k, v)| (k.to_string(), Value::String(v.clone())))
            .collect();
        let json = serde_json::json!({"version": "1", "params": {"gain": 0.5}, "fields": fields});
        let json = serde_json::to_vec(&json).unwrap();
        [&(json.len() as u64).to_le_bytes()[..], &json].concat()
    }

    fn quoted(path: &str) -> String {
        serde_json::to_string(path).unwrap()
    }

    pub(crate) fn plugin(id: &str, state: Vec<u8>) -> PluginRef {
        PluginRef {
            format: "clap".into(),
            id: id.into(),
            state: Arc::from(state),
            ..PluginRef::default()
        }
    }

    pub(crate) fn sampler(path: &str) -> PluginRef {
        plugin(SAMPLER, blob(&[(SAMPLE_FIELD, quoted(path))]))
    }

    /// A Lemon Slicer whose state names `stored` and whose file to open is `given`.
    pub(crate) fn slicer(stored: &str, given: &str) -> PluginRef {
        let mut st = lemon_slicer::state::State::default();
        st.set_path(stored);
        let raw = format!("\"{}\"", hex(&st.to_bytes()));
        plugin(
            SLICER,
            blob(&[(STATE_FIELD, raw), (SAMPLE_FIELD, quoted(given))]),
        )
    }

    /// A Lemon Drum Sampler with `samples` on pads by index, as the editor's load puts them, and `given` files.
    pub(crate) fn drums(samples: &[(usize, &str)], given: &[&str]) -> PluginRef {
        let mut st = lemon_drum_sampler::state::State::default();
        for &(pad, path) in samples {
            st.set_sample(pad, path);
        }
        let given: Vec<String> = given.iter().map(|g| g.to_string()).collect();
        plugin(
            DRUM_SAMPLER,
            lemon_drum_sampler::params::plugin_state_from_fl(&st.to_bytes(), &given),
        )
    }

    /// A song with a MiniSynth channel whose rack hosts `plugins` after its synth.
    pub(crate) fn song_of(plugins: Vec<PluginRef>) -> Song {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let rack = &mut song.channels.last_mut().unwrap().rack;
        for p in plugins {
            let id = rack.add(PLUGIN_INSTRUMENT);
            rack.get_mut(id).unwrap().file.plugin = Some(Box::new(p));
        }
        song
    }

    /// The plugins of `song`'s racks, in order.
    pub(crate) fn plugins_in(song: &Song) -> Vec<&PluginRef> {
        (song.channels.iter())
            .flat_map(|c| &c.rack.modules)
            .filter_map(|m| m.file.plugin.as_deref())
            .collect()
    }

    #[test]
    fn each_lemon_sampler_names_its_files() {
        assert_eq!(
            files_of(&sampler("/u/My Samples/a.wav")),
            ["/u/My Samples/a.wav"]
        );
        assert!(files_of(&sampler("")).is_empty());
        assert_eq!(files_of(&slicer("/u/loop.wav", "")), ["/u/loop.wav"]);
        assert_eq!(
            files_of(&slicer("/u/old.wav", "/u/new.wav")),
            ["/u/new.wav", "/u/old.wav"]
        );
        // Pad 3 only, after empty pads: a given file is counted wherever it is.
        let kit = drums(&[(2, "/u/snare.wav")], &["", "/u/kick.wav"]);
        assert_eq!(files_of(&kit), ["/u/snare.wav", "/u/kick.wav"]);
        // Another plugin, or Lemon Sampler as a VST3, is none of them.
        let mut other = sampler("/u/a.wav");
        other.id = "org.openstudio.lemon-chorus".into();
        assert!(files_of(&other).is_empty());
        other = sampler("/u/a.wav");
        other.format = "vst3".into();
        assert!(files_of(&other).is_empty());
    }

    #[test]
    fn a_song_names_each_file_once() {
        let song = song_of(vec![
            sampler("/u/a.wav"),
            sampler("/u/a.wav"),
            slicer("/u/b.wav", ""),
        ]);
        assert_eq!(sample_files(&song), ["/u/a.wav", "/u/b.wav"]);
    }

    #[test]
    fn relinking_changes_each_path_where_it_is() {
        let mut song = song_of(vec![
            sampler("C:\\u\\a.wav"),
            slicer("/u/old.wav", "/u/b.wav"),
            drums(&[(2, "/u/snare.wav"), (5, "/x/keep.wav")], &["/u/kick.wav"]),
        ]);
        let unchanged = song.clone();
        let map = |path: &str| {
            let name = tails(path).last().unwrap().display().to_string();
            (!path.starts_with("/x")).then(|| format!("/new/{name}"))
        };
        relink(&mut song, &map);
        let states: Vec<Vec<String>> = plugins_in(&song).into_iter().map(files_of).collect();
        assert_eq!(
            states,
            [
                vec!["/new/a.wav"],
                vec!["/new/b.wav", "/new/old.wav"],
                vec!["/new/snare.wav", "/x/keep.wav", "/new/kick.wav"],
            ]
        );
        // The length before the JSON stays right, and the rest of the state stays.
        for (new, old) in plugins_in(&song).into_iter().zip(plugins_in(&unchanged)) {
            let (prefixed, json) = split_length(&new.state);
            assert!(prefixed);
            let new: Value = serde_json::from_slice(json).unwrap();
            let old: Value = serde_json::from_slice(split_length(&old.state).1).unwrap();
            assert_eq!(new["params"], old["params"]);
            assert_eq!(new["version"], old["version"]);
        }
        // A drum state still reads, its pads where they were.
        let kit = plugins_in(&song)[2];
        let raw = lemon_drum_sampler::params::fl_from_plugin_state(&kit.state).unwrap();
        let pads = lemon_drum_sampler::state::State::parse(&raw)
            .unwrap()
            .pads();
        assert_eq!(pads[2].layers[0].path, "/new/snare.wav");
        assert_eq!(pads[5].layers[0].path, "/x/keep.wav");
        assert_eq!(pads.len(), 32);
    }

    #[test]
    fn nothing_mapped_leaves_the_state_as_it_was() {
        let mut song = song_of(vec![sampler("/u/a.wav")]);
        let before = plugins_in(&song)[0].state.clone();
        relink(&mut song, &|_| None);
        assert!(Arc::ptr_eq(&before, &plugins_in(&song)[0].state));
    }

    #[test]
    fn a_paths_tails_never_leave_a_folder() {
        let tails_of = |p: &str| -> Vec<String> {
            tails(p)
                .iter()
                .map(|t| {
                    t.iter()
                        .map(|c| c.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/")
                })
                .collect()
        };
        assert_eq!(
            tails_of("/home/me/My Samples/a.wav"),
            [
                "home/me/My Samples/a.wav",
                "me/My Samples/a.wav",
                "My Samples/a.wav",
                "a.wav"
            ]
        );
        assert_eq!(
            tails_of("C:\\Users\\me\\a.wav"),
            ["Users/me/a.wav", "me/a.wav", "a.wav"]
        );
        assert_eq!(tails_of("x/../y/a.wav"), ["y/a.wav", "a.wav"]);
        assert_eq!(tails_of("a/./b.wav"), ["b.wav"]);
        assert!(tails_of("a/..").is_empty());
    }
}
