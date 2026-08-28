//! The audio backend seam.
//!
//! `DESIGN.md` §3.4 puts a trait here from day one. It costs almost nothing and
//! buys the option to swap in libmpv if pure-Rust gapless on an awkward format
//! becomes a multi-week problem. It also buys something more immediate: every
//! playback rule in §6.4 can be tested against [`MockBackend`] with no audio
//! device, which is what makes the engine's behaviour assertable rather than
//! something you check by listening.

pub mod mock;
pub mod rodio;

use crate::error::CoreError;
use crate::id::TrackId;
use crate::model::ReplayGain;
use std::path::PathBuf;
use std::time::Duration;

pub use mock::MockBackend;
pub use rodio::RodioBackend;

/// Everything the backend needs in order to play something.
///
/// The path is absolute: this is below the layer where §4.3's relative-path
/// rule applies, and resolution against the library root has already happened.
/// `replay_gain` arrives unapplied because which gain to use depends on how
/// playback is ordered (§6.7), and only the engine knows that.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackRef {
    pub id: TrackId,
    pub path: PathBuf,
    pub duration: Duration,
    pub replay_gain: ReplayGain,
}

/// An audio output.
///
/// Implementations must never panic: §3.3 is explicit that a panic on the audio
/// thread is the worst failure this application has, because it leaves a UI
/// showing a playing track and a transport that does nothing. Report through
/// `Result` and let the engine turn it into an `Event::Error`.
pub trait AudioBackend: Send {
    /// Starts `track` immediately, replacing anything playing.
    ///
    /// # Errors
    /// [`CoreError::Database`] if the file cannot be opened or decoded.
    fn play(&mut self, track: &TrackRef) -> Result<(), CoreError>;

    /// Prepares the following track so it can begin without a gap.
    ///
    /// §3.4: appending to a rodio `Sink` is queueing, not gapless -- the next
    /// track's decoder must be initialised and its first packets decoded before
    /// the current track's tail drains. Real gapless lands at M5b; until then an
    /// implementation may treat this as a hint and still produce a small gap.
    ///
    /// # Errors
    /// [`CoreError::Database`] if the file cannot be opened or decoded.
    fn preload_next(&mut self, track: &TrackRef) -> Result<(), CoreError>;

    /// Discards anything preloaded, because what plays next just changed.
    fn clear_preload(&mut self);

    fn pause(&mut self);
    fn resume(&mut self);

    /// Stops and discards position.
    fn stop(&mut self);

    /// # Errors
    /// [`CoreError::Database`] if the backend cannot seek this stream.
    fn seek(&mut self, position: Duration) -> Result<(), CoreError>;

    /// Per-track ReplayGain correction, in decibels. Zero means unity (§6.7).
    fn set_gain(&mut self, db: f32);

    /// User-facing volume as a linear amplitude, 0.0 to 1.0.
    ///
    /// Separate from [`Self::set_gain`] on purpose: one is a correction the
    /// tags asked for and changes per track, the other is what the user set and
    /// does not. A backend with native ReplayGain support can honour both.
    fn set_volume(&mut self, linear: f32);

    /// How far into the current track playback has reached.
    fn position(&self) -> Duration;

    /// Whether the current track has run out.
    fn finished(&self) -> bool;
}
