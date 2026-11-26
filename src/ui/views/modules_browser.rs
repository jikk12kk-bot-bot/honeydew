//! The Modules browser, adding to a channel's rack.

use eframe::egui;

use super::{AppCtx, Nav, Overlay, preset_picker};
use crate::plugins;
use crate::song::ModuleInstance;
use crate::ui::modules_browser::{
    BrowserAction, InsertAt, Marks, ModulesBrowser, add_module, add_plugin,
};
use crate::ui::rack::{Held, Outcome, PresetTarget, hold_removal, paste, takes};

/// The flash of a `Paste` with nothing on the clipboard that the channel takes.
pub const CANNOT_PASTE: &str = "Cannot paste module";

/// The open browser. It starts afresh on each opening, however it was last closed: the slide-in, an empty search; but
/// its list opens scrolled where the last browse left it, on any channel ([`State::scroll`]).
#[derive(Default)]
pub struct State {
    browser: ModulesBrowser,
    /// The egui pass it was last drawn in: a gap means it was closed in between.
    pass: u64,
    /// A `Replace` of a module automation lanes target, held back ([`hold_removal`]): the app asks `Delete automation?`
    /// first.
    pub(crate) ask_remove: Option<Held>,
    /// How far the list was scrolled when last drawn without a search, in points: the next opening (from any channel's
    /// rack, or back from `Rack Preset` / `Instrument`) starts there.
    scroll: f32,
    /// This opening's scroll as first drawn (the stored one, kept within this channel's list), and whether it moved
    /// since: only a move is stored, so a shorter list (a drum channel's) does not pull a longer one's place up.
    first: Option<(f32, bool)>,
}

/// Draws the browser, modal, over the left of the screen. A pick adds the module to `channel`'s rack at `at` (an
/// instrument only where the channel takes one: the instruments are not listed elsewhere) with no undo step, and the
/// browser stays open: each later pick of the session (an opening starts one) replaces the module the last one added,
/// in its slot, so the whole browse adds one module, its lists in its place included (issue #123: through `picker`'s
/// `added`, which their Back keeps). `Paste` (always enabled) copies the `clipboard` modules that fit the channel
/// ([`paste`]), a final song edit, and closes it; a `Paste` of nothing that fits flashes [`CANNOT_PASTE`]
/// instead. A `Replace` of a module automation lanes target is held back for the app's question
/// ([`State::ask_remove`]), and closes it. `Rack Preset` and `Instrument` give way to their lists in the same panel
/// (`PresetTarget::ModulesRack`, `PresetTarget::ModulesInstrument`), whose Back at their root brings the browser back
/// in place, through `picker` ([`preset_picker::State::back_to_modules`]). The X, a click outside, Escape or the
/// channel being gone close it. It opens in the ordering the stored `ModuleBrowserOrdering` names, else Category, and
/// stores what its `Sort` menu picks there at once; PINNED and RECENT come from the settings, which take their changes
/// at once too. A DW Sampler picked loads Close Grand, through `picker`
/// ([`preset_picker::load_close_grand`]).
pub fn show(
    state: &mut State,
    picker: &mut preset_picker::State,
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    channel: usize,
    at: InsertAt,
    clipboard: &[ModuleInstance],
) -> Option<Nav> {
    let pass = ui.ctx().cumulative_pass_nr();
    if state.pass + 1 != pass {
        // Its ordering is the stored one, applied at each opening. Back from a list in its place: no slide-in.
        let back = std::mem::take(&mut picker.back_to_modules);
        state.browser = match back {
            true => ModulesBrowser::in_place(),
            false => ModulesBrowser::default(),
        };
        state.browser.sort = app.settings.module_ordering();
        state.browser.marks = Marks {
            pinned: app.settings.pinned_modules.clone(),
            recent: app.settings.recent_modules.clone(),
        };
        state.browser.set_scroll(state.scroll);
        state.first = None;
        // An opening starts a browse; Back from a list goes on with it.
        if !back {
            picker.added = None;
        }
    }
    state.pass = pass;
    let action = match app.song.channels.get(channel) {
        Some(ch) => {
            let action = state.browser.show(ui.ctx(), &app.cx, takes(ch));
            // Where it is scrolled, for the next opening: kept as it draws, so any close keeps it.
            if let Some(scroll) = state.browser.scroll() {
                let (first, moved) = state.first.get_or_insert((scroll, false));
                *moved |= scroll != *first;
                if *moved {
                    state.scroll = scroll;
                }
            }
            // A pick from `Sort`'s menu is saved as `ModuleBrowserOrdering`, the active one too; an opening saves
            // nothing.
            if let Some(sort) = state.browser.sort_picked.take() {
                app.settings.module_browser_ordering = sort.id();
            }
            // PINNED and RECENT are saved as they change.
            if std::mem::take(&mut state.browser.marks_changed) {
                let marks = &state.browser.marks;
                app.settings.pinned_modules.clone_from(&marks.pinned);
                app.settings.recent_modules.clone_from(&marks.recent);
            }
            action?
        }
        None => BrowserAction::Close,
    };
    let before = app.song.channels.get(channel).cloned();
    let edited = match (action, app.song.channels.get_mut(channel)) {
        (BrowserAction::Pick(type_id), Some(ch)) => {
            let at = picker.added.map_or(at, InsertAt::Replace);
            let id = add_module(&mut ch.rack, type_id, at);
            picker.added = Some(id);
            // Every new DW Sampler loads Close Grand.
            preset_picker::load_close_grand(picker, app.files, ch.id, id, type_id);
            true
        }
        (BrowserAction::Plugin(uid), Some(ch)) => {
            let at = picker.added.map_or(at, InsertAt::Replace);
            match add_plugin(&mut ch.rack, ch.id, uid, at) {
                Ok(id) => {
                    picker.added = Some(id);
                    // Its own editor opens at once if it has one; a failure shows on its card.
                    if let Some(plugins::State::Live(info)) = plugins::state(ch.id, id)
                        && info.has_editor
                    {
                        let _ = plugins::show_editor(ch.id, id, true);
                    }
                    true
                }
                Err(why) => {
                    app.files.flash(why);
                    false
                }
            }
        }
        (BrowserAction::Paste, Some(ch)) => {
            let pasted = paste(ch, clipboard, at) == Outcome::ChainChanged;
            if !pasted {
                app.files.flash(CANNOT_PASTE);
            }
            pasted
        }
        (BrowserAction::RackPreset, Some(_)) => {
            let target = PresetTarget::ModulesRack(at);
            return Some(Nav::Swap(Overlay::PresetPicker { channel, target }));
        }
        (BrowserAction::Instrument, Some(_)) => {
            let target = PresetTarget::ModulesInstrument(at);
            return Some(Nav::Swap(Overlay::PresetPicker { channel, target }));
        }
        _ => false,
    };
    if edited {
        let ch = &mut app.song.channels[channel];
        match before.and_then(|before| hold_removal(ch, before)) {
            Some(after) => {
                state.ask_remove = Some(Held {
                    channel,
                    after,
                    skipped: 0,
                });
                return Some(Nav::Close);
            }
            None if matches!(action, BrowserAction::Pick(_) | BrowserAction::Plugin(_)) => {
                // A pick marks the song changed, with no undo step.
                app.files.song_changed();
                app.pick_loaded();
                return None;
            }
            None => app.song_edited(true),
        }
    }
    Some(Nav::Close)
}
