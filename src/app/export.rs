//! The exports in the app ([`crate::project_files::export`]): an export that goes ahead closes the Save dialog,
//! and an audio one renders behind the busy screen (`Overlay::Busy`), one slice a frame before the screens, with the
//! transport stopped.

use eframe::egui;

use super::HoneydewApp;
use crate::displayer::mp4::Video;
use crate::project_files::export::{Done, Job, NO_AUDIO_RATE, Split, slice, tags};
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
    pub(super) fn export_slice(&mut self, ctx: &egui::Context) {
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
            match done {
                None => return self.navigate(Nav::Open(Overlay::Busy { title })),
                Some(Done::Stem(name, bytes)) => {
                    self.files.write_stem(&export, &name, bytes);
                    return self.navigate(Nav::Open(Overlay::Busy { title }));
                }
                Some(Done::File(bytes)) => self.files.write_export(&export, bytes),
                Some(Done::Stems) => self.files.stems_rendered(),
                Some(Done::Written) => self.files.video_written(&export),
                Some(Done::Rendered(song, stems)) => {
                    self.files.flp_rendered(&song, &export, &stems)
                }
                Some(Done::Failed(error)) => self.files.render_failed(&error),
            }
        }
        self.export = None;
        self.nav
            .overlays
            .retain(|o| !matches!(o, Overlay::Busy { .. }));
    }
}
