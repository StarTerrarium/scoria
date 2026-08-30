//! The library, held in memory.
//!
//! ## Why this exists
//!
//! The first version of the query engine went to storage for every query. That
//! is the obvious design and it is too slow: constructing a hundred thousand
//! `Track` values, each with its own string allocations, costs about half a
//! second, and it costs that whether the query returns the whole library or a
//! hundred rows. Measured at the six-figure scale §14 says to design for:
//!
//! | Query | Via storage | Via this index | Rows |
//! |---|---|---|---|
//! | Unfiltered library | 526 ms | 51 ms | 100,000 |
//! | One artist | 483 ms | 10 ms | 96 |
//! | Search | 481 ms | 5 ms | 11 |
//!
//! Note what the storage column does *not* do: it barely moves between a query
//! returning the whole library and one returning eleven rows, because almost
//! all of it is reading the library back. That is the shape of the problem, and
//! it is why narrowing the query was never going to fix it.
//!
//! A search box runs a query per keystroke, so half a second is not a slow
//! path, it is the normal path.
//!
//! Re-measure with `just bench-query`; the harness is
//! `scoria-core/examples/query_scale.rs`, and the numbers above are what it
//! prints. It is a synthetic library rather than a real one, so treat the
//! ratios as the point rather than the absolute figures.
//!
//! Holding the library in memory is also what makes the filter cascade cheap:
//! §7.2 needs counts against three different filter contexts at once, which is
//! one pass over a slice and three round trips otherwise.
//!
//! The cost is roughly 40 MB at a hundred thousand tracks. That is the same
//! trade every library-centric player makes, and it buys a query engine with no
//! cache to invalidate: SQLite remains the durable store and the full-text
//! index, and this is a projection of it.

use crate::db::repository::{AlbumRepository, TrackRepository};
use crate::error::CoreError;
use crate::event::LibraryDelta;
use crate::id::{AlbumId, TrackId};
use crate::model::{Album, Track};
use std::collections::HashMap;

/// Every track in the library, ready to query.
#[derive(Debug, Clone, Default)]
pub struct LibraryIndex {
    tracks: Vec<Track>,
    positions: HashMap<TrackId, usize>,
    /// Albums by id, so anything needing one -- the artwork cache, a pane
    /// heading -- can have it without a query. A frontend asks for these while
    /// drawing, and drawing happens every frame.
    albums: HashMap<AlbumId, Album>,
}

impl LibraryIndex {
    /// Reads the whole library out of storage.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if storage fails.
    pub fn load<R: TrackRepository + AlbumRepository>(repository: &R) -> Result<Self, CoreError> {
        Ok(Self::from_parts(
            repository.all_tracks()?,
            repository.all_albums()?,
        ))
    }

    #[must_use]
    pub fn from_parts(tracks: Vec<Track>, albums: Vec<Album>) -> Self {
        let positions = tracks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
        let albums = albums.into_iter().map(|a| (a.id, a)).collect();
        Self {
            tracks,
            positions,
            albums,
        }
    }

    #[must_use]
    pub fn album(&self, id: AlbumId) -> Option<&Album> {
        self.albums.get(&id)
    }

    #[must_use]
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    #[must_use]
    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.get(*self.positions.get(&id)?)
    }

    /// Counts one play against the in-memory copy (§6.8).
    ///
    /// Storage is the source of truth and is written separately; this keeps the
    /// index agreeing with it without reloading the library, which is what lets
    /// the number change in the table as the track plays.
    pub fn record_play(&mut self, id: TrackId) {
        if let Some(&position) = self.positions.get(&id)
            && let Some(track) = self.tracks.get_mut(position)
        {
            track.play_count = track.play_count.saturating_add(1);
        }
    }

    /// Resolves ids to rows, preserving the caller's order and skipping ids
    /// that are no longer present.
    ///
    /// Order matters for the same reason it does in the repository: a playback
    /// snapshot is an ordered list, and silently returning it in another order
    /// is the failure §6.2 exists to prevent.
    #[must_use]
    pub fn rows(&self, ids: &[TrackId]) -> Vec<&Track> {
        ids.iter().filter_map(|id| self.track(*id)).collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Brings the index back in line with storage after a scan.
    ///
    /// Takes the delta as a hint about *what* changed but re-reads the affected
    /// rows rather than trusting a copy: the scanner has already written them,
    /// and storage is the source of truth. [`LibraryDelta::Invalidated`]
    /// reloads everything, which is what it is for.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if storage fails.
    pub fn apply<R: TrackRepository + AlbumRepository>(
        &mut self,
        repository: &R,
        delta: &LibraryDelta,
    ) -> Result<(), CoreError> {
        match delta {
            LibraryDelta::Invalidated => *self = Self::load(repository)?,
            LibraryDelta::TracksRemoved(ids) => {
                let removed: std::collections::HashSet<TrackId> = ids.iter().copied().collect();
                self.tracks.retain(|t| !removed.contains(&t.id));
                self.reindex();
            }
            LibraryDelta::TracksAdded(ids)
            | LibraryDelta::TracksChanged(ids)
            | LibraryDelta::AvailabilityChanged(ids) => {
                for id in ids {
                    match repository.track(*id)? {
                        Some(track) => self.upsert(track),
                        // Gone from storage entirely: drop it here too.
                        None => {
                            if let Some(position) = self.positions.get(id).copied() {
                                self.tracks.remove(position);
                                self.reindex();
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn upsert(&mut self, track: Track) {
        match self.positions.get(&track.id).copied() {
            Some(position) => {
                if let Some(slot) = self.tracks.get_mut(position) {
                    *slot = track;
                }
            }
            None => {
                self.positions.insert(track.id, self.tracks.len());
                self.tracks.push(track);
            }
        }
    }

    fn reindex(&mut self) {
        self.positions = self
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id, i))
            .collect();
    }
}
