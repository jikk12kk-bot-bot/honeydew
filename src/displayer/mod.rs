//! The Displayer (our own addition): an image or a video shown with the song, in a floating window or behind the
//! views, and rendered into an MP4 with the song's sound.
//!
//! It belongs to no song: its settings are the app's ([`DisplayerSettings`], in [`crate::settings::Settings`]), and
//! its file stays where it is: only its path (on Android its document URI) is kept. A video always starts with the
//! song (song second 0 is the trim's start), runs in seconds whatever the tempo, repeats its trimmed part while
//! `Loop` is on and is black after it otherwise. Its sound is listen-only ([`honeydew_audio::listen`]): heard after
//! the master and never in an audio render.
//!
//! Video needs the `video` feature (FFmpeg); without it, as on the web, the Displayer shows images only.

use serde::{Deserialize, Serialize};

pub mod media;
pub mod mp4;

/// What the Displayer shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    #[default]
    None,
    Image,
    Video,
}

/// Where it shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Display {
    /// Its own window over the views: Mini, which a drag moves and its corner resizes, or Maxi, over the whole area
    /// and letting taps through to the views.
    #[default]
    Window,
    /// Behind the views: under the notes, the clips and the controls, which the views' backdrops let through.
    Background,
    Hidden,
}

/// How the picture fills its room.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fit {
    /// All of it, bars beside or above it.
    #[default]
    Fit,
    /// The room filled, its edges cut.
    Fill,
    /// Stretched to the room.
    Stretch,
}

/// The MP4 render's frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Aspect {
    /// 16:9.
    #[default]
    Wide,
    /// 9:16.
    Tall,
    /// 1:1.
    Square,
}

/// The MP4 render's options (the Save dialog's MP4 page), kept with the app's settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mp4Options {
    pub aspect: Aspect,
    /// The short side: 720 or 1080.
    pub height: u32,
    /// 30 or 60.
    pub fps: u32,
    pub fit: Fit,
    /// The video's own sound in the file too (off: the song's alone).
    pub video_sound: bool,
}

impl Default for Mp4Options {
    fn default() -> Self {
        Self {
            aspect: Aspect::Wide,
            height: 720,
            fps: 30,
            fit: Fit::Fit,
            video_sound: false,
        }
    }
}

impl Mp4Options {
    /// The frame's width and height.
    pub fn size(&self) -> (u32, u32) {
        let short = if self.height >= 1080 { 1080 } else { 720 };
        let long = short * 16 / 9;
        match self.aspect {
            Aspect::Wide => (long, short),
            Aspect::Tall => (short, long),
            Aspect::Square => (short, short),
        }
    }

    /// 30 or 60.
    pub fn fps(&self) -> u32 {
        if self.fps >= 60 { 60 } else { 30 }
    }
}

/// The Displayer's settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayerSettings {
    pub source: Source,
    /// The image's and the video's files: a path, or on Android a document URI. Each is kept while the other shows.
    pub image: String,
    pub video: String,
    /// The video's trim, in seconds from its start; `end` 0 is its end.
    pub start: f64,
    pub end: f64,
    #[serde(rename = "loop")]
    pub looped: bool,
    pub muted: bool,
    /// The video's sound level, 0 to 1.
    pub volume: f32,
    pub display: Display,
    /// The window's opacity and the background's, 0.05 to 1.
    pub window_opacity: f32,
    pub background_opacity: f32,
    pub fit: Fit,
    /// The window's place and size, as shares of the work area (left, top, width, height); `None` before any.
    pub window: Option<[f32; 4]>,
    /// The window fills the work area (Maxi) rather than its own place (Mini).
    pub maxi: bool,
    pub mp4: Mp4Options,
}

impl Default for DisplayerSettings {
    fn default() -> Self {
        Self {
            source: Source::None,
            image: String::new(),
            video: String::new(),
            start: 0.0,
            end: 0.0,
            looped: true,
            muted: false,
            volume: 0.8,
            display: Display::Window,
            window_opacity: 1.0,
            background_opacity: 1.0,
            fit: Fit::Fit,
            window: None,
            maxi: false,
            mp4: Mp4Options::default(),
        }
    }
}

impl DisplayerSettings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The file the source shows, if any.
    pub fn file(&self) -> Option<&str> {
        let file = match self.source {
            Source::None => return None,
            Source::Image => &self.image,
            Source::Video => &self.video,
        };
        (!file.is_empty()).then_some(file.as_str())
    }

    /// The trim for a video `duration` seconds long (0: unknown).
    pub fn trim(&self, duration: f64) -> Trim {
        let length = if duration > 0.0 {
            duration
        } else {
            f64::INFINITY
        };
        let start = self.start.clamp(
            0.0,
            if length.is_finite() {
                (length - 0.05).max(0.0)
            } else {
                f64::MAX
            },
        );
        let end = if self.end > start && self.end < length {
            self.end
        } else {
            length
        };
        Trim {
            start,
            end,
            looped: self.looped,
        }
    }

    /// Values out of range (a hand-edited file) back in.
    pub fn sanitize(&mut self) {
        let d = Self::default();
        if !(self.start.is_finite() && self.start >= 0.0) {
            self.start = 0.0;
        }
        if !(self.end.is_finite() && self.end >= 0.0) {
            self.end = 0.0;
        }
        if !(0.0..=1.0).contains(&self.volume) {
            self.volume = d.volume;
        }
        if !(0.05..=1.0).contains(&self.window_opacity) {
            self.window_opacity = d.window_opacity;
        }
        if !(0.05..=1.0).contains(&self.background_opacity) {
            self.background_opacity = d.background_opacity;
        }
        if let Some(w) = self.window
            && !(w.iter().all(|v| v.is_finite()) && w[2] > 0.05 && w[3] > 0.05)
        {
            self.window = None;
        }
    }
}

/// A video's trimmed part and whether it repeats.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trim {
    pub start: f64,
    /// Infinite while the video's length is unknown.
    pub end: f64,
    pub looped: bool,
}

impl Default for Trim {
    fn default() -> Self {
        Self {
            start: 0.0,
            end: f64::INFINITY,
            looped: true,
        }
    }
}

impl Trim {
    /// The video second shown at song second `secs` (the song's start is the trim's start); `None` past the trimmed
    /// part when it does not repeat: black.
    pub fn at(&self, secs: f64) -> Option<f64> {
        let secs = secs.max(0.0);
        let len = self.end - self.start;
        if !len.is_finite() {
            return Some(self.start + secs);
        }
        if len <= 0.0 {
            return Some(self.start);
        }
        if secs < len {
            Some(self.start + secs)
        } else if self.looped {
            Some(self.start + secs % len)
        } else {
            None
        }
    }
}

/// Song tick `tick` in seconds from the song's start at `bpm`.
pub fn song_secs(tick: u64, bpm: f64) -> f64 {
    tick as f64 / f64::from(crate::song::PPQ) * 60.0 / bpm.max(1.0)
}

/// Where a picture `size` goes in `room` by `fit`: the rect it is drawn into (which a Fill overflows; clip to the
/// room).
pub fn placed(size: (f32, f32), room: eframe::egui::Rect, fit: Fit) -> eframe::egui::Rect {
    let (w, h) = (size.0.max(1.0), size.1.max(1.0));
    let k = match fit {
        Fit::Stretch => return room,
        Fit::Fit => (room.width() / w).min(room.height() / h),
        Fit::Fill => (room.width() / w).max(room.height() / h),
    };
    eframe::egui::Rect::from_center_size(room.center(), eframe::egui::vec2(w * k, h * k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trim_starts_with_the_song_and_loops_or_goes_black() {
        let looped = Trim {
            start: 2.0,
            end: 6.0,
            looped: true,
        };
        assert_eq!(looped.at(0.0), Some(2.0));
        assert_eq!(looped.at(3.0), Some(5.0));
        assert_eq!(looped.at(5.0), Some(3.0));
        let once = Trim {
            looped: false,
            ..looped
        };
        assert_eq!(once.at(3.5), Some(5.5));
        assert_eq!(once.at(4.0), None);
    }

    #[test]
    fn the_mp4_sizes() {
        let o = |aspect, height| {
            Mp4Options {
                aspect,
                height,
                ..Mp4Options::default()
            }
            .size()
        };
        assert_eq!(o(Aspect::Wide, 720), (1280, 720));
        assert_eq!(o(Aspect::Wide, 1080), (1920, 1080));
        assert_eq!(o(Aspect::Tall, 1080), (1080, 1920));
        assert_eq!(o(Aspect::Square, 720), (720, 720));
    }
}
