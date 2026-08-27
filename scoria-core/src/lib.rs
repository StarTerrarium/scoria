//! Library index, scanner, query engine, playback engine and queue.
//!
//! This crate never depends on a UI crate. The frontend talks to it by sending
//! [`Command`] and consuming [`Event`], both serialisable; it never calls in
//! here directly. See `DESIGN.md` §3.2 for why the seam is shaped that way, and
//! `scripts/check-boundaries.sh` for what enforces it.

pub mod command;
pub mod date;
pub mod error;
pub mod event;
pub mod id;
pub mod model;
pub mod path;
pub mod playback;
pub mod query;

#[cfg(test)]
mod test_support;

pub use command::{Command, SourceSpec};
pub use date::Date;
pub use error::CoreError;
pub use event::{Event, LibraryDelta, PlaybackState};
pub use id::{AlbumId, QueryId, TrackId};
pub use model::{Album, GainMode, ReplayGain, Timestamp, Track};
pub use path::RelativePath;
pub use playback::{Origin, PlaybackSource, RepeatMode};
pub use query::Selection;
pub use query::sort::{Direction, SortChain, SortField, SortKey, SortPreset};
