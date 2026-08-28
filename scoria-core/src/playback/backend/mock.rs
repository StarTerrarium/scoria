//! A backend that records what it was asked to do.
//!
//! Not `#[cfg(test)]`: the engine's tests live in an integration test, which
//! compiles against the crate as an external consumer, and a mock behind
//! `cfg(test)` would be invisible there. It is also genuinely useful to anyone
//! embedding the core.

use super::{AudioBackend, TrackRef};
use crate::error::CoreError;
use crate::id::TrackId;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// One thing the engine asked the backend to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    Play(TrackId),
    Preload(TrackId),
    ClearPreload,
    Pause,
    Resume,
    Stop,
    Seek(Duration),
    /// The ReplayGain correction applied, in dB.
    Gain(f32),
    Volume(f32),
}

/// Shared state, so a test can inspect and steer the backend after handing it
/// to the engine thread.
#[derive(Debug, Default)]
struct State {
    calls: Vec<Call>,
    /// What the player holds, oldest first, mirroring the real backend.
    queued: Vec<TrackRef>,
    position: Duration,
    /// Tracks whose files "do not exist", to exercise §6.4's skip rule.
    unplayable: Vec<TrackId>,
}

/// A handle a test keeps after giving the backend away.
#[derive(Debug, Clone, Default)]
pub struct MockControl {
    state: Arc<Mutex<State>>,
}

impl MockControl {
    /// Locks the shared state.
    ///
    /// A poisoned lock means a test thread panicked while holding it. Reading
    /// through the poison is right here: the data is a plain record of calls,
    /// and the panic that poisoned it is the failure worth reporting, not this.
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Everything the engine has asked for, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<Call> {
        self.lock().calls.clone()
    }

    /// Just the tracks that were started, in order.
    #[must_use]
    pub fn played(&self) -> Vec<TrackId> {
        self.lock()
            .calls
            .iter()
            .filter_map(|c| match c {
                Call::Play(id) => Some(*id),
                _ => None,
            })
            .collect()
    }

    /// The gain applied to the most recently started track.
    #[must_use]
    pub fn last_gain(&self) -> Option<f32> {
        self.lock().calls.iter().rev().find_map(|c| match c {
            Call::Gain(db) => Some(*db),
            _ => None,
        })
    }

    #[must_use]
    pub fn now_playing(&self) -> Option<TrackId> {
        self.lock().queued.first().map(|t| t.id)
    }

    /// The track queued behind the current one, if any.
    #[must_use]
    pub fn preloaded(&self) -> Option<TrackId> {
        self.lock().queued.get(1).map(|t| t.id)
    }

    /// Everything the player is holding, oldest first.
    #[must_use]
    pub fn queue(&self) -> Vec<TrackId> {
        self.lock().queued.iter().map(|t| t.id).collect()
    }

    /// Makes a track fail to open, standing in for a file that has gone.
    pub fn set_unplayable(&self, ids: &[TrackId]) {
        self.lock().unplayable = ids.to_vec();
    }

    /// Pretends the current track ran out.
    ///
    /// If another track was queued behind it, the mock flows into it exactly as
    /// the real backend does -- which is what makes a gapless transition
    /// observable in a test: no second `Play` call appears.
    pub fn finish_track(&self) {
        let mut state = self.lock();
        if !state.queued.is_empty() {
            state.queued.remove(0);
            state.position = Duration::ZERO;
        }
    }

    pub fn set_position(&self, position: Duration) {
        self.lock().position = position;
    }

    pub fn clear_calls(&self) {
        self.lock().calls.clear();
    }
}

/// An [`AudioBackend`] that plays nothing.
#[derive(Debug, Default)]
pub struct MockBackend {
    control: MockControl,
}

impl MockBackend {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A handle for inspecting and steering this backend once it has been
    /// handed to the engine.
    #[must_use]
    pub fn control(&self) -> MockControl {
        self.control.clone()
    }
}

impl AudioBackend for MockBackend {
    fn play(&mut self, track: &TrackRef, gain_db: f32) -> Result<(), CoreError> {
        let mut state = self.control.lock();
        if state.unplayable.contains(&track.id) {
            return Err(CoreError::Database(format!(
                "mock: {} is unplayable",
                track.id
            )));
        }
        state.calls.push(Call::Gain(gain_db));
        state.calls.push(Call::Play(track.id));
        state.queued.clear();
        state.queued.push(track.clone());
        state.position = Duration::ZERO;
        Ok(())
    }

    fn preload_next(&mut self, track: &TrackRef, gain_db: f32) -> Result<(), CoreError> {
        let mut state = self.control.lock();
        if state.unplayable.contains(&track.id) {
            return Err(CoreError::Database(format!(
                "mock: {} is unplayable",
                track.id
            )));
        }
        state.calls.push(Call::Gain(gain_db));
        state.calls.push(Call::Preload(track.id));
        state.queued.push(track.clone());
        Ok(())
    }

    fn clear_preload(&mut self) {
        let mut state = self.control.lock();
        state.calls.push(Call::ClearPreload);
        state.queued.truncate(1);
    }

    fn pause(&mut self) {
        self.control.lock().calls.push(Call::Pause);
    }

    fn resume(&mut self) {
        self.control.lock().calls.push(Call::Resume);
    }

    fn stop(&mut self) {
        let mut state = self.control.lock();
        state.calls.push(Call::Stop);
        state.queued.clear();
        state.position = Duration::ZERO;
    }

    fn seek(&mut self, position: Duration) -> Result<(), CoreError> {
        let mut state = self.control.lock();
        state.calls.push(Call::Seek(position));
        state.position = position;
        Ok(())
    }

    fn set_volume(&mut self, linear: f32) {
        self.control.lock().calls.push(Call::Volume(linear));
    }

    fn position(&self) -> Duration {
        self.control.lock().position
    }

    fn current(&mut self) -> Option<TrackId> {
        self.control.lock().queued.first().map(|t| t.id)
    }
}
