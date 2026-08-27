//! Playback: the snapshot, the audio backend, and the thread that owns it.
//!
//! The engine and backend arrive at M5. What is here now is the snapshot, which
//! is the part every other decision hangs off (`DESIGN.md` §6.2).

pub mod rng;
pub mod source;

pub use rng::Rng;
pub use source::{Origin, PlaybackSource, RepeatMode};
