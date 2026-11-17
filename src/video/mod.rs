//! FFmpeg, through the small C surface of `shim.c` (the `video` feature): a decoder of one stream of a media file
//! (RGBA frames at a size we set, stereo float sound at the output's rate) and an MP4 encoder (H.264 + AAC). The
//! Displayer plays with [`player::Player`]; the MP4 render encodes with [`Encoder`].
//!
//! FFmpeg (LGPL-2.1-or-later, here built with x264 and so GPL-2.0-or-later) and x264 (GPL-2.0-or-later) are linked
//! statically from tools/build-ffmpeg.sh's build: see the README's `License and credits`.

use std::ffi::{CString, c_char, c_int};
use std::path::Path;
use std::ptr::NonNull;
use std::sync::{Arc, Once};

pub mod player;

#[repr(C)]
struct HdDecoder {
    _private: [u8; 0],
}

#[repr(C)]
struct HdEncoder {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn hd_init();
    fn hd_dec_open(
        path: *const c_char,
        fd: c_int,
        audio: c_int,
        hardware: c_int,
        out_rate: c_int,
        err: *mut c_char,
        err_len: c_int,
    ) -> *mut HdDecoder;
    fn hd_dec_info(
        d: *mut HdDecoder,
        duration: *mut f64,
        width: *mut c_int,
        height: *mut c_int,
        hardware: *mut c_int,
    );
    fn hd_dec_set_size(d: *mut HdDecoder, width: c_int, height: c_int);
    fn hd_dec_seek(d: *mut HdDecoder, secs: f64) -> c_int;
    fn hd_dec_video(
        d: *mut HdDecoder,
        skip: c_int,
        rgba: *mut *const u8,
        width: *mut c_int,
        height: *mut c_int,
        secs: *mut f64,
    ) -> c_int;
    fn hd_dec_audio(d: *mut HdDecoder, pcm: *mut *const f32, secs: *mut f64) -> c_int;
    fn hd_dec_close(d: *mut HdDecoder);
    fn hd_enc_open(
        path: *const c_char,
        width: c_int,
        height: c_int,
        fps: c_int,
        rate: c_int,
        bitrate: c_int,
        hardware: c_int,
        preset: *const c_char,
        err: *mut c_char,
        err_len: c_int,
    ) -> *mut HdEncoder;
    fn hd_enc_video(e: *mut HdEncoder, rgba: *const u8, index: i64) -> c_int;
    fn hd_enc_audio(e: *mut HdEncoder, left: *const f32, right: *const f32, frames: c_int)
    -> c_int;
    fn hd_enc_finish(e: *mut HdEncoder) -> c_int;
    fn hd_enc_close(e: *mut HdEncoder);
    fn hd_error_text(code: c_int, err: *mut c_char, err_len: c_int);
}

/// FFmpeg's log down to its errors, once.
fn init() {
    static INIT: Once = Once::new();
    // SAFETY: no arguments; FFmpeg's log level is a global it reads.
    INIT.call_once(|| unsafe { hd_init() });
}

/// The text of the C side's error buffer.
fn error_text(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if text.is_empty() {
        "FFmpeg failed".into()
    } else {
        text
    }
}

/// FFmpeg's text for error `code`.
fn code_text(code: c_int) -> String {
    let mut buf = [0 as c_char; 256];
    // SAFETY: the buffer and its length match.
    unsafe { hd_error_text(code, buf.as_mut_ptr(), buf.len() as c_int) };
    error_text(&buf)
}

/// FFmpeg's end of stream (`AVERROR_EOF`).
const EOF: c_int =
    -((b'E' as c_int) | ((b'O' as c_int) << 8) | ((b'F' as c_int) << 16) | ((b' ' as c_int) << 24));

/// A media file to read: a path, or on Android a picked document's descriptor, which this owns and closes. Several
/// decoders share one (the picture's and the sound's), each reading at its own offset.
pub struct MediaFile {
    path: Option<CString>,
    fd: i32,
}

impl MediaFile {
    /// The Displayer's media at `location`: a path, or an Android `content://` URI. `None` when it cannot be opened
    /// (the file is gone).
    pub fn open(location: &str) -> Option<Arc<Self>> {
        #[cfg(target_os = "android")]
        if location.starts_with("content://") {
            let fd = crate::android::open_uri_fd(location)?;
            return Some(Arc::new(Self { path: None, fd }));
        }
        if !Path::new(location).is_file() {
            return None;
        }
        let path = CString::new(location).ok()?;
        Some(Arc::new(Self {
            path: Some(path),
            fd: -1,
        }))
    }
}

impl Drop for MediaFile {
    fn drop(&mut self) {
        #[cfg(unix)]
        if self.fd >= 0 {
            // SAFETY: the descriptor is ours (detached from Java for us) and nothing reads it any more: every decoder
            // holds this file alive.
            unsafe { libc_close(self.fd) };
        }
    }
}

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "close"]
    fn libc_close(fd: c_int) -> c_int;
}

/// One stream's decoder: the picture's or the sound's.
pub struct Decoder {
    raw: NonNull<HdDecoder>,
    /// The stream's length in seconds, 0 if unknown.
    pub duration: f64,
    /// The picture's display size (rotation applied); 0 × 0 for sound.
    pub size: (u32, u32),
    /// The picture comes from MediaCodec (Android's hardware decoders).
    pub hardware: bool,
    _file: Arc<MediaFile>,
}

// SAFETY: a decoder is used by one thread at a time (it moves to its thread); FFmpeg's contexts allow that.
unsafe impl Send for Decoder {}

/// A decoded picture, RGBA rows packed, valid until the decoder's next call.
pub struct Picture<'a> {
    pub rgba: &'a [u8],
    pub width: u32,
    pub height: u32,
    /// Its time, seconds from the stream's start.
    pub secs: f64,
}

impl Decoder {
    /// The picture (`audio` false) or the sound of `file`: MediaCodec first with `hardware`; sound at `rate` Hz.
    pub fn open(
        file: &Arc<MediaFile>,
        audio: bool,
        hardware: bool,
        rate: u32,
    ) -> Result<Self, String> {
        init();
        let mut err = [0 as c_char; 256];
        let path = file.path.as_ref().map_or(std::ptr::null(), |p| p.as_ptr());
        // SAFETY: the path outlives the call; the error buffer and its length match.
        let raw = unsafe {
            hd_dec_open(
                path,
                file.fd,
                c_int::from(audio),
                c_int::from(hardware),
                rate as c_int,
                err.as_mut_ptr(),
                err.len() as c_int,
            )
        };
        let raw = NonNull::new(raw).ok_or_else(|| error_text(&err))?;
        let (mut duration, mut w, mut h, mut hw) = (0.0, 0, 0, 0);
        // SAFETY: a decoder just opened.
        unsafe { hd_dec_info(raw.as_ptr(), &mut duration, &mut w, &mut h, &mut hw) };
        Ok(Self {
            raw,
            duration: if duration.is_finite() {
                duration.max(0.0)
            } else {
                0.0
            },
            size: (w.max(0) as u32, h.max(0) as u32),
            hardware: hw != 0,
            _file: file.clone(),
        })
    }

    /// Pictures come out at `width` × `height`.
    pub fn set_size(&mut self, width: u32, height: u32) {
        // SAFETY: our decoder.
        unsafe { hd_dec_set_size(self.raw.as_ptr(), width as c_int, height as c_int) };
    }

    /// Goes to the key frame at or before `secs`: the next frames lead up to it.
    pub fn seek(&mut self, secs: f64) -> bool {
        // SAFETY: our decoder.
        unsafe { hd_dec_seek(self.raw.as_ptr(), secs.max(0.0)) >= 0 }
    }

    /// The next picture: `Ok(None)` at the end. With `skip` it is decoded only, its `rgba` empty.
    pub fn picture(&mut self, skip: bool) -> Result<Option<Picture<'_>>, String> {
        let (mut rgba, mut w, mut h, mut secs) = (std::ptr::null(), 0, 0, 0.0);
        // SAFETY: our decoder; the out pointers are locals.
        let ret = unsafe {
            hd_dec_video(
                self.raw.as_ptr(),
                c_int::from(skip),
                &mut rgba,
                &mut w,
                &mut h,
                &mut secs,
            )
        };
        match ret {
            1 => {
                let len = (w.max(0) as usize) * (h.max(0) as usize) * 4;
                let rgba = if rgba.is_null() || skip {
                    &[][..]
                } else {
                    // SAFETY: the C side hands out w × h × 4 bytes, valid until its next call, which needs `&mut self`.
                    unsafe { std::slice::from_raw_parts(rgba, len) }
                };
                Ok(Some(Picture {
                    rgba,
                    width: w.max(0) as u32,
                    height: h.max(0) as u32,
                    secs,
                }))
            }
            0 => Ok(None),
            code => Err(code_text(code)),
        }
    }

    /// The next sound, interleaved stereo at the rate it was opened with, and its first frame's time: `Ok(None)` at
    /// the end. The frames may be none (the resampler holding some back).
    pub fn sound(&mut self) -> Result<Option<(&[f32], f64)>, String> {
        let (mut pcm, mut secs) = (std::ptr::null(), 0.0);
        // SAFETY: our decoder; the out pointers are locals.
        let ret = unsafe { hd_dec_audio(self.raw.as_ptr(), &mut pcm, &mut secs) };
        if ret == EOF {
            return Ok(None);
        }
        if ret < 0 {
            return Err(code_text(ret));
        }
        let frames = if pcm.is_null() {
            &[][..]
        } else {
            // SAFETY: the C side hands out `ret` stereo frames, valid until its next call, which needs `&mut self`.
            unsafe { std::slice::from_raw_parts(pcm, ret as usize * 2) }
        };
        Ok(Some((frames, secs)))
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: our decoder, closed once.
        unsafe { hd_dec_close(self.raw.as_ptr()) };
    }
}

/// An MP4 file being written: H.264 pictures and AAC stereo sound.
pub struct Encoder {
    raw: NonNull<HdEncoder>,
    width: u32,
    height: u32,
    frames: i64,
}

// SAFETY: used by one thread at a time.
unsafe impl Send for Encoder {}

impl Encoder {
    /// An MP4 at `path`, `width` × `height` at `fps`, sound at `rate` Hz. x264 encodes (its `veryfast` preset): the
    /// same everywhere, and the frame size free.
    pub fn create(
        path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        rate: u32,
    ) -> Result<Self, String> {
        init();
        let path_c = CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let preset = CString::new("veryfast").expect("no NUL");
        // About 0.1 bit a pixel at 30 fps: 1080p30 at 8 Mbit/s.
        let bitrate = ((width * height) as f64 * fps as f64 * 0.13).clamp(1.0e6, 2.4e7) as c_int;
        let mut err = [0 as c_char; 256];
        // SAFETY: the strings outlive the call; the error buffer and its length match.
        let raw = unsafe {
            hd_enc_open(
                path_c.as_ptr(),
                width as c_int,
                height as c_int,
                fps as c_int,
                rate as c_int,
                bitrate,
                0,
                preset.as_ptr(),
                err.as_mut_ptr(),
                err.len() as c_int,
            )
        };
        let raw = NonNull::new(raw).ok_or_else(|| error_text(&err))?;
        Ok(Self {
            raw,
            width,
            height,
            frames: 0,
        })
    }

    /// The next picture, RGBA `width × height × 4`, rows packed.
    pub fn picture(&mut self, rgba: &[u8]) -> Result<(), String> {
        assert_eq!(rgba.len(), (self.width * self.height * 4) as usize);
        // SAFETY: our encoder; the frame is the size it was opened with.
        let ret = unsafe { hd_enc_video(self.raw.as_ptr(), rgba.as_ptr(), self.frames) };
        self.frames += 1;
        if ret < 0 { Err(code_text(ret)) } else { Ok(()) }
    }

    /// The next sound, the channels apart.
    pub fn sound(&mut self, left: &[f32], right: &[f32]) -> Result<(), String> {
        let n = left.len().min(right.len());
        if n == 0 {
            return Ok(());
        }
        // SAFETY: our encoder; both slices hold n frames.
        let ret =
            unsafe { hd_enc_audio(self.raw.as_ptr(), left.as_ptr(), right.as_ptr(), n as c_int) };
        if ret < 0 { Err(code_text(ret)) } else { Ok(()) }
    }

    /// Flushes and writes the file's end.
    pub fn finish(self) -> Result<(), String> {
        // SAFETY: our encoder; dropped (closed) after.
        let ret = unsafe { hd_enc_finish(self.raw.as_ptr()) };
        if ret < 0 { Err(code_text(ret)) } else { Ok(()) }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: our encoder, closed once.
        unsafe { hd_enc_close(self.raw.as_ptr()) };
    }
}
