//! Track folders: the playlist's channels grouped under a folder row that opens and closes, so a song of many channels
//! scrolls less. A folder row shows right above its first member (the playlist's order), an empty folder's at the end
//! of the list; an open folder's members show under it, marked as its, a closed folder's none. A folder's members stay
//! together in the playlist ([`Song::gather_folders`]).
//!
//! Members are named by channel id, so a reorder, an insert or a delete moves nothing; an id no channel has any more is
//! ignored, and dropped by [`Song::prune_folders`]. The master is in none. A folder stays when it empties: only
//! [`Song::delete_folder`] takes it away (its channels stay, out of any folder).
//!
//! Stored in Honeydew's own `HDFo` chunk, one per folder ([`write`], [`read`]); other apps keep it as an unknown chunk.

use super::Song;
use crate::flm::FlmError;
use crate::flm::chunk::{ChunkWriter, Cursor};

/// The chunk id of a folder.
pub const CHUNK: [u8; 4] = *b"HDFo";

/// The most folders a song keeps (and a file is read with).
pub const MAX_FOLDERS: usize = 64;

/// The longest folder name kept, in characters.
pub const MAX_NAME: usize = 64;

/// A folder of channels.
#[derive(Clone, Debug, PartialEq)]
pub struct Folder {
    pub name: String,
    /// Shows its members under its row; closed, its row alone.
    pub open: bool,
    /// Its members' channel ids, in the order they joined.
    pub channels: Vec<i32>,
    /// Its colour's hue (0 … 1, as a channel's): its row, its bar down its members' headers.
    pub hue: f32,
}

/// The hue a song's `n`-th folder gets: golden-ratio steps, as the channels' ([`Song::insert_channel`]), from another
/// start.
pub fn default_hue(n: usize) -> f32 {
    (0.08 + n as f64 * 0.618_034).fract() as f32
}

/// Where a channel stands in a folder ([`Song::folder_place`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FolderPlace {
    /// The folder's index in `song.file.folders`.
    pub folder: usize,
    pub open: bool,
    /// How many members the folder has (channels that exist).
    pub members: usize,
}

impl Song {
    /// The folder channel `channel` is in, by index.
    pub fn folder_of(&self, channel: usize) -> Option<usize> {
        let id = self.channels.get(channel).filter(|_| channel > 0)?.id;
        (self.file.folders.iter()).position(|f| f.channels.contains(&id))
    }

    /// The channels of folder `folder` that exist, in playlist order.
    pub fn folder_members(&self, folder: usize) -> Vec<usize> {
        let Some(f) = self.file.folders.get(folder) else {
            return Vec::new();
        };
        (self.playlist().into_iter())
            .filter(|&c| c > 0 && f.channels.contains(&self.channels[c].id))
            .collect()
    }

    /// Where channel `channel` stands in its folder, if it is in one.
    pub fn folder_place(&self, channel: usize) -> Option<FolderPlace> {
        let folder = self.folder_of(channel)?;
        Some(FolderPlace {
            folder,
            open: self.file.folders[folder].open,
            members: self.folder_members(folder).len(),
        })
    }

    /// Whether channel `channel` has no row as its folder is closed.
    pub fn folded_away(&self, channel: usize) -> bool {
        self.folder_place(channel).is_some_and(|p| !p.open)
    }

    /// A new open folder `Folder N` (the smallest free N ≥ 1), holding channel `channel` if one is given (it leaves its
    /// folder). Returns its index; `None` past [`MAX_FOLDERS`], or for the master or a missing channel.
    pub fn new_folder(&mut self, channel: Option<usize>) -> Option<usize> {
        let bad = channel.is_some_and(|c| c == 0 || c >= self.channels.len());
        if bad || self.file.folders.len() >= MAX_FOLDERS {
            return None;
        }
        let name = (1..)
            .map(|n| format!("Folder {n}"))
            .find(|n| self.file.folders.iter().all(|f| &f.name != n))?;
        let hue = default_hue(self.file.folders.len());
        self.file.folders.push(Folder {
            name,
            open: true,
            channels: Vec::new(),
            hue,
        });
        let folder = self.file.folders.len() - 1;
        if let Some(channel) = channel {
            self.set_folder_member(channel, Some(folder));
        }
        Some(folder)
    }

    /// Makes channel `channel` a member of `folder` (none: of no folder), and of no other, where it stands in the
    /// playlist. False, and no change, for the master, a missing channel or folder, or the folder it is in already.
    pub fn set_folder_member(&mut self, channel: usize, folder: Option<usize>) -> bool {
        if channel == 0 || channel >= self.channels.len() {
            return false;
        }
        if folder.is_some_and(|f| f >= self.file.folders.len()) || self.folder_of(channel) == folder
        {
            return false;
        }
        let id = self.channels[channel].id;
        for f in &mut self.file.folders {
            f.channels.retain(|&c| c != id);
        }
        if let Some(f) = folder {
            self.file.folders[f].channels.push(id);
        }
        true
    }

    /// The track menu's `Move to <folder>` / `Remove from Folder`: channel `channel` into `folder` (none: out of its
    /// folder). Into a folder with members it goes right below the last of them; out of one, right below the folder's
    /// last other member, so that it leaves the folder's place. False, and no change, as for [`Self::set_folder_member`].
    pub fn move_to_folder(&mut self, channel: usize, folder: Option<usize>) -> bool {
        let Some(from) = self.folder_of(channel).or(folder) else {
            return false;
        };
        if !self.set_folder_member(channel, folder) {
            return false;
        }
        let id = self.channels[channel].id;
        // Below the folder's last member other than it (the joined folder's, else the left one's).
        let anchor = folder.unwrap_or(from);
        let others: Vec<i32> = (self.file.folders[anchor].channels.iter())
            .copied()
            .filter(|&c| c != id)
            .collect();
        let mut order = self.playlist();
        let last = (order.iter()).rposition(|&c| others.contains(&self.channels[c].id));
        if let (Some(last), Some(at)) = (last, order.iter().position(|&c| c == channel)) {
            order.remove(at);
            let to = if at <= last { last } else { last + 1 };
            order.insert(to, channel);
            self.set_playlist(&order);
        }
        self.gather_folders();
        true
    }

    /// Keeps each folder's members together: those after a channel of no folder (or another's) move up to right below
    /// the folder's first member, in their order. Whether anything moved.
    pub fn gather_folders(&mut self) -> bool {
        let mut moved = false;
        for folder in 0..self.file.folders.len() {
            let order = self.playlist();
            let ids = &self.file.folders[folder].channels;
            let member = |c: usize| c > 0 && ids.contains(&self.channels[c].id);
            let Some(first) = order.iter().position(|&c| member(c)) else {
                continue;
            };
            let members: Vec<usize> = order.iter().copied().filter(|&c| member(c)).collect();
            let rest: Vec<usize> = order.iter().copied().filter(|&c| !member(c)).collect();
            let mut gathered = rest[..first].to_vec();
            gathered.extend(&members);
            gathered.extend(&rest[first..]);
            moved |= self.set_playlist(&gathered);
        }
        moved
    }

    /// Opens a closed folder, closes an open one.
    pub fn toggle_folder(&mut self, folder: usize) {
        if let Some(f) = self.file.folders.get_mut(folder) {
            f.open = !f.open;
        }
    }

    /// Opens or closes every folder.
    pub fn set_folders_open(&mut self, open: bool) {
        for f in &mut self.file.folders {
            f.open = open;
        }
    }

    /// Renames folder `folder` to `name`, trimmed and at most [`MAX_NAME`] characters. False, and no change, for a blank
    /// name, the same one, or a missing folder.
    pub fn rename_folder(&mut self, folder: usize, name: &str) -> bool {
        let name: String = name.trim().chars().take(MAX_NAME).collect();
        match self.file.folders.get_mut(folder) {
            Some(f) if !name.is_empty() && f.name != name => {
                f.name = name;
                true
            }
            _ => false,
        }
    }

    /// Sets folder `folder`'s hue. Whether it changed.
    pub fn set_folder_hue(&mut self, folder: usize, hue: f32) -> bool {
        match self.file.folders.get_mut(folder) {
            Some(f) if f.hue != hue && hue.is_finite() => {
                f.hue = hue.clamp(0.0, 1.0);
                true
            }
            _ => false,
        }
    }

    /// A new channel at index `new` joins the folder of the channel at index `of` (the one it went in above, or was
    /// cloned from), if that is in one: a channel inserted or cloned among a folder's tracks is one of them.
    pub(super) fn join_folder_of(&mut self, new: usize, of: usize) {
        if let Some(folder) = self.folder_of(of) {
            self.set_folder_member(new, Some(folder));
        }
    }

    /// Whether every member of folder `folder` is muted (channel strip param 2 on), and it has one.
    pub fn folder_muted(&self, folder: usize) -> bool {
        let members = self.folder_members(folder);
        !members.is_empty() && members.iter().all(|&c| self.channels[c].params.0[2] != 0.0)
    }

    /// Whether every member of folder `folder` is soloed (channel strip param 3 past 0.5), and it has one.
    pub fn folder_soloed(&self, folder: usize) -> bool {
        let members = self.folder_members(folder);
        !members.is_empty() && members.iter().all(|&c| self.channels[c].params.0[3] > 0.5)
    }

    /// Deletes folder `folder`; its channels stay where they are, in no folder.
    pub fn delete_folder(&mut self, folder: usize) -> bool {
        if folder >= self.file.folders.len() {
            return false;
        }
        self.file.folders.remove(folder);
        true
    }

    /// Drops the ids no channel has (or the master's). Empty folders stay.
    pub fn prune_folders(&mut self) {
        let ids: Vec<i32> = self.channels.iter().skip(1).map(|c| c.id).collect();
        for f in &mut self.file.folders {
            f.channels.retain(|c| ids.contains(c));
        }
    }
}

/// Writes one `HDFo` chunk a folder: open (u8), the name, the member count (u32), the ids (i32 each) and the hue
/// (f32).
pub fn write(writer: &mut ChunkWriter, folders: &[Folder]) -> Result<(), FlmError> {
    for f in folders {
        writer.chunk(CHUNK, |w| {
            w.u8(u8::from(f.open))?;
            w.vstr32(f.name.as_bytes())?;
            w.u32(f.channels.len() as u32)?;
            for &id in &f.channels {
                w.u32(id as u32)?;
            }
            w.f32(f.hue)?;
            Ok(())
        })?;
    }
    Ok(())
}

/// Reads an `HDFo`'s `payload` as the song's `n`-th folder. Fails on a short one, which the reader then keeps as an
/// unknown chunk. One without a hue (an older build's) gets the `n`-th [`default_hue`].
pub fn read(payload: &[u8], path: &str, n: usize) -> Result<Folder, FlmError> {
    let mut r = Cursor::new(payload, path);
    let open = r.u8()? & 1 == 1;
    let name: String = String::from_utf8_lossy(r.vstr32()?)
        .chars()
        .take(MAX_NAME)
        .collect();
    let count = r.u32()? as usize;
    // Never more ids than the payload holds.
    let mut channels = Vec::with_capacity(count.min(payload.len() / 4));
    for _ in 0..count {
        channels.push(r.u32()? as i32);
    }
    let hue = r.f32_or(f32::NAN);
    let hue = match hue.is_finite() {
        true => hue.clamp(0.0, 1.0),
        false => default_hue(n),
    };
    Ok(Folder {
        name,
        open,
        channels,
        hue,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::{ChannelKind, SynthKind};

    /// The master and five synths, 1 … 5.
    fn song() -> Song {
        let mut song = Song::default();
        for _ in 0..5 {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        }
        song
    }

    /// The index of the channel with id `id`.
    fn at(song: &Song, id: i32) -> usize {
        song.channels.iter().position(|c| c.id == id).unwrap()
    }

    /// The channels' ids in playlist order.
    fn ids(song: &Song) -> Vec<i32> {
        song.playlist()
            .into_iter()
            .map(|c| song.channels[c].id)
            .collect()
    }

    #[test]
    fn moving_into_a_folder_puts_the_channel_below_its_last_member() {
        let mut song = song();
        let f = song.new_folder(Some(at(&song, 2))).unwrap();
        assert_eq!(song.file.folders[f].name, "Folder 1");
        assert!(song.move_to_folder(at(&song, 4), Some(f)));
        assert_eq!(ids(&song), [0, 1, 2, 4, 3, 5]);
        let place = song.folder_place(at(&song, 4)).unwrap();
        assert_eq!((place.folder, place.open, place.members), (f, true, 2));
        song.toggle_folder(f);
        assert!(song.folded_away(at(&song, 2)) && song.folded_away(at(&song, 4)));
        assert!(!song.folded_away(at(&song, 3)));
        assert_eq!(song.new_folder(Some(0)), None, "the master is in none");
    }

    #[test]
    fn an_emptied_folder_stays_until_deleted_and_a_deleted_channel_is_pruned() {
        let mut song = song();
        let f = song.new_folder(Some(at(&song, 1))).unwrap();
        song.move_to_folder(at(&song, 2), Some(f));
        assert!(song.move_to_folder(at(&song, 1), None));
        assert_eq!(ids(&song), [0, 2, 1, 3, 4, 5], "1 leaves below the folder");
        assert!(!song.move_to_folder(at(&song, 1), None), "in none already");
        let two = at(&song, 2);
        song.channels.remove(two);
        song.prune_folders();
        assert_eq!(song.file.folders.len(), 1);
        assert!(song.file.folders[0].channels.is_empty());
        assert!(song.delete_folder(0));
        assert!(song.file.folders.is_empty());
        // An empty folder from the `+`.
        assert_eq!(song.new_folder(None), Some(0));
        assert!(song.rename_folder(0, "  Drums  "));
        assert_eq!(song.file.folders[0].name, "Drums");
        assert!(!song.rename_folder(0, "   "));
    }

    #[test]
    fn a_channel_inserted_or_cloned_among_a_folder_s_tracks_joins_it() {
        let mut song = song();
        let f = song.new_folder(Some(at(&song, 2))).unwrap();
        song.move_to_folder(at(&song, 3), Some(f));
        // Above its first member, the playlist's `Insert Channel` there.
        let new = song
            .insert_channel(at(&song, 2), ChannelKind::Drums)
            .unwrap();
        assert_eq!(song.folder_of(new), Some(f));
        let copy = song.clone_channel(at(&song, 3));
        assert_eq!(song.folder_of(copy), Some(f));
        // Added at the end, or above a channel in no folder: in none.
        song.add_channel(ChannelKind::Drums);
        assert_eq!(song.folder_of(song.channels.len() - 1), None);
        let new = song
            .insert_channel(at(&song, 1), ChannelKind::Drums)
            .unwrap();
        assert_eq!(song.folder_of(new), None);
        assert_eq!(song.folder_members(f).len(), 4);
        // Mute and solo state over the members.
        assert!(!song.folder_muted(f) && !song.folder_soloed(f));
        for c in song.folder_members(f) {
            song.channels[c].params.0[2] = 1.0;
        }
        assert!(song.folder_muted(f));
    }

    #[test]
    fn a_folder_s_members_are_gathered_below_its_first() {
        let mut song = song();
        let f = song.new_folder(Some(at(&song, 1))).unwrap();
        // 4 joins where it stands, two channels away: it moves up below 1.
        song.set_folder_member(at(&song, 4), Some(f));
        assert!(song.gather_folders());
        assert_eq!(ids(&song), [0, 1, 4, 2, 3, 5]);
        assert!(!song.gather_folders());
    }

    #[test]
    fn folders_survive_a_save_and_a_load() {
        let mut song = song();
        let f = song.new_folder(Some(at(&song, 2))).unwrap();
        song.move_to_folder(at(&song, 4), Some(f));
        song.rename_folder(f, "Leads");
        song.set_folder_hue(f, 0.25);
        song.toggle_folder(f);
        // An empty one is kept too.
        song.new_folder(None);
        let back = crate::flm::read(&crate::flm::write(&song).unwrap()).unwrap();
        assert_eq!(back.file.folders, song.file.folders);
    }

    #[test]
    fn folders_round_trip_through_their_chunks() {
        let mut song = song();
        let f = song.new_folder(Some(3)).unwrap();
        song.move_to_folder(at(&song, 5), Some(f));
        song.toggle_folder(f);
        let mut writer = ChunkWriter::default();
        write(&mut writer, &song.file.folders).unwrap();
        let bytes = writer.into_bytes();
        // A chunk: the id, the payload's length, then the payload.
        let payload = &bytes[8..];
        assert_eq!(&bytes[..4], &CHUNK);
        assert_eq!(read(payload, "HDFo", 0).unwrap(), song.file.folders[0]);
        assert!(read(&payload[..3], "HDFo", 0).is_err());
        // An older build's chunk, without the hue, gets the default one.
        let old = read(&payload[..payload.len() - 4], "HDFo", 3).unwrap();
        assert_eq!(old.hue, default_hue(3));
    }
}
