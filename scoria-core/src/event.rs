//! Events travelling from the core out to a frontend.
//!
//! The other half of the seam (`DESIGN.md` §3.2).

use crate::error::CoreError;
use crate::id::{QueryId, TrackId};
use crate::playback::PlaybackSource;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PlaybackState {
    #[default]
    Stopped,
    Playing,
    Paused,
}

/// What changed in the library.
///
/// Coarse on purpose for now. §14 records the granularity as open until the
/// scanner exists and we know what it can cheaply report; `Invalidated` is the
/// escape hatch that always works, and the specific variants let the UI avoid a
/// full re-query when the scanner can be precise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibraryDelta {
    /// Something changed; re-run outstanding queries.
    Invalidated,
    TracksAdded(Vec<TrackId>),
    /// Nothing emits this yet, and a scan never will: §7.4 marks a file that
    /// has gone rather than deleting its row, which is what
    /// [`Self::AvailabilityChanged`] reports. Kept because the index applies
    /// it correctly and a library rebuilt from scratch is the case that would
    /// want it -- do not reach for it to mean "a file vanished".
    TracksRemoved(Vec<TrackId>),
    /// Files appeared or vanished. Vanished tracks are not deleted -- they are
    /// shown struck through and skipped on advance (§7.4).
    AvailabilityChanged(Vec<TrackId>),
}

/// Everything the core reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Event {
    PlaybackStateChanged(PlaybackState),
    /// `position` indexes the snapshot in its natural order, so the queue
    /// overlay can mark the row directly whether or not shuffle is on.
    TrackChanged {
        track: TrackId,
        position: usize,
    },
    PositionChanged(Duration),
    QueueChanged(PlaybackSource),
    QueryResult(QueryId, Vec<TrackId>),
    ScanProgress {
        seen: usize,
        total: Option<usize>,
    },
    LibraryChanged(LibraryDelta),
    /// A scan stopped, whether or not it found anything.
    ///
    /// `LibraryChanged` is not enough on its own: a scan that finds nothing
    /// changed emits no deltas, so without this a frontend that had started
    /// saying "scanning" would have no way to learn it had finished, and would
    /// keep saying it. Emitted after any deltas, and after `Error` when the
    /// scan failed, so it always terminates the sequence.
    ScanFinished,
    /// A failure the core could not handle itself.
    ///
    /// This is what makes the no-panic policy workable: code that cannot
    /// recover locally reports here instead of dying, and the audio thread
    /// stays alive.
    Error(CoreError),
}
