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
    playing: Option<TrackRef>,
    preloaded: Option<TrackRef>,
    position: Duration,
    finished: bool,
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
        self.lock().playing.as_ref().map(|t| t.id)
    }

    #[must_use]
    pub fn preloaded(&self) -> Option<TrackId> {
        self.lock().preloaded.as_ref().map(|t| t.id)
    }

    /// Makes a track fail to open, standing in for a file that has gone.
    pub fn set_unplayable(&self, ids: &[TrackId]) {
        self.lock().unplayable = ids.to_vec();
    }

    /// Pretends the current track ran out.
    pub fn finish_track(&self) {
        self.lock().finished = true;
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
    fn play(&mut self, track: &TrackRef) -> Result<(), CoreError> {
        let mut state = self.control.lock();
        if state.unplayable.contains(&track.id) {
            return Err(CoreError::Database(format!(
                "mock: {} is unplayable",
                track.id
            )));
        }
        state.calls.push(Call::Play(track.id));
        state.playing = Some(track.clone());
        state.preloaded = None;
        state.position = Duration::ZERO;
        state.finished = false;
        Ok(())
    }

    fn preload_next(&mut self, track: &TrackRef) -> Result<(), CoreError> {
        let mut state = self.control.lock();
        if state.unplayable.contains(&track.id) {
            return Err(CoreError::Database(format!(
                "mock: {} is unplayable",
                track.id
            )));
        }
        state.calls.push(Call::Preload(track.id));
        state.preloaded = Some(track.clone());
        Ok(())
    }

    fn clear_preload(&mut self) {
        let mut state = self.control.lock();
        state.calls.push(Call::ClearPreload);
        state.preloaded = None;
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
        state.playing = None;
        state.preloaded = None;
        state.position = Duration::ZERO;
        state.finished = false;
    }

    fn seek(&mut self, position: Duration) -> Result<(), CoreError> {
        let mut state = self.control.lock();
        state.calls.push(Call::Seek(position));
        state.position = position;
        Ok(())
    }

    fn set_gain(&mut self, db: f32) {
        self.control.lock().calls.push(Call::Gain(db));
    }

    fn set_volume(&mut self, linear: f32) {
        self.control.lock().calls.push(Call::Volume(linear));
    }

    fn position(&self) -> Duration {
        self.control.lock().position
    }

    fn finished(&self) -> bool {
        self.control.lock().finished
    }
}
