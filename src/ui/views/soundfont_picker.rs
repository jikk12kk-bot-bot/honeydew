//! The SoundFont picker overlay: the SoundFont Player's `Patch` spinner (or, in the error state, a tap on its panel)
//! opens it over `My SoundFonts` (and imported content's `SoundFonts`, when there), starting in the patches of the
//! module's SoundFont when the rack read them, its current patch current. A patch pick sets the module's `SF2D` (the
//! SoundFont's shorthand path and the patch index) at once, as one final song edit: a new engine, and no undo step (the
//! rack is not covered by undo); the spinner shows it and the picker stays open. The patches' `Sort` is stored as
//! `SoundFontPatchOrdering` at once. A SoundFont that cannot be read loads too, at patch 0, the picker closes and the
//! panel shows its error. Playing it is the engine's.
//!
//! Issue #120: a patch pick plays a short note of it on the channel, [`KEY_NOTE`] as a live note at
//! velocity 1, from the frame after the pick (once its engine is in) for [`PREVIEW_S`]; the next pick or the picker
//! closing stops it first. The app's keyboard stays in front of the picker, which ends above it, playing the
//! channel. Its files keep where they were left, as the other pickers do (#131,
//! [`crate::ui::preset_picker::PresetPicker::keep_place`]).

use eframe::egui;
use honeydew_audio::device::AudioCommand;

use super::{AppCtx, Nav, rack};
use crate::audio_link::AudioLink;
use crate::flm::sf2d::Sf2d;
use crate::panels::inst::soundfont_player;
use crate::project_files::{SOUNDFONT_EXTS, SOUNDFONTS, SOUNDFONTS_FOLDER};
use crate::ui::picker_preview::{KEY_NOTE, Sound};
use crate::ui::soundfont_picker::{SoundFontAction, SoundFontPicker, TITLE};

/// How long a patch pick's note sounds at most, in seconds.
pub const PREVIEW_S: f64 = 1.5;

/// The open picker. It starts afresh on each opening, however it was last closed.
#[derive(Default)]
pub struct State {
    picker: Option<SoundFontPicker>,
    /// The egui pass it was last drawn in: a gap means it was closed in between.
    pass: u64,
    /// The channel it plays: the patch's note, the keys.
    channel: usize,
    /// The last pick's note.
    preview: Option<Preview>,
}

/// A patch pick's note: due (it sounds the frame after the pick, in the pick's engine), or sounding until an egui time.
#[derive(Clone, Copy)]
enum Preview {
    Due,
    Until(f64),
}

/// Draws the picker for the SoundFont Player `module` on `channel`, modal, over the left of the screen, and loads its
/// pick, telling `rack` the patches it read so the panel needs no second read. The X, a click outside, Escape, a
/// SoundFont that fails, or the module being gone close it.
pub fn show(
    state: &mut State,
    rack: &mut rack::State,
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    channel: usize,
    module: i32,
) -> Option<Nav> {
    let player = app.song.find_module(channel, module);
    let Some(player) = player.filter(|m| m.type_id == soundfont_player::TYPE_ID) else {
        return Some(Nav::Close);
    };
    let pass = ui.ctx().cumulative_pass_nr();
    if state.pass + 1 != pass {
        state.picker = None;
    }
    state.pass = pass;
    state.channel = channel;
    // The last frame's pick sounds now, in its engine; it stops by itself.
    let now = ui.input(|i| i.time);
    match state.preview {
        Some(Preview::Due) => {
            send(app.audio, channel, [Sound::On(KEY_NOTE)]);
            state.preview = Some(Preview::Until(now + PREVIEW_S));
            ui.ctx().request_repaint_after_secs(PREVIEW_S as f32);
        }
        Some(Preview::Until(end)) if now >= end => {
            send(app.audio, channel, [Sound::Off(KEY_NOTE)]);
            state.preview = None;
        }
        Some(Preview::Until(end)) => ui.ctx().request_repaint_after_secs((end - now) as f32),
        None => {}
    }
    let files = &*app.files;
    let library = files.preset_library(&SOUNDFONT_EXTS);
    let picker = state.picker.get_or_insert_with(|| {
        // The module's SoundFont, as the rack read it (it drew the card that opened this), at the path the library
        // lists it under.
        let sf2d = player.file.custom_data.as_deref().and_then(Sf2d::read);
        let current = sf2d.and_then(|s| {
            let patches = rack.soundfont(&s.path)?.as_ref().ok()?.clone();
            // An index past either end is the patch at that end, as the spinner reads it.
            let patch = usize::try_from(s.patch)
                .unwrap_or(0)
                .min(patches.len().saturating_sub(1));
            Some((files.library_path(&s.path)?, patches, patch))
        });
        // `My Files` is the whole user folder, `My SoundFonts` its shortcut; each player keeps its own place.
        let user = files.user_dir().map(std::path::Path::to_owned);
        let mut picker = SoundFontPicker::new(user, current)
            .with_shortcuts([SOUNDFONTS_FOLDER])
            .with_factory(files.content_folder(SOUNDFONTS));
        picker.sort = app.settings.patch_ordering();
        let channel_id = app.song.channels.get(channel).map(|c| c.id);
        let id = egui::Id::new(TITLE)
            .with("place")
            .with((channel_id, module));
        picker.keep_place(ui.ctx(), &*library, id);
        picker
    });
    let action = picker.show(ui.ctx(), &app.cx, &*library);
    drop(library);
    send(app.audio, channel, picker.take_sounds());
    // A pick from `Sort`'s menu is saved as `SoundFontPatchOrdering`, the active one too.
    if let Some(sort) = picker.sort_picked.take() {
        app.settings.sound_font_patch_ordering = sort.id();
    }
    let (path, patch, read) = match action? {
        SoundFontAction::Close => {
            stop(state, app.audio);
            state.picker = None;
            return Some(Nav::Close);
        }
        SoundFontAction::Patch {
            path,
            patches,
            index,
        } => (path, index, Ok(patches)),
        SoundFontAction::Failed { path, error } => {
            stop(state, app.audio);
            state.picker = None;
            (path, 0, Err(error))
        }
    };
    let stays = read.is_ok();
    // The picker lists the user and content folders only, so a pick always has a shorthand; one without loads nothing.
    let Some(shorthand) = app.files.shorthand(&path) else {
        stop(state, app.audio);
        state.picker = None;
        return Some(Nav::Close);
    };
    if stays {
        // The note still sounding stops in the engine it plays in; the pick's comes next frame.
        if let Some(Preview::Until(_)) = state.preview {
            send(app.audio, channel, [Sound::Off(KEY_NOTE)]);
        }
        state.preview = Some(Preview::Due);
    }
    rack.soundfont_read(shorthand.clone(), read);
    let sf2d = Sf2d {
        path: shorthand,
        patch: patch as i32,
    };
    // A pick tries the SoundFonts and samples missing before again (as a sample pick does): re-picking the same file
    // and patch rebuilds when the engine had no SoundFont there.
    let missing = app.audio.samples.soundfont(&sf2d.path).is_none();
    app.audio.samples.forget_missing();
    let player = (app.song.channels.get_mut(channel)).and_then(|c| c.rack.get_mut(module));
    if let Some(player) = player
        && let Ok(blob) = sf2d.write(player.file.custom_data.as_deref())
    {
        let changed = player.file.custom_data.as_ref() != Some(&blob);
        player.file.custom_data = Some(blob);
        if changed || missing {
            app.song_edited(true);
        }
    }
    (!stays).then_some(Nav::Close)
}

/// The picker's sounds stop: the pick's note, the keys held. [`super::Slots::overlays`] calls it each frame the picker
/// is not open.
pub(super) fn stop(state: &mut State, audio: &mut AudioLink) {
    let mut sounds = Vec::new();
    if let Some(picker) = &mut state.picker {
        picker.release();
        sounds = picker.take_sounds();
    }
    if let Some(Preview::Until(_)) = state.preview.take() {
        sounds.push(Sound::Off(KEY_NOTE));
    }
    send(audio, state.channel, sounds);
}

/// Sends `sounds` to `channel` as live notes: velocity 1, pan centre.
fn send(audio: &mut AudioLink, channel: usize, sounds: impl IntoIterator<Item = Sound>) {
    for sound in sounds {
        audio.send(match sound {
            Sound::On(key) => AudioCommand::NoteOn {
                channel,
                key,
                velocity: 1.0,
            },
            Sound::Off(key) => AudioCommand::NoteOff { channel, key },
        });
    }
}
