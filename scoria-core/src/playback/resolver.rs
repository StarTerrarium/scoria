//! Turning track ids into something the backend can open.
//!
//! This is where §4.3's relative paths become absolute, and it is the only
//! place that happens on the playback path.

use crate::error::CoreError;
use crate::id::TrackId;
use crate::playback::backend::TrackRef;
use crate::playback::engine::TrackResolver;
use crate::query::index::LibraryIndex;
use std::path::{Path, PathBuf};

/// Resolves against a snapshot of the library and a root directory.
///
/// Holds its own copy of the index rather than borrowing one, because the audio
/// thread outlives any particular query and §6.1 requires that browsing --
/// including a rescan landing mid-album -- never disturbs playback. When the
/// library changes, the session builds a new resolver and sends it across.
#[derive(Debug, Clone)]
pub struct IndexResolver {
    index: LibraryIndex,
    root: PathBuf,
}

impl IndexResolver {
    #[must_use]
    pub fn new(index: LibraryIndex, root: impl Into<PathBuf>) -> Self {
        Self {
            index,
            root: root.into(),
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl TrackResolver for IndexResolver {
    fn resolve(&self, id: TrackId) -> Result<TrackRef, CoreError> {
        let track = self
            .index
            .track(id)
            .ok_or_else(|| CoreError::NoSuchTrack(id.get()))?;
        // Unavailable tracks resolve to an error, which is what makes §6.4's
        // skip-on-advance rule work without the engine knowing about
        // availability at all. It says which file, because §7.4 keeps the row
        // visible precisely so someone can go and look for it.
        if !track.available {
            return Err(CoreError::FileMissing {
                title: track.title.clone(),
            });
        }
        Ok(TrackRef {
            id: track.id,
            path: track.path.to_absolute(&self.root),
            duration: track.duration,
            replay_gain: track.replay_gain,
        })
    }
}
