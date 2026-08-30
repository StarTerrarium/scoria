//! Repository traits.
//!
//! `CLAUDE.md`: database access goes through these, with no raw SQL in
//! `playback/` or in any UI crate. That keeps the storage engine a decision we
//! can revisit -- §12 records rusqlite as chosen over Turso for now, and this
//! is what makes revisiting it nearly free.
//!
//! The traits also take and return [`RelativePath`], never `PathBuf`, so the
//! §4.3 rule that no absolute path is ever persisted is carried by the type
//! rather than by discipline.

use crate::date::Date;
use crate::error::CoreError;
use crate::id::{AlbumId, TrackId};
use crate::library::AlbumIdentity;
use crate::model::{Album, AudioProperties, ReplayGain, Timestamp, Track};
use crate::path::RelativePath;
use std::time::Duration;

/// A track as the scanner produces it, before storage assigns an id.
#[derive(Debug, Clone, PartialEq)]
pub struct NewTrack {
    pub album_id: AlbumId,
    pub path: RelativePath,
    pub title: String,
    pub artist: String,
    pub album_artist: String,
    pub album_artist_sort: Option<String>,
    pub album: String,
    pub date: Option<Date>,
    pub disc: Option<u16>,
    pub track_number: Option<u16>,
    pub duration: Duration,
    pub replay_gain: ReplayGain,
    pub added_at: Timestamp,
    pub genre: Option<String>,
    pub composer: Option<String>,
    pub audio: AudioProperties,
}

/// A track the storage layer actually wrote.
///
/// Returned only for rows that were inserted or that genuinely differ from
/// what was already stored. A rescan sees every file again, and almost none of
/// them have changed; saying so is what lets the scanner report a retag
/// without also reporting the whole library every time the watcher wakes up.
///
/// Carries the path because unchanged rows produce nothing, so the results
/// cannot be lined up against the input by position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upserted {
    pub id: TrackId,
    pub path: RelativePath,
}

/// An album as the scanner produces it.
#[derive(Debug, Clone, PartialEq)]
pub struct NewAlbum {
    pub identity: AlbumIdentity,
    pub title: String,
    pub album_artist: String,
    pub album_artist_sort: Option<String>,
    pub date: Option<Date>,
}

/// Storage for tracks.
pub trait TrackRepository {
    /// Inserts or updates by path, returning the id either way.
    ///
    /// Keyed on path because that is what a rescan has in hand: the same file
    /// seen again is the same track, retagged or not.
    ///
    /// # Errors
    /// [`CoreError::Database`] if the write fails.
    fn upsert_track(&mut self, track: &NewTrack) -> Result<TrackId, CoreError>;

    /// Upserts many in one transaction, reporting only what actually changed.
    ///
    /// A scan of a six-figure library is one transaction per batch, not one
    /// per file. It also sees every file again on every pass -- the scanner
    /// has no mtime shortcut -- and almost none of them differ from what is
    /// stored. A row that matches is left alone and is not returned, so the
    /// caller can tell a retag from a file it has merely looked at again.
    ///
    /// # Errors
    /// [`CoreError::Database`] if the write fails. Either all of `tracks` are
    /// applied or none are.
    fn upsert_tracks(&mut self, tracks: &[NewTrack]) -> Result<Vec<Upserted>, CoreError>;

    /// # Errors
    /// [`CoreError::Database`] on failure, [`CoreError::CorruptData`] if a
    /// stored row cannot be represented by the model.
    fn track(&self, id: TrackId) -> Result<Option<Track>, CoreError>;

    /// Fetches many, preserving the caller's order and skipping ids that are
    /// no longer present.
    ///
    /// Order matters: the queue overlay renders a snapshot in snapshot order,
    /// and a repository that returned storage order would silently reorder it.
    ///
    /// # Errors
    /// As [`Self::track`].
    fn tracks_by_ids(&self, ids: &[TrackId]) -> Result<Vec<Track>, CoreError>;

    /// # Errors
    /// As [`Self::track`].
    fn track_by_path(&self, path: &RelativePath) -> Result<Option<Track>, CoreError>;

    /// # Errors
    /// As [`Self::track`].
    fn all_tracks(&self) -> Result<Vec<Track>, CoreError>;

    /// # Errors
    /// [`CoreError::Database`] on failure.
    fn track_count(&self) -> Result<usize, CoreError>;

    /// Marks tracks available or not.
    ///
    /// §7.4: files that vanish are marked, never deleted. A struck-through row
    /// states what will happen; a row that silently disappears makes users
    /// doubt the software.
    ///
    /// # Errors
    /// [`CoreError::Database`] on failure.
    fn set_available(&mut self, ids: &[TrackId], available: bool) -> Result<(), CoreError>;

    /// Counts one play of a track, returning the new total (§6.8).
    ///
    /// A method of its own rather than a field on [`NewTrack`], because a
    /// rescan writes every other field and must never be able to write this
    /// one. Incremented in the statement rather than read-modify-written here,
    /// so a scan running at the same time cannot lose a play.
    ///
    /// # Errors
    /// [`CoreError::Database`] on failure, [`CoreError::NoSuchTrack`] if the id
    /// is not in the library.
    fn record_play(&mut self, id: TrackId) -> Result<u32, CoreError>;

    /// Every path currently indexed, for the scanner to diff against the disk.
    ///
    /// # Errors
    /// As [`Self::track`].
    fn all_track_paths(&self) -> Result<Vec<(TrackId, RelativePath)>, CoreError>;

    /// Removes tracks outright. Used when a library is rescanned from scratch,
    /// not when a file merely goes missing.
    ///
    /// # Errors
    /// [`CoreError::Database`] on failure.
    fn delete_tracks(&mut self, ids: &[TrackId]) -> Result<(), CoreError>;

    /// Full-text search, most relevant first.
    ///
    /// # Errors
    /// [`CoreError::Database`] on failure.
    fn search_tracks(&self, query: &str) -> Result<Vec<TrackId>, CoreError>;
}

/// Storage for albums.
pub trait AlbumRepository {
    /// Inserts, or returns the existing id when the identity already resolves
    /// to a stored album (§4.2).
    ///
    /// # Errors
    /// [`CoreError::Database`] if the write fails.
    fn upsert_album(&mut self, album: &NewAlbum) -> Result<AlbumId, CoreError>;

    /// # Errors
    /// [`CoreError::Database`] on failure.
    fn album(&self, id: AlbumId) -> Result<Option<Album>, CoreError>;

    /// # Errors
    /// [`CoreError::Database`] on failure.
    fn album_by_identity(&self, identity: &AlbumIdentity) -> Result<Option<AlbumId>, CoreError>;

    /// # Errors
    /// [`CoreError::Database`] on failure.
    fn all_albums(&self) -> Result<Vec<Album>, CoreError>;

    /// # Errors
    /// [`CoreError::Database`] on failure.
    fn album_count(&self) -> Result<usize, CoreError>;

    /// Removes albums that no longer have any tracks. A rescan that empties an
    /// album should not leave it in the album pane.
    ///
    /// # Errors
    /// [`CoreError::Database`] on failure.
    fn remove_empty_albums(&mut self) -> Result<usize, CoreError>;
}
