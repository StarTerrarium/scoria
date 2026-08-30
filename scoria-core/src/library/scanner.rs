//! Walking the library root and indexing what is there.
//!
//! `DESIGN.md` §5.2. The scan itself is synchronous: it is a filesystem walk
//! and a pile of tag parsing, which is blocking work whichever way it is
//! dressed up. §3.3 puts scanning on tokio, and the way to honour that without
//! pretending blocking work is async is to run this from `spawn_blocking` --
//! see [`Scanner::scan`]'s note. Keeping the logic sync also means it is
//! testable without a runtime.

use crate::db::repository::{AlbumRepository, NewAlbum, NewTrack, TrackRepository};
use crate::error::CoreError;
use crate::event::LibraryDelta;
use crate::id::{AlbumId, TrackId};
use crate::library::artwork::{self, ArtworkCache, ArtworkKey};
use crate::library::identity::AlbumIdentity;
use crate::library::tags::{self, TrackTags};
use crate::model::Timestamp;
use crate::path::RelativePath;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Tracks are written in batches of this size, so a scan of a large library is
/// a few hundred transactions rather than one enormous one or one per file.
const BATCH: usize = 256;

/// How far along a scan is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanProgress {
    pub seen: usize,
    /// Known only after the walk finishes, which is why `Event::ScanProgress`
    /// carries an `Option`.
    pub total: Option<usize>,
}

/// What a scan did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScanOutcome {
    pub added: Vec<TrackId>,
    /// Tracks already indexed whose stored row genuinely differs from what is
    /// on disk now -- a retag, in practice.
    ///
    /// Not "seen again": a scan reads every file's tags on every pass, so
    /// almost every track is looked at and almost none of them change.
    /// Storage decides, by leaving a row that already matches alone.
    pub updated: Vec<TrackId>,
    /// Tracks whose files no longer resolve. Marked, never deleted (§7.4).
    pub now_unavailable: Vec<TrackId>,
    /// Tracks whose files came back.
    pub now_available: Vec<TrackId>,
    pub albums_removed: usize,
    /// Cached images dropped because their album is gone.
    pub artwork_pruned: usize,
    /// Files that could not be read, with the reason. A scan reports these
    /// rather than failing: one corrupt file should not cost you the library.
    pub failed: Vec<(PathBuf, String)>,
}

impl ScanOutcome {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.updated.is_empty()
            && self.now_unavailable.is_empty()
            && self.now_available.is_empty()
    }

    /// The deltas a frontend needs to refresh itself.
    ///
    /// Resolves the granularity question §14 left open. The scanner knows
    /// exactly which tracks changed, so it says so, and the UI can avoid
    /// re-running a query it does not need to. `Invalidated` remains available
    /// for callers that cannot be precise, but the scanner never needs it.
    #[must_use]
    pub fn deltas(&self) -> Vec<LibraryDelta> {
        let mut deltas = Vec::new();
        if !self.added.is_empty() {
            deltas.push(LibraryDelta::TracksAdded(self.added.clone()));
        }
        // A retag reaches storage on the next scan whether or not anyone is
        // told, and for a long time nobody was: `updated` produced no delta,
        // so the new title sat in the database while the table went on showing
        // the old one until something else prompted a re-query.
        if !self.updated.is_empty() {
            deltas.push(LibraryDelta::TracksChanged(self.updated.clone()));
        }
        let availability: Vec<TrackId> = self
            .now_unavailable
            .iter()
            .chain(&self.now_available)
            .copied()
            .collect();
        if !availability.is_empty() {
            deltas.push(LibraryDelta::AvailabilityChanged(availability));
        }
        deltas
    }
}

/// Indexes a library root into a repository.
pub struct Scanner<'a> {
    root: PathBuf,
    artwork: Option<&'a ArtworkCache>,
}

impl<'a> Scanner<'a> {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            artwork: None,
        }
    }

    /// Extract and cache album artwork as the scan goes (§5.3).
    #[must_use]
    pub fn with_artwork(mut self, cache: &'a ArtworkCache) -> Self {
        self.artwork = Some(cache);
        self
    }

    /// Walks the root and brings the repository up to date with it.
    ///
    /// This blocks. Call it from `tokio::task::spawn_blocking` so it does not
    /// stall the runtime; it is deliberately not `async fn`, because nothing it
    /// does would yield and an async wrapper would only obscure that.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if storage fails. Unreadable *files* are
    /// collected into [`ScanOutcome::failed`] rather than aborting the scan.
    pub fn scan<R>(
        &self,
        repository: &mut R,
        progress: &mut dyn FnMut(ScanProgress),
    ) -> Result<ScanOutcome, CoreError>
    where
        R: TrackRepository + AlbumRepository,
    {
        let files = self.walk();
        let total = Some(files.len());
        progress(ScanProgress { seen: 0, total });

        let known: HashMap<RelativePath, TrackId> = repository
            .all_track_paths()?
            .into_iter()
            .map(|(id, path)| (path, id))
            .collect();
        let previously_unavailable = repository.unavailable_track_ids()?;

        let mut outcome = ScanOutcome::default();
        let mut albums: HashMap<String, AlbumId> = HashMap::new();
        let mut artwork_keys: HashSet<ArtworkKey> = HashSet::new();
        let mut artwork_done: HashSet<AlbumId> = HashSet::new();
        let mut seen_paths: HashSet<RelativePath> = HashSet::new();
        let mut batch: Vec<(NewTrack, bool)> = Vec::with_capacity(BATCH);
        let now = unix_now();

        for (index, absolute) in files.iter().enumerate() {
            progress(ScanProgress { seen: index, total });

            let relative = match RelativePath::from_absolute(absolute, &self.root) {
                Ok(relative) => relative,
                Err(e) => {
                    outcome.failed.push((absolute.clone(), e.to_string()));
                    continue;
                }
            };

            let tags = match tags::read(absolute) {
                Ok(tags) => tags,
                Err(e) => {
                    outcome.failed.push((absolute.clone(), e.to_string()));
                    continue;
                }
            };

            // Resolved before the album row, because the artwork is filed under
            // the identity rather than under whatever rowid the album lands on.
            let identity = AlbumIdentity::resolve(
                tags.musicbrainz_album_id.as_deref(),
                tags.album_artist.as_deref(),
                tags.album.as_deref(),
                tags.date,
                relative.parent(),
            );
            let album_key = ArtworkKey::for_album(&identity);
            artwork_keys.insert(album_key.clone());

            let album_id =
                match self.resolve_album(repository, &mut albums, &relative, &tags, identity) {
                    Ok(id) => id,
                    Err(e) => {
                        outcome.failed.push((absolute.clone(), e.to_string()));
                        continue;
                    }
                };

            if let Some(cache) = self.artwork
                && artwork_done.insert(album_id)
                && !cache.has(&album_key)
                && let Some(found) = artwork::find(absolute)
            {
                // Artwork is a nicety; failing to cache it must not cost the
                // track that carried it.
                if let Err(e) = cache.store(&album_key, &found) {
                    outcome.failed.push((absolute.clone(), e.to_string()));
                }
            }

            let is_new = !known.contains_key(&relative);
            seen_paths.insert(relative.clone());
            batch.push((
                self.build_track(album_id, relative, &tags, absolute, now),
                is_new,
            ));

            if batch.len() >= BATCH {
                self.flush(repository, &mut batch, &mut outcome)?;
            }
        }
        self.flush(repository, &mut batch, &mut outcome)?;

        // Anything indexed but not seen on disk has gone missing. §7.4: mark,
        // never delete. A struck-through row states what will happen; a row
        // that silently disappears makes users doubt the software.
        let missing: Vec<TrackId> = known
            .iter()
            .filter(|(path, _)| !seen_paths.contains(*path))
            .map(|(_, id)| *id)
            .collect();
        if !missing.is_empty() {
            repository.set_available(&missing, false)?;
            outcome.now_unavailable = missing;
        }

        // Files that were missing and are back. The upsert already set them
        // available -- and because the match test includes `available`, a file
        // that returned with tags nobody touched is still written and still
        // reported. This only turns that into a list.
        //
        // Through sets rather than Vec::contains: these are three lists that
        // can each be the length of the library, and a linear scan inside a
        // filter over another of them is quadratic on exactly the rescan that
        // finds a reconnected drive.
        let written: HashSet<TrackId> = outcome
            .added
            .iter()
            .chain(&outcome.updated)
            .copied()
            .collect();
        let gone: HashSet<TrackId> = outcome.now_unavailable.iter().copied().collect();
        outcome.now_available = previously_unavailable
            .into_iter()
            .filter(|id| written.contains(id) && !gone.contains(id))
            .collect();

        outcome.albums_removed = repository.remove_empty_albums()?;

        // The cache mirrors the library: art for albums that are gone goes too,
        // and so does anything left behind by an older naming scheme.
        if let Some(cache) = self.artwork {
            outcome.artwork_pruned = cache.retain(&artwork_keys).unwrap_or(0);
        }
        progress(ScanProgress {
            seen: files.len(),
            total,
        });
        Ok(outcome)
    }

    fn flush<R: TrackRepository>(
        &self,
        repository: &mut R,
        batch: &mut Vec<(NewTrack, bool)>,
        outcome: &mut ScanOutcome,
    ) -> Result<(), CoreError> {
        if batch.is_empty() {
            return Ok(());
        }
        let tracks: Vec<NewTrack> = batch.iter().map(|(track, _)| track.clone()).collect();
        let written = repository.upsert_tracks(&tracks)?;

        // Only what actually changed comes back, so the results cannot be
        // lined up against the batch by position -- an unchanged row produces
        // nothing at all. Matched on path, which is what both sides are keyed
        // on anyway.
        let new_paths: HashSet<&RelativePath> = batch
            .iter()
            .filter(|(_, is_new)| *is_new)
            .map(|(track, _)| &track.path)
            .collect();
        for row in written {
            if new_paths.contains(&row.path) {
                outcome.added.push(row.id);
            } else {
                outcome.updated.push(row.id);
            }
        }
        batch.clear();
        Ok(())
    }

    fn resolve_album<R>(
        &self,
        repository: &mut R,
        cache: &mut HashMap<String, AlbumId>,
        relative: &RelativePath,
        tags: &TrackTags,
        identity: AlbumIdentity,
    ) -> Result<AlbumId, CoreError>
    where
        R: AlbumRepository,
    {
        // Every track of an album resolves to the same identity, so caching by
        // key turns one database round trip per file into one per album.
        let key = identity.key();
        if let Some(id) = cache.get(&key) {
            return Ok(*id);
        }

        let id = repository.upsert_album(&NewAlbum {
            title: tags
                .album
                .clone()
                .unwrap_or_else(|| self.fallback_album(relative)),
            album_artist: tags.album_artist.clone().unwrap_or_else(unknown_artist),
            album_artist_sort: tags.album_artist_sort.clone(),
            date: tags.date,
            identity,
        })?;
        cache.insert(key, id);
        Ok(id)
    }

    fn build_track(
        &self,
        album_id: AlbumId,
        path: RelativePath,
        tags: &TrackTags,
        absolute: &Path,
        now: i64,
    ) -> NewTrack {
        // A track with no title tag still has to be findable and playable, so
        // it is labelled with its filename rather than left blank. This is the
        // one place a value is filled in rather than left absent, and it is a
        // label rather than a claim about the tags.
        let title = tags.title.clone().unwrap_or_else(|| {
            absolute
                .file_stem()
                .map_or_else(|| path.to_string(), |s| s.to_string_lossy().into_owned())
        });

        NewTrack {
            album_id,
            path,
            title,
            artist: tags.artist.clone().unwrap_or_else(unknown_artist),
            album_artist: tags.album_artist.clone().unwrap_or_else(unknown_artist),
            album_artist_sort: tags.album_artist_sort.clone(),
            album: tags.album.clone().unwrap_or_else(unknown_album),
            date: tags.date,
            disc: tags.disc,
            track_number: tags.track_number,
            duration: tags.duration,
            replay_gain: tags.replay_gain,
            added_at: Timestamp(now),
            genre: tags.genre.clone(),
            composer: tags.composer.clone(),
            audio: tags.audio,
        }
    }

    fn fallback_album(&self, relative: &RelativePath) -> String {
        relative
            .parent()
            .and_then(|p| p.file_name())
            .map_or_else(unknown_album, |n| n.to_string_lossy().into_owned())
    }

    /// Every audio file under the root.
    ///
    /// Iterative rather than recursive, so a pathologically deep tree cannot
    /// overflow the stack, and symlinked directories are not followed -- a
    /// library containing a link back to its own root would otherwise never
    /// finish.
    fn walk(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut stack = vec![self.root.clone()];

        while let Some(directory) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue; // Unreadable directory: skip it, do not fail the scan.
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };

                if kind.is_dir() {
                    stack.push(path);
                } else if kind.is_file() && tags::is_audio_file(&path) {
                    files.push(path);
                }
            }
        }

        // read_dir order is filesystem order, which varies between machines and
        // even between runs. Sorting makes a scan reproducible, which in turn
        // makes ids stable enough for tests to assert on.
        files.sort();
        files
    }
}

fn unknown_artist() -> String {
    "Unknown Artist".to_owned()
}

fn unknown_album() -> String {
    "Unknown Album".to_owned()
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
