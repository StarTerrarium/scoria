//! Playback: the snapshot, the audio backend, and the thread that owns it.

pub mod backend;
pub mod engine;
pub mod resolver;
pub mod rng;
pub mod source;

pub use backend::{AudioBackend, MockBackend, RodioBackend, TrackRef};
pub use engine::{EngineCommand, PlaybackHandle, TrackResolver, spawn};
pub use resolver::IndexResolver;
pub use rng::Rng;
pub use source::{Origin, PlaybackSource, RepeatMode};
