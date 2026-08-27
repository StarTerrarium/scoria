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

    /// A playback snapshot was asked for a position it does not contain.
    #[error("position {position} is outside a snapshot of {len} tracks")]
    PositionOutOfRange { position: usize, len: usize },

    /// The snapshot is empty, so there is nothing to play.
    #[error("playback source is empty")]
    EmptySource,
}
