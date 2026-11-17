#![cfg_attr(target_arch = "wasm32", allow(dead_code))]
//! The working copy: the song as it is now, unsaved changes and all, kept in the user folder as [`SESSION_FILE`], so
//! that closing the app (or Android ending it in the background) loses nothing. It is never the song's own file: the
//! song's `.flm` changes only when the user saves, as before.
//!
//! - Kept ([`ProjectFiles::keep_session`], at each frame's end): [`SESSION_DELAY`] seconds after the last change (a
//!   drag writes once, at its end), at once when the app loses focus (Android's background) and at exit, encoded and
//!   written whole on a worker. Only while the song has unsaved changes ([`ProjectFiles::modified`]): a save, a load,
//!   a new song or `Discard changes` clears the flag, and the file is removed.
//! - Restored ([`ProjectFiles::restore_session`], at start, in place of the last song): the song comes back with
//!   its unsaved changes, still the file it was opened from (a save writes there, once asked) or untitled when it
//!   had none or that file is gone.
//! - On disk only: the web, and the session's in-memory copies, have none; tests only write into temp folders.
//!
//! The file: [`MAGIC`], a version byte, the song file's path (u32 LE byte length, UTF-8; 0 for none), then the song
//! as a `.flm` would hold it.

use std::path::{Path, PathBuf};

use super::ProjectFiles;
use crate::song::Song;

/// The working copy's file in the user folder.
pub const SESSION_FILE: &str = "session.state";
/// Seconds after the last change before the working copy is kept.
pub const SESSION_DELAY: f64 = 2.0;
/// The flash when a start brings unsaved changes back.
pub const RESTORED: &str = "Unsaved changes restored";
const MAGIC: &[u8; 4] = b"HDSS";
const VERSION: u8 = 1;

/// What is known of the working copy on disk.
#[derive(Default)]
pub struct Session {
    /// [`ProjectFiles`]' change count when last seen, and when (the app's time) it was.
    seen: u64,
    seen_at: f64,
    /// The change count the file holds.
    kept: u64,
    /// The file is there, or being written.
    on_disk: bool,
    /// The last write or removal, on its worker: the next waits for it, so they land in order.
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<std::thread::JoinHandle<()>>,
}

/// The working copy's bytes: `song` (a `.flm`'s) and the song file it belongs to.
pub fn encode(file: Option<&Path>, song: &[u8]) -> Vec<u8> {
    let path = file.and_then(Path::to_str).unwrap_or("").as_bytes();
    let mut bytes = Vec::with_capacity(9 + path.len() + song.len());
    bytes.extend_from_slice(MAGIC);
    bytes.push(VERSION);
    bytes.extend_from_slice(&(path.len() as u32).to_le_bytes());
    bytes.extend_from_slice(path);
    bytes.extend_from_slice(song);
    bytes
}

/// The song file (`None`: untitled) and the song's bytes of a working copy; `None` when it is none.
pub fn decode(bytes: &[u8]) -> Option<(Option<PathBuf>, &[u8])> {
    let rest = bytes.strip_prefix(MAGIC)?;
    let (&version, rest) = rest.split_first()?;
    if version != VERSION || rest.len() < 4 {
        return None;
    }
    let (len, rest) = rest.split_at(4);
    let len = u32::from_le_bytes(len.try_into().ok()?) as usize;
    if rest.len() < len {
        return None;
    }
    let (path, song) = rest.split_at(len);
    let path = std::str::from_utf8(path).ok()?;
    let file = (!path.is_empty()).then(|| PathBuf::from(path));
    Some((file, song))
}

impl ProjectFiles {
    /// Where the working copy goes: the user folder on disk only (in tests, a temp folder only).
    #[cfg(not(target_arch = "wasm32"))]
    fn session_path(&self) -> Option<PathBuf> {
        let allowed =
            |u: &&Path| u.is_absolute() && (cfg!(not(test)) || u.starts_with(std::env::temp_dir()));
        let user = self.user.as_deref().filter(allowed).filter(|_| !self.copies)?;
        Some(user.join(SESSION_FILE))
    }

    /// Each frame's end, `now` the app's time: keeps the working copy of `song` while it has unsaved changes,
    /// [`SESSION_DELAY`] after its last change, or at once when `urgent` (focus lost, exit); removes it once there
    /// are none. Returns the seconds until a write is due, for a repaint then (egui paints only on input).
    pub fn keep_session(&mut self, song: &Song, now: f64, urgent: bool) -> Option<f64> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = self.session_path()?;
            let edits = self.edits;
            let session = &mut self.session;
            if session.seen != edits {
                (session.seen, session.seen_at) = (edits, now);
            }
            if !self.changed {
                if std::mem::take(&mut session.on_disk) {
                    session.run(move || match std::fs::remove_file(&path) {
                        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                            eprintln!("{} not removed: {e}", path.display())
                        }
                        _ => {}
                    });
                }
                return None;
            }
            if session.on_disk && session.kept == edits {
                return None;
            }
            let wait = SESSION_DELAY - (now - session.seen_at);
            if !urgent && wait > 0.0 {
                return Some(wait);
            }
            (session.kept, session.on_disk) = (edits, true);
            let file = self.song_file().map(Path::to_owned);
            let (song, write) = (song.clone(), self.codec.write);
            self.session.run(move || {
                let written = write(&song).and_then(|bytes| {
                    super::write_atomic(&path, &encode(file.as_deref(), &bytes))
                        .map_err(|e| e.to_string())
                });
                if let Err(err) = written {
                    eprintln!("{} not written: {err}", path.display());
                }
            });
            None
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (song, now, urgent);
            None
        }
    }

    /// Waits for the working copy's write or removal under way (at exit, so that it lands before the app is gone).
    pub fn finish_session(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(worker) = self.session.worker.take() {
            let _ = worker.join();
        }
    }

    /// A working copy was kept: [`Self::restore_session`] has one to try.
    pub fn has_session(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.session_path().is_some_and(|path| path.is_file())
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }

    /// Drops the working copy unread: the start before was cut short opening it (a song that brings the app down).
    pub fn discard_session(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = self.session_path() {
            let _ = std::fs::remove_file(path);
        }
    }

    /// At start: the working copy's song, with its unsaved changes, when one was kept; it stays the file it was
    /// opened from when that is still there, else it is untitled. `None` (and the file removed) when there is none or
    /// it cannot be read.
    pub fn restore_session(&mut self) -> Option<Song> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = self.session_path()?;
            let file = std::fs::File::open(&path).ok()?;
            let restored = super::read_file_limited(file, super::MAX_FILE_BYTES)
                .ok()
                .and_then(|bytes| self.session_song(&bytes));
            if restored.is_none() {
                eprintln!("{} could not be restored", path.display());
                let _ = std::fs::remove_file(&path);
            }
            restored
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }

    /// The song of the working copy `bytes`, now the open one with unsaved changes.
    #[cfg(not(target_arch = "wasm32"))]
    fn session_song(&mut self, bytes: &[u8]) -> Option<Song> {
        let (file, song_bytes) = decode(bytes)?;
        let (mut song, _) = (self.codec.read)(song_bytes).ok()?;
        if song.channels.is_empty() {
            return None;
        }
        crate::panels::cstm::load_dw_programs(&mut song, &crate::sample_cache::reader(self));
        let file = file.filter(|f| self.provider.exists(f) || self.library.exists(f));
        match file {
            Some(file) => self.set_path(file),
            None => self.path = None,
        }
        self.changed = true;
        let session = &mut self.session;
        (session.seen, session.kept, session.on_disk) = (self.edits, self.edits, true);
        self.flash = Some(RESTORED.into());
        Some(song)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Session {
    /// Runs `job` on a worker after the last one.
    fn run(&mut self, job: impl FnOnce() + Send + 'static) {
        let previous = self.worker.take();
        self.worker = Some(std::thread::spawn(move || {
            if let Some(previous) = previous {
                let _ = previous.join();
            }
            job();
        }));
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::project_files::content::tests::Temp;

    #[test]
    fn the_working_copy_reads_back_its_file_and_song() {
        let bytes = encode(Some(Path::new("/songs/Beat.flm")), b"song");
        assert_eq!(
            decode(&bytes),
            Some((Some(PathBuf::from("/songs/Beat.flm")), &b"song"[..]))
        );
        assert_eq!(decode(&encode(None, b"x")), Some((None, &b"x"[..])));
        assert_eq!(decode(b"HDSS"), None);
        assert_eq!(decode(b"10LFsong"), None);
    }

    #[test]
    fn unsaved_changes_are_kept_restored_and_removed_once_saved() {
        let tmp = Temp::new("session");
        let state = tmp.0.join(SESSION_FILE);
        let mut files = ProjectFiles::on_disk(&tmp.0);
        let song = Song {
            title: "Kept".into(),
            tempo_bpm: 99.0,
            ..Song::default()
        };
        // Nothing changed: nothing kept.
        files.keep_session(&song, 0.0, false);
        files.finish_session();
        assert!(!state.exists());
        // A change: kept only after the delay, or at once when urgent.
        files.song_changed();
        files.keep_session(&song, 10.0, false);
        files.finish_session();
        assert!(!state.exists());
        files.keep_session(&song, 10.0 + SESSION_DELAY, false);
        files.finish_session();
        assert!(state.exists());
        // The next start brings it back, unsaved and untitled.
        let mut next = ProjectFiles::on_disk(&tmp.0);
        let restored = next.restore_session().expect("restored");
        assert_eq!((restored.title.as_str(), restored.tempo_bpm), ("Kept", 99.0));
        assert!(next.modified());
        assert_eq!(next.song_file(), None);
        assert_eq!(next.take_flash().as_deref(), Some(RESTORED));
        // Saved (or discarded): the working copy goes.
        next.saved();
        next.keep_session(&restored, 20.0, false);
        next.finish_session();
        assert!(!state.exists());
        assert!(ProjectFiles::on_disk(&tmp.0).restore_session().is_none());
    }

    #[test]
    fn a_damaged_working_copy_is_dropped() {
        let tmp = Temp::new("session-bad");
        let state = tmp.0.join(SESSION_FILE);
        std::fs::write(&state, b"not one").unwrap();
        let mut files = ProjectFiles::on_disk(&tmp.0);
        assert!(files.restore_session().is_none());
        assert!(!files.modified());
        assert!(!state.exists());
    }
}
