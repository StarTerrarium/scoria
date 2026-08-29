//! Errors crossing the seam.
//!
//! `CoreError` is carried by `Event::Error`, so it is serialisable and must stay
//! free of types the UI cannot reconstruct.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Anything the core can fail at and report to a frontend.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
pub enum CoreError {
    /// A path that must be relative to the library root was not.
    #[error("path must be relative to the library root: {path}")]
    NotRelative { path: String },

    /// A path escaped the library root via `..`.
    #[error("path escapes the library root: {path}")]
    EscapesRoot { path: String },

    /// A track id referred to something no longer in the library.
    #[error("no such track: {0}")]
    NoSuchTrack(u64),

    /// The track is in the library but its file is not where it was (§7.4).
    ///
    /// Distinct from [`Self::NoSuchTrack`] because they need different words:
    /// one is a stale id, which nobody can act on, and the other is a file
    /// somebody moved, which they can.
    #[error("{title}: the file is missing")]
    FileMissing { title: String },

    /// A playback snapshot was asked for a position it does not contain.
    #[error("position {position} is outside a snapshot of {len} tracks")]
    PositionOutOfRange { position: usize, len: usize },

    /// The snapshot is empty, so there is nothing to play.
    #[error("playback source is empty")]
    EmptySource,

    /// Storage failed. Carries a message rather than the driver's error type,
    /// because this crosses the seam and must stay serialisable.
    #[error("database error: {0}")]
    Database(String),

    /// The database holds something the model cannot represent.
    #[error("corrupt library data: {0}")]
    CorruptData(String),

    #[error("configuration error: {0}")]
    Config(String),

    /// No library root has been chosen yet, so there is nothing to scan or
    /// resolve paths against. The first-run picker exists to fix this (§5.1).
    #[error("no library root configured")]
    NoLibraryRoot,
}
