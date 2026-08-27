//! Commands travelling from a frontend into the core.
//!
//! One half of the seam (`DESIGN.md` §3.2). Serialisable from the start: 1.0 is
//! not a daemon, but the seam is daemon-shaped, so putting a socket in the
//! middle later is mechanical rather than a rewrite. It is also what makes the
//! TUI cheap.

use crate::id::{AlbumId, QueryId, TrackId};
use crate::playback::RepeatMode;
use crate::query::Selection;
use crate::query::sort::SortChain;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// What to snapshot when playback starts.
///
/// Resolves the gap §14 recorded: `Command::PlayFrom` referred to a
/// `SourceSpec` that was never defined.
///
/// The variants correspond to the play-behaviour setting in §6.3 plus the
/// album shortcut in §6.4. Note that `Shown` carries the track list explicitly
/// rather than a `Selection`: the view already has the answer, and handing it
/// over means the core never re-runs a query to decide what plays, which is
/// invariant 5 stated as a type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceSpec {
    /// "Play the whole library": ignores filters, respects the given sort,
    /// since sort is something the user just deliberately set (§6.3).
    Library { sort: SortChain },
    /// "Play what's shown", the default: exactly these tracks, in this order.
    Shown {
        tracks: Vec<TrackId>,
        label: Option<String>,
    },
    /// One album, from a group header or the album pane (§6.4).
    Album(AlbumId),
}

/// Everything a frontend can ask the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// Play one track, snapshotting just it.
    Play(TrackId),
    /// Snapshot `source` and start at `at`.
    PlayFrom {
        source: SourceSpec,
        at: TrackId,
    },
    Pause,
    Resume,
    /// Stop and discard the current position.
    ///
    /// Resolves the other gap §14 recorded. It exists because MPRIS defines a
    /// Stop verb that desktop controls and `playerctl` will send regardless of
    /// whether we want one, and answering it by pausing would be a lie.
    Stop,
    Next,
    Previous,
    Seek(Duration),
    /// Linear amplitude, 0.0 to 1.0. Distinct from ReplayGain, which is applied
    /// per track from tags (§6.7) and is not user-facing.
    SetVolume(f32),
    /// Jump to a row of the current snapshot, from the queue overlay (§9.2).
    SetPosition(usize),
    SetShuffle(bool),
    SetRepeat(RepeatMode),
    /// Run a query; the answer arrives as `Event::QueryResult` with this id.
    Query(QueryId, Selection),
    Rescan,
    SetRoot(PathBuf),
}
