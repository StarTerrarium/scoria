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

    /// The playback path failed: no output device, a file that will not
    /// decode, a stream that will not seek, or the audio thread itself.
    ///
    /// These, and the three below, used to be [`Self::Database`] -- so a
    /// missing sound card reported itself as "database error: cannot open
    /// audio output", which is a sentence that sends someone to look in
    /// entirely the wrong place. They render as their message alone, because
    /// each already says what failed and where.
    #[error("{0}")]
    Audio(String),

    /// A file or directory outside the database could not be read or written.
    /// The artwork cache, in practice.
    #[error("{0}")]
    Io(String),

    /// The desktop's media controls are unavailable (§12). Normal on a
    /// machine with no session bus, and never a reason to fail to start.
    #[error("{0}")]
    Desktop(String),

    /// Watching the library, or the thread that scans it, failed.
    #[error("{0}")]
    Scan(String),

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_that_is_not_the_database_does_not_claim_to_be() {
        // These were all CoreError::Database, whose Display prefixes
        // "database error: ". A missing sound card therefore reported itself
        // as a database problem, which sends someone to look in the wrong
        // place -- and Event::Error is what the status bar renders.
        for error in [
            CoreError::Audio("cannot open audio output: no such device".to_owned()),
            CoreError::Io("cannot write /home/someone/.cache/scoria/artwork: disk full".to_owned()),
            CoreError::Desktop("cannot reach the desktop controls".to_owned()),
            CoreError::Scan("cannot watch /home/someone/Music: permission denied".to_owned()),
        ] {
            let rendered = error.to_string();
            assert!(
                !rendered.contains("database"),
                "{error:?} renders as {rendered:?}"
            );
            assert!(!rendered.is_empty());
        }
    }

    #[test]
    fn a_database_failure_still_says_so() {
        assert!(
            CoreError::Database("disk I/O error".to_owned())
                .to_string()
                .starts_with("database error:")
        );
    }

    #[test]
    fn every_error_survives_the_seam() {
        // Event::Error carries these to a frontend, so they have to round-trip.
        for error in [
            CoreError::Audio("a".to_owned()),
            CoreError::Io("b".to_owned()),
            CoreError::Desktop("c".to_owned()),
            CoreError::Scan("d".to_owned()),
            CoreError::NoLibraryRoot,
            CoreError::FileMissing {
                title: "花冷え。".to_owned(),
            },
        ] {
            let json = serde_json::to_string(&error).unwrap_or_default();
            assert_eq!(
                serde_json::from_str::<CoreError>(&json).ok(),
                Some(error.clone()),
                "{error:?} did not survive a round trip"
            );
        }
    }
}
