//! Third-party plugins in the app (desktop; `crate::plugins`): the scan's start and results, the instances kept in
//! step with the song every frame, the param changes a plugin's own editor makes, and what went wrong.
//!
//! A plugin's editor moving one of its params is a hand move on the module: the song value, a `SetParam` for the
//! engine's saved value and the song-changed flag ([`SongEdit::ChannelParam`], as the rack's controls), and it is the
//! recent control, so REC records it and the CTRL menu automates it. Unlike the rack's controls, each edit in the
//! plugin's window is an undo step once it ends (its gesture's release, else the frame it moved in): so is a built-in
//! plugin's edit of its saved state with no param (its `Edited`), which also marks the song changed
//! ([`History::plugin_edited`]). A built-in plugin's knob menu's `Create automation clip` adds its automation track
//! (#113).
//!
//! [`History::plugin_edited`]: crate::history::History::plugin_edited

use eframe::egui;
use honeydew_audio::device::AudioCommand;

use super::HoneydewApp;
use crate::history::PluginStep;
use crate::plugins;
use crate::song::{AutomationTarget, ModuleInstance};
use crate::ui::views::{Nav, Overlay, SongEdit};

/// The message tag of the plugins' notices.
const NOTICE_TAG: &str = "plugin notice";

impl HoneydewApp {
    /// An edit in the window of plugin module `instance_id` of channel `channel_id` ended, `before` the module as it
    /// found it: an undo step, which sets the song-changed flag ([`History::plugin_edited`]).
    ///
    /// [`History::plugin_edited`]: crate::history::History::plugin_edited
    fn plugin_step(&mut self, key: (i32, i32), before: &ModuleInstance, state: bool, now: f64) {
        let step = PluginStep {
            channel_id: key.0,
            instance_id: key.1,
            state,
        };
        if self.history.plugin_edited(&self.song, step, before) {
            self.plugin_stepped = Some((key, now));
            self.count_time();
            self.files.song_changed();
        }
    }

    /// At the start: the cached scan, and a background scan of new and changed plugin files.
    pub(super) fn start_plugins(&mut self, ctx: &egui::Context) {
        let cache = self
            .storage
            .as_deref()
            .and_then(|s| s.get_string(plugins::CACHE_KEY));
        plugins::start(ctx, cache, &self.settings.plugin_folders);
    }

    /// Every frame, early: the scan's result kept, the instances matched to the song, the plugins' own param changes
    /// applied, and their notices shown once no other message is.
    pub(super) fn plugins_frame(&mut self, ctx: &egui::Context, tick: u64) {
        if plugins::poll_scan()
            && let Some(storage) = self.storage.as_deref_mut()
        {
            storage.set_string(plugins::CACHE_KEY, plugins::cache_json());
            self.settings_unflushed = true;
        }
        plugins::sync(&self.song);
        if plugins::adopt_params(&mut self.song) {
            self.song_edited(SongEdit::RackRebuild, tick);
        }
        let mut moved = false;
        for m in plugins::idle() {
            let Some(channel) = self.song.channels.iter().position(|c| c.id == m.channel_id) else {
                continue;
            };
            let target = AutomationTarget {
                instance_id: m.instance_id,
                param: m.param,
            };
            self.recent = Some((m.channel_id, target));
            // The module as the edit found it, kept until the edit ends.
            let key = (m.channel_id, m.instance_id);
            if !self.plugin_edits.contains_key(&key)
                && let Some(module) = self.song.channels[channel].rack.get(m.instance_id)
            {
                self.plugin_edits.insert(key, (module.clone(), 0));
            }
            if let Some((_, open)) = self.plugin_edits.get_mut(&key) {
                match m.gesture {
                    0 => *open += 1,
                    2 => *open = open.saturating_sub(1),
                    _ => {}
                }
            }
            let Some(value) = self.song.channels[channel]
                .param_mut(target)
                .filter(|_| m.gesture == 1)
            else {
                continue;
            };
            *value = m.value.clamp(0.0, 1.0);
            self.audio.send(AudioCommand::SetParam {
                channel,
                instance_id: m.instance_id,
                index: m.param,
                value: m.value.clamp(0.0, 1.0),
            });
            moved = true;
        }
        if moved {
            self.song_edited(SongEdit::ChannelParam, tick);
        }
        let now = ctx.input(|i| i.time);
        // A built-in plugin's edit of its saved state: the module takes it, an undo step. A param edit of it waiting
        // for its end is part of it; one the step just before was (a knob's drag tells both, its `Edited` a tick
        // later) takes it instead.
        for (channel_id, instance_id, before) in plugins::take_edited(&mut self.song) {
            let key = (channel_id, instance_id);
            let pending = self.plugin_edits.remove(&key).map(|(b, _)| b);
            let recent = self
                .plugin_stepped
                .is_some_and(|(k, t)| k == key && now - t < 1.0);
            if pending.is_none() && recent && self.history.plugin_amend(&self.song, key) {
                continue;
            }
            self.plugin_step(key, &pending.unwrap_or(before), true, now);
        }
        // Each edit in a plugin's window that ended (no gesture of it open, no pointer down on its card): an undo step.
        if !ctx.input(|i| i.pointer.any_down()) {
            let ended: Vec<_> = (self.plugin_edits.iter())
                .filter(|(_, (_, open))| *open == 0)
                .map(|(&key, _)| key)
                .collect();
            for key in ended {
                let (before, _) = self.plugin_edits.remove(&key).expect("listed");
                self.plugin_step(key, &before, false, now);
            }
        }
        // Issue #113: a built-in plugin's knob menu's `Create automation clip` is `Add automation track` for its param.
        for (channel_id, instance_id, param) in plugins::take_automate() {
            if let Some(channel) = self.song.channels.iter().position(|c| c.id == channel_id) {
                let target = AutomationTarget { instance_id, param };
                self.recent = Some((channel_id, target));
                self.add_automation_track(channel, target, tick);
            }
        }
        let message = |o: &Overlay| matches!(o, Overlay::Message { .. });
        if !self.nav.overlays.iter().any(message) {
            let lines = plugins::take_notices();
            if !lines.is_empty() {
                self.navigate(Nav::Open(Overlay::Message {
                    tag: NOTICE_TAG,
                    lines,
                    confirm: None,
                    extra: None,
                }));
            }
        }
        if let Some(after) = plugins::idle_after() {
            ctx.request_repaint_after(after);
        }
        // What a built-in plugin's editor saved where there is no save dialog (Tesla's `Save`, on Android and the web).
        for (path, bytes) in os_plugkit::embed::take_saved() {
            self.files.keep_saved(&path, bytes);
        }
    }
}
