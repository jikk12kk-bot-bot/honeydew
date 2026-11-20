//! The Displayer's media at run time: its file read where it is (a path, an Android document URI, the web's picked
//! bytes), the image as a texture, the video's player and its newest picture as a texture, and the file pickers.

use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, ColorImage, TextureHandle, TextureId, TextureOptions, Vec2};

use super::{DisplayerSettings, Source};

/// The longest side of an image texture: phones' GPUs take 4096.
const MAX_IMAGE_SIDE: u32 = 4096;

/// How the media is.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    /// Nothing to show (source None, or no file picked).
    #[default]
    Empty,
    Loading,
    Ready,
    /// The file is not where it was: the panel offers `Relink`.
    NotFound,
    Failed(String),
    /// A video in a build without video (the web).
    NoVideo,
}

/// A pick's result: the file's location, or why there is none (`None` when cancelled).
pub type Picked = Option<Result<String, String>>;

/// The media.
#[derive(Default)]
pub struct Media {
    /// What is loaded: the source and its file.
    loaded: Option<(Source, String)>,
    pub status: Status,
    image: Option<TextureHandle>,
    loading: Option<Receiver<Result<ColorImage, Status>>>,
    #[cfg(feature = "video")]
    player: Option<crate::video::player::Player>,
    video: Option<TextureHandle>,
    #[cfg(feature = "video")]
    serial: u64,
    black: bool,
    /// The video's length in seconds and size, once known.
    pub duration: f64,
    pub size: Option<(u32, u32)>,
    /// MediaCodec decodes the video.
    pub hardware: bool,
    picking: Option<(Source, Receiver<Picked>)>,
    /// The file's name to show, by location: an Android document's name is a query, not one for every frame.
    name: Option<(String, String)>,
}

impl Media {
    /// Loads what `s` shows when it changed, and moves the video to song second `song_secs` (`playing` or not, at
    /// `bpm`), or to video second `preview` while the panel scrubs a trim.
    pub fn sync(
        &mut self,
        ctx: &egui::Context,
        s: &DisplayerSettings,
        (song_secs, bpm): (f64, f64),
        playing: bool,
        preview: Option<f64>,
    ) {
        let want = s.file().map(|f| (s.source, f.to_owned()));
        if want != self.loaded {
            self.load(ctx, want);
        }
        if let Some(rx) = &self.loading
            && let Ok(result) = rx.try_recv()
        {
            self.loading = None;
            match result {
                Ok(image) => {
                    self.size = Some((image.size[0] as u32, image.size[1] as u32));
                    self.image =
                        Some(ctx.load_texture("displayer image", image, TextureOptions::LINEAR));
                    self.status = Status::Ready;
                }
                Err(status) => self.status = status,
            }
        }
        #[cfg(feature = "video")]
        self.sync_video(ctx, s, (song_secs, bpm), playing, preview);
        #[cfg(not(feature = "video"))]
        let _ = (song_secs, bpm, playing, preview);
    }

    #[cfg(feature = "video")]
    fn sync_video(
        &mut self,
        ctx: &egui::Context,
        s: &DisplayerSettings,
        (song_secs, bpm): (f64, f64),
        playing: bool,
        preview: Option<f64>,
    ) {
        use crate::video::player::{Status as PlayerStatus, Target};
        let Some(player) = &self.player else { return };
        match player.status() {
            PlayerStatus::Opening => {}
            PlayerStatus::Ready {
                duration,
                size,
                hardware,
            } => {
                (self.duration, self.size, self.hardware) = (duration, Some(size), hardware);
                if self.status == Status::Loading {
                    self.status = Status::Ready;
                }
            }
            PlayerStatus::NotFound => self.status = Status::NotFound,
            PlayerStatus::Failed(err) => self.status = Status::Failed(err),
        }
        let trim = s.trim(self.duration);
        let gain = if s.muted { 0.0 } else { s.volume };
        let target = Target {
            picture: preview.or_else(|| trim.at(song_secs)),
            song_secs,
            bpm,
            playing: playing && preview.is_none(),
            trim,
            gain,
        };
        player.set_target(target);
        let (video, serial, black) = (&mut self.video, &mut self.serial, &mut self.black);
        let seen = *serial;
        player.with_frame(seen, |frame| {
            *serial = frame.serial;
            *black = frame.black;
            if frame.black
                || frame.rgba.len() != (frame.width * frame.height * 4) as usize
                || frame.width == 0
            {
                return;
            }
            let size = [frame.width as usize, frame.height as usize];
            let image = ColorImage::from_rgba_unmultiplied(size, &frame.rgba);
            match video {
                Some(t) if t.size() == size => t.set(image, TextureOptions::LINEAR),
                _ => {
                    *video =
                        Some(ctx.load_texture("displayer video", image, TextureOptions::LINEAR))
                }
            }
        });
    }

    fn load(&mut self, ctx: &egui::Context, want: Option<(Source, String)>) {
        self.loaded = want.clone();
        self.image = None;
        self.video = None;
        self.loading = None;
        self.duration = 0.0;
        self.size = None;
        self.hardware = false;
        self.black = false;
        #[cfg(feature = "video")]
        {
            self.player = None;
        }
        let Some((source, file)) = want else {
            self.status = Status::Empty;
            return;
        };
        match source {
            Source::None => self.status = Status::Empty,
            Source::Image => {
                self.status = Status::Loading;
                let (tx, rx) = mpsc::channel();
                self.loading = Some(rx);
                let ctx = ctx.clone();
                let job = move || {
                    let result = decode_image(&file, MAX_IMAGE_SIDE).map(|img| {
                        let size = [img.width() as usize, img.height() as usize];
                        ColorImage::from_rgba_unmultiplied(size, img.as_raw())
                    });
                    let _ = tx.send(result);
                    ctx.request_repaint();
                };
                #[cfg(target_arch = "wasm32")]
                job();
                #[cfg(not(target_arch = "wasm32"))]
                std::thread::spawn(job);
            }
            Source::Video => {
                // The file opens on the player's thread: an Android document's descriptor is a JNI call.
                #[cfg(feature = "video")]
                {
                    self.status = Status::Loading;
                    self.player = Some(crate::video::player::Player::start(file, ctx.clone()));
                }
                #[cfg(not(feature = "video"))]
                {
                    let _ = file;
                    self.status = Status::NoVideo;
                }
            }
        }
    }

    /// Opens the file again (after a `Relink` that picked the same file, or a file that came back).
    pub fn reload(&mut self) {
        self.loaded = None;
    }

    /// The picture to draw and its size; `None` draws black (nothing loaded, the video not started or over).
    pub fn picture(&self) -> Option<(TextureId, Vec2)> {
        let t = match self.loaded.as_ref()?.0 {
            Source::Image => self.image.as_ref()?,
            Source::Video if !self.black => self.video.as_ref()?,
            _ => return None,
        };
        Some((t.id(), t.size_vec2()))
    }

    /// The video has played its trimmed part, unlooped: nothing to show until the song is back in it.
    pub fn over(&self) -> bool {
        let video = matches!(&self.loaded, Some((Source::Video, _)));
        video && self.black && self.status == Status::Ready
    }

    /// The name to show of the file at `location`, asked once per file.
    pub fn file_name(&mut self, location: &str) -> String {
        match &self.name {
            Some((at, name)) if at == location => name.clone(),
            _ => {
                let name = display_name(location);
                self.name = Some((location.to_owned(), name.clone()));
                name
            }
        }
    }

    /// Starts the system's file picker for `source`'s files; [`Self::take_pick`] gets the pick.
    pub fn pick(&mut self, source: Source, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel::<Picked>();
        self.picking = Some((source, rx));
        let ctx = ctx.clone();
        let done = move |picked: Picked| {
            let _ = tx.send(picked);
            ctx.request_repaint();
        };
        let video = source == Source::Video;
        #[cfg(not(any(target_arch = "wasm32", target_os = "android", target_os = "ios")))]
        std::thread::spawn(move || {
            let (name, exts): (&str, &[&str]) = if video {
                ("Videos", &["mp4", "m4v", "mov", "mkv", "webm"])
            } else {
                ("Images", &["png", "jpg", "jpeg", "webp"])
            };
            let picked = rfd::FileDialog::new().add_filter(name, exts).pick_file();
            done(picked.map(|p| Ok(p.to_string_lossy().into_owned())));
        });
        #[cfg(target_os = "android")]
        crate::android::pick_media(if video { "video/*" } else { "image/*" }, done);
        #[cfg(target_os = "ios")]
        crate::ios::pick(false, move |picked| {
            done(picked.map(|p| Ok(p.to_string_lossy().into_owned())))
        });
        #[cfg(target_arch = "wasm32")]
        web::pick(video, done);
    }

    /// The pick finished: its source and result.
    pub fn take_pick(&mut self) -> Option<(Source, Picked)> {
        let (source, rx) = self.picking.as_ref()?;
        let picked = rx.try_recv().ok()?;
        let source = *source;
        self.picking = None;
        Some((source, picked))
    }
}

/// The whole file at `location`; `Status::NotFound` when it is not there.
pub fn read_file(location: &str) -> Result<Vec<u8>, Status> {
    #[cfg(target_os = "android")]
    if location.starts_with("content://") {
        use std::io::Read;
        use std::os::fd::FromRawFd;
        let fd = crate::android::open_uri_fd(location).ok_or(Status::NotFound)?;
        // SAFETY: the descriptor is ours, detached from Java for us; the File closes it.
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|e| Status::Failed(e.to_string()))?;
        return Ok(bytes);
    }
    #[cfg(target_arch = "wasm32")]
    return web::bytes(location).ok_or(Status::NotFound);
    #[cfg(not(target_arch = "wasm32"))]
    std::fs::read(location).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Status::NotFound,
        _ => Status::Failed(e.to_string()),
    })
}

/// The image at `location`, decoded, its longest side at most `max_side`.
pub fn decode_image(location: &str, max_side: u32) -> Result<image::RgbaImage, Status> {
    let bytes = read_file(location)?;
    let img = image::load_from_memory(&bytes).map_err(|e| Status::Failed(e.to_string()))?;
    let img = if img.width().max(img.height()) > max_side {
        img.resize(max_side, max_side, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    Ok(img.to_rgba8())
}

/// The file's name to show.
pub fn display_name(location: &str) -> String {
    #[cfg(target_os = "android")]
    if location.starts_with("content://") {
        return crate::android::uri_name(location).unwrap_or_else(|| "Picked file".into());
    }
    if let Some(name) = location.strip_prefix(WEB_PREFIX) {
        return name.to_owned();
    }
    std::path::Path::new(location)
        .file_name()
        .map_or_else(|| location.to_owned(), |n| n.to_string_lossy().into_owned())
}

/// The web keeps a picked image's bytes for the session under this prefix and its name: there is no path to keep.
const WEB_PREFIX: &str = "web:";

#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use wasm_bindgen::JsCast;
    use web_sys::HtmlInputElement;

    use super::{Picked, WEB_PREFIX};

    thread_local! {
        static FILES: RefCell<HashMap<String, Vec<u8>>> = RefCell::new(HashMap::new());
    }

    pub fn bytes(location: &str) -> Option<Vec<u8>> {
        FILES.with(|f| f.borrow().get(location).cloned())
    }

    /// The browser's file input; a video says the web shows images only.
    pub fn pick(video: bool, done: impl FnOnce(Picked) + 'static) {
        if video {
            return done(Some(Err("The web version shows images only".into())));
        }
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return done(None);
        };
        let Ok(input) = document.create_element("input") else {
            return done(None);
        };
        let input: HtmlInputElement = input.unchecked_into();
        input.set_type("file");
        input.set_accept("image/png,image/jpeg,image/webp");
        let picked = input.clone();
        let on_change = wasm_bindgen::closure::Closure::once_into_js(move || {
            let Some(file) = picked.files().and_then(|files| files.get(0)) else {
                return;
            };
            wasm_bindgen_futures::spawn_local(async move {
                let result = match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                    Ok(buffer) => {
                        let key = format!("{WEB_PREFIX}{}", file.name());
                        let bytes = web_sys::js_sys::Uint8Array::new(&buffer).to_vec();
                        FILES.with(|f| f.borrow_mut().insert(key.clone(), bytes));
                        Some(Ok(key))
                    }
                    Err(_) => Some(Err("The file cannot be read".into())),
                };
                done(result);
            });
        });
        input.set_onchange(Some(on_change.unchecked_ref()));
        input.click();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_ready_video_on_black_is_over() {
        let mut media = Media {
            loaded: Some((Source::Video, "clip.mp4".into())),
            status: Status::Ready,
            black: true,
            ..Media::default()
        };
        assert!(media.over());
        media.black = false;
        assert!(!media.over(), "a picture shows");
        media.black = true;
        media.status = Status::Loading;
        assert!(!media.over(), "still opening");
        media.status = Status::Ready;
        media.loaded = Some((Source::Image, "photo.png".into()));
        assert!(!media.over(), "an image is never over");
        media.loaded = None;
        assert!(!media.over());
    }
}
