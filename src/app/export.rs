//! The exports in the app ([`crate::project_files::export`]): an export that goes ahead closes the Save dialog,
//! and an audio one renders behind the busy screen (`Overlay::Busy`), one slice a frame before the screens, with the
//! transport stopped.

use eframe::egui;

use super::HoneydewApp;
use crate::displayer::mp4::Video;
use crate::project_files::export::{Done, Export, Job, NO_AUDIO_RATE, Split, slice, tags};
#[cfg(target_os = "android")]
use std::sync::Arc;
#[cfg(target_os = "android")]
use std::sync::atomic::{AtomicBool, Ordering};
use crate::render::RenderError;
use crate::sample_cache;
use crate::ui::save_dialog::{ExportFormat, RenderPage};
use crate::ui::views::{Nav, Overlay};

fn dialog(o: &Overlay) -> bool {
    matches!(o, Overlay::SaveDialog { .. })
}

impl HoneydewApp {
    /// After the files, each frame: a closed Save dialog lets go of its state. An export that went ahead closes the
    /// dialog; an audio one starts rendering with the page's options, the samples loaded as a rebuild loads them, at
    /// the project's sample rate where the song has one (PR #87), else the engine rate (no resampling; 44 100 Hz without
    /// an output), tagged with the song's metadata while `Write tags on export` is on (#139).
    pub(super) fn export_started(&mut self, ctx: &egui::Context) {
        let Some(export) = self.files.take_export() else {
            if !self.nav.overlays.iter().any(dialog) {
                self.slots.export.dialog = None;
            }
            return;
        };
        self.nav.overlays.retain(|o| !dialog(o));
        self.slots.export.dialog = None;
        ctx.request_repaint();
        if !export.format.is_rendered() && export.split != Split::Flp {
            return;
        }
        self.audio
            .samples
            .load(&self.song, &sample_cache::reader(&self.files));
        // Settings' `Sample rate:`, as playback's (songs do not keep one).
        let rate = match crate::audio_link::rate() {
            0 => self.audio.sample_rate().unwrap_or(NO_AUDIO_RATE),
            rate => rate,
        };
        let video = export.format == ExportFormat::Mp4;
        let page = match video {
            // The whole song, unprocessed but for the remainder's fade, in float.
            true => RenderPage {
                high_quality: true,
                ..RenderPage::default()
            },
            false => self.slots.export.page,
        };
        let tags = match self.settings.write_tags && !video {
            true => tags(&self.song),
            false => Vec::new(),
        };
        let path = export.path.clone();
        let job = Job::new(&self.song, &self.audio.samples, export, page, rate);
        let job = match (job, video) {
            (Ok(job), true) => match Video::new(&path, &self.settings.displayer, rate) {
                Ok(v) => Ok(job.with_video(v)),
                Err(e) => Err(RenderError::Video(e)),
            },
            (job, _) => job,
        };
        match job {
            Ok(job) => {
                // The busy screen hides the editors, which will not see their sounds end: every note the keys,
                // the pads and the shown roll sound stops now, as when the app is hidden.
                self.stop_edit_sounds();
                self.navigate(Nav::Open(Overlay::Busy { title: job.title() }));
                self.export = Some(job.with_tags(tags));
            }
            Err(error) => self.files.render_failed(&error),
        }
    }

    /// Before the screens: a slice of the render under way, its stage's title on the busy screen (a stem's under
    /// `Exporting channel %d: %s`), and the transport stopped (its Stop goes out after the screens, as the play
    /// button's does). Each stem is written once encoded, and the file once the job is over; an error shows its message.
    /// On Android, an export that rendered on a thread while the app was away comes back first ([`Self::export_back`]).
    pub(super) fn export_slice(&mut self, ctx: &egui::Context) {
        #[cfg(target_os = "android")]
        if self.export_back(ctx) {
            return;
        }
        let Some(job) = &mut self.export else {
            return;
        };
        self.main_screen.playing = false;
        ctx.request_repaint();
        // An MP4's Cancel: the job goes, and its unfinished file with it.
        if job.is_video() {
            crate::ui::views::busy_abortable(ctx);
            if crate::ui::views::take_busy_aborted(ctx) {
                self.export = None;
                self.nav
                    .overlays
                    .retain(|o| !matches!(o, Overlay::Busy { .. }));
                return;
            }
        }
        // A stem that could not be written ends the export; its message is up.
        if !self.files.stem_write_failed() {
            let done = job.step(&mut slice());
            let (export, title) = (job.export().clone(), job.title());
            // The bar under the title: how far the render or the encoding is.
            crate::ui::views::busy_progress(ctx, job.progress());
            if !self.export_done(done, &export, title) {
                return;
            }
        }
        self.export = None;
        self.nav
            .overlays
            .retain(|o| !matches!(o, Overlay::Busy { .. }));
    }

    /// What a step of `export` ended with: the busy screen's `title` while it goes on (`false`), a stem written, or
    /// the job over (`true`): its file written, or its error's message.
    fn export_done(&mut self, done: Option<Done>, export: &Export, title: String) -> bool {
        match done {
            None => {
                self.navigate(Nav::Open(Overlay::Busy { title }));
                return false;
            }
            Some(Done::Stem(name, bytes)) => {
                self.files.write_stem(export, &name, bytes);
                self.navigate(Nav::Open(Overlay::Busy { title }));
                return false;
            }
            Some(Done::File(bytes)) => self.files.write_export(export, bytes),
            Some(Done::Stems) => self.files.stems_rendered(),
            Some(Done::Written) => self.files.video_written(export),
            Some(Done::Rendered(song, stems)) => self.files.flp_rendered(&song, export, &stems),
            Some(Done::Failed(error)) => self.files.render_failed(&error),
        }
        true
    }

    /// Android's `onPause`: the export under way renders on a thread of its own while the app is away, since Android
    /// runs no frame for an app in the background (the foreground service keeps the process running,
    /// `HoneydewActivity.onPause`). It comes back when the app does ([`Self::export_back`]).
    #[cfg(target_os = "android")]
    pub(super) fn export_leave(&mut self) {
        let Some(job) = self.export.take() else {
            return;
        };
        // An `onResume` from before (the app's start) is not this return.
        crate::android::take_resumed();
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, job) = (stop.clone(), Moved(job));
        let thread = std::thread::spawn(move || {
            let mut job = job.into_inner();
            let mut dones = Vec::new();
            while !flag.load(Ordering::Acquire) {
                match job.step(&mut slice()) {
                    None => {}
                    Some(stem @ Done::Stem(..)) => dones.push(stem),
                    Some(last) => {
                        dones.push(last);
                        break;
                    }
                }
            }
            Moved((job, dones))
        });
        self.export_away = Some(Away { stop, thread });
    }

    /// The export that rendered away ([`Self::export_leave`]) comes back once the app is on the screen again, or
    /// once it is over: what it ended with is handled as a frame's step would have. `true` while it is still away.
    #[cfg(target_os = "android")]
    fn export_back(&mut self, ctx: &egui::Context) -> bool {
        let Some(away) = &self.export_away else {
            return false;
        };
        if !away.thread.is_finished() && !crate::android::take_resumed() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
            return true;
        }
        let Some(away) = self.export_away.take() else {
            return false;
        };
        away.stop.store(true, Ordering::Release);
        let Ok(Moved((job, dones))) = away.thread.join() else {
            // The thread panicked: the export is lost.
            self.nav
                .overlays
                .retain(|o| !matches!(o, Overlay::Busy { .. }));
            return false;
        };
        let (export, title) = (job.export().clone(), job.title());
        self.export = Some(job);
        for done in dones {
            if self.export_done(Some(done), &export, title.clone()) {
                self.export = None;
                self.nav
                    .overlays
                    .retain(|o| !matches!(o, Overlay::Busy { .. }));
                break;
            }
        }
        ctx.request_repaint();
        false
    }
}

/// An export rendering on a thread while the app is away ([`HoneydewApp::export_leave`]): its stop flag, and the
/// thread, which hands back the job and what its steps ended with.
#[cfg(target_os = "android")]
pub(super) struct Away {
    stop: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<Moved<(Job, Vec<Done>)>>,
}

/// A value moved whole to another thread and back.
#[cfg(target_os = "android")]
pub(super) struct Moved<T>(T);

#[cfg(target_os = "android")]
impl<T> Moved<T> {
    /// The value; taking `self` whole makes a closure capture the wrapper, not its field.
    fn into_inner(self) -> T {
        self.0
    }
}

// SAFETY: an export job (and what its steps hand back) only ever has one thread using it: the app's frames, or the
// export thread while the app is away, handed over by the move and the join. Its parts (the engine, the encoders,
// FFmpeg's contexts) share nothing with the app's and are tied to no thread.
#[cfg(target_os = "android")]
unsafe impl<T> Send for Moved<T> {}
