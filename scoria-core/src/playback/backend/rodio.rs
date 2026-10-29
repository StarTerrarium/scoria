//! The rodio backend: what actually makes sound, gaplessly.
//!
//! Note on the API. `rodio` 0.22 is not the `rodio` most examples describe --
//! there is no `OutputStream::try_default` and no `Sink`. Output opens with
//! `DeviceSinkBuilder`, and `Player` is what `Sink` used to be. There is also a
//! `speakers` module that looks like the natural entry point and is gated
//! behind the `experimental` feature, so it is deliberately unused. This was
//! written against the 0.22 source rather than from memory, for the same reason
//! `CLAUDE.md` says to check `docs.rs/iced/0.14`.
//!
//! ## How gapless works here
//!
//! rodio's queue moves between sources at a span boundary and pads only enough
//! to complete a frame -- at most `channels - 1` samples. So appending the next
//! track before the current one drains *is* gapless; the gap in a naive player
//! comes from tearing down the sink and constructing a new decoder at the
//! boundary, which is what [`AudioBackend::play`] does and
//! [`AudioBackend::preload_next`] avoids.
//!
//! Two consequences shaped this module. `Player`'s volume applies to every
//! queued source, so per-track ReplayGain cannot go through it -- each source
//! carries its own gain via `amplify` instead. And `Player::clear` skips the
//! current track as well as queued ones, so retracting a preload needs
//! [`Cancellable`].

use super::cancellable::{Cancel, Cancellable};
use super::{AudioBackend, TrackRef};
use crate::error::CoreError;
use crate::id::TrackId;
use rodio::source::Source;
use rodio::stream::DeviceSinkBuilder;
use rodio::{MixerDeviceSink, Player};
use std::collections::VecDeque;
use std::time::Duration;

/// A track handed to the player, and the switch that can retract it.
struct Queued {
    id: TrackId,
    cancel: Cancel,
}

/// Playback through the system's default output.
pub struct RodioBackend {
    /// Holds the device open. Dropping it silences everything, so it is kept
    /// even though nothing reads it after construction.
    _device: MixerDeviceSink,
    player: Player,
    /// What we handed the player, oldest first. Mirrors its internal queue so
    /// the backend can tell which track is being heard.
    queued: VecDeque<Queued>,
}

impl RodioBackend {
    /// Opens the default audio device.
    ///
    /// cpal tries PipeWire, then PulseAudio, then ALSA (§12).
    ///
    /// # Errors
    ///
    /// [`CoreError::Audio`] if no output device is available or it cannot be
    /// configured. A machine with no sound card is a normal thing to run into,
    /// not a reason to abort.
    pub fn open() -> Result<Self, CoreError> {
        let mut device = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| CoreError::Audio(format!("cannot open audio output: {e}")))?;
        // rodio prints a paragraph to stderr when the sink drops. Shutting down
        // is not an event a library should narrate to the user's terminal.
        device.log_on_drop(false);

        let player = Player::connect_new(device.mixer());
        Ok(Self {
            _device: device,
            player,
            queued: VecDeque::new(),
        })
    }

    /// Opens a file and wraps it with its own gain and a cancel switch.
    fn append(&mut self, track: &TrackRef, gain_db: f32) -> Result<(), CoreError> {
        let decoder = crate::decode::open(&track.path)?;

        // Gain rides on the source rather than on the player, because the
        // player's volume would also apply to whatever else is queued.
        let cancel = Cancel::new();
        let source = Cancellable::new(decoder.amplify(to_amplitude(gain_db)), cancel.clone());

        self.player.append(source);
        self.queued.push_back(Queued {
            id: track.id,
            cancel,
        });
        Ok(())
    }

    /// Drops the record of sources the player has already finished.
    ///
    /// `Player::len` counts what it still holds, so anything our queue has
    /// beyond that has been played out. This is how a transition the backend
    /// made on its own becomes visible to the engine.
    fn reconcile(&mut self) {
        while self.queued.len() > self.player.len() {
            self.queued.pop_front();
        }
    }
}

/// Decibels to a linear amplitude factor. Zero dB is unity, which is what an
/// untagged track gets (§6.7).
fn to_amplitude(db: f32) -> f32 {
    if db.is_finite() {
        10.0_f32.powf(db / 20.0)
    } else {
        1.0
    }
}

impl AudioBackend for RodioBackend {
    fn play(&mut self, track: &TrackRef, gain_db: f32) -> Result<(), CoreError> {
        self.player.clear();
        self.queued.clear();
        self.append(track, gain_db)?;
        self.player.play();
        Ok(())
    }

    fn preload_next(&mut self, track: &TrackRef, gain_db: f32) -> Result<(), CoreError> {
        self.append(track, gain_db)
    }

    fn clear_preload(&mut self) {
        // Everything after the first entry is queued but not playing. Cancelled
        // sources yield nothing and are skipped instantly, which is the only
        // way to un-append without disturbing the current track.
        for queued in self.queued.iter().skip(1) {
            queued.cancel.cancel();
        }
        self.queued.truncate(1);
    }

    fn pause(&mut self) {
        self.player.pause();
    }

    fn resume(&mut self) {
        self.player.play();
    }

    fn stop(&mut self) {
        self.player.stop();
        self.player.clear();
        self.queued.clear();
    }

    fn seek(&mut self, position: Duration) -> Result<(), CoreError> {
        self.player
            .try_seek(position)
            .map_err(|e| CoreError::Audio(format!("cannot seek: {e}")))
    }

    fn set_volume(&mut self, linear: f32) {
        self.player.set_volume(linear.clamp(0.0, 1.0));
    }

    fn position(&self) -> Duration {
        self.player.get_pos()
    }

    fn current(&mut self) -> Option<TrackId> {
        self.reconcile();
        self.queued.front().map(|q| q.id)
    }
}
