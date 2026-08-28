//! The rodio backend: what actually makes sound.
//!
//! Note on the API. `rodio` 0.22 is not the `rodio` most examples describe --
//! there is no `OutputStream::try_default` and no `Sink`. Output is opened with
//! `DeviceSinkBuilder`, and `Player` is what `Sink` used to be. There is also a
//! `speakers` module that looks like the natural entry point and is gated
//! behind the `experimental` feature, so it is deliberately not used. This was
//! written against the 0.22 source rather than from memory, for the same reason
//! `CLAUDE.md` says to check `docs.rs/iced/0.14`.

use super::{AudioBackend, TrackRef};
use crate::error::CoreError;
use rodio::stream::DeviceSinkBuilder;
use rodio::{MixerDeviceSink, Player};
use std::fs::File;
use std::io::BufReader;
use std::time::Duration;

/// Playback through the system's default output.
pub struct RodioBackend {
    /// Holds the device open. Dropping it silences everything, so it is kept
    /// even though nothing reads it after construction.
    _device: MixerDeviceSink,
    player: Player,
    /// User volume, linear.
    volume: f32,
    /// Per-track ReplayGain correction, as a linear factor rather than dB, so
    /// the multiplication at the point of use is trivial.
    gain: f32,
}

impl RodioBackend {
    /// Opens the default audio device.
    ///
    /// cpal tries PipeWire, then PulseAudio, then ALSA (§12).
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if no output device is available or it cannot be
    /// configured. A machine with no sound card is a normal thing to run into,
    /// not a reason to abort.
    pub fn open() -> Result<Self, CoreError> {
        let mut device = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| CoreError::Database(format!("cannot open audio output: {e}")))?;
        // rodio prints a paragraph to stderr when the sink drops. Shutting down
        // is not an event a library should narrate to the user's terminal.
        device.log_on_drop(false);

        let player = Player::connect_new(device.mixer());
        Ok(Self {
            _device: device,
            player,
            volume: 1.0,
            gain: 1.0,
        })
    }

    /// Volume and ReplayGain multiply; the backend applies the product.
    fn apply_amplitude(&self) {
        self.player.set_volume(self.volume * self.gain);
    }

    fn decode(track: &TrackRef) -> Result<rodio::Decoder<BufReader<File>>, CoreError> {
        let file = File::open(&track.path).map_err(|e| {
            CoreError::Database(format!("cannot open {}: {e}", track.path.display()))
        })?;
        rodio::Decoder::new(BufReader::new(file)).map_err(|e| {
            CoreError::Database(format!("cannot decode {}: {e}", track.path.display()))
        })
    }
}

impl AudioBackend for RodioBackend {
    fn play(&mut self, track: &TrackRef) -> Result<(), CoreError> {
        let source = Self::decode(track)?;
        self.player.clear();
        self.player.append(source);
        self.apply_amplitude();
        self.player.play();
        Ok(())
    }

    /// Not yet implemented; gapless is M5b.
    ///
    /// Appending here would be queueing rather than gapless (§3.4) and would
    /// also break [`Self::finished`], which reports on the queue being empty.
    /// Doing nothing produces a small gap between tracks, which is the honest
    /// behaviour until the real thing lands.
    fn preload_next(&mut self, _track: &TrackRef) -> Result<(), CoreError> {
        Ok(())
    }

    fn clear_preload(&mut self) {}

    fn pause(&mut self) {
        self.player.pause();
    }

    fn resume(&mut self) {
        self.player.play();
    }

    fn stop(&mut self) {
        self.player.stop();
    }

    fn seek(&mut self, position: Duration) -> Result<(), CoreError> {
        self.player
            .try_seek(position)
            .map_err(|e| CoreError::Database(format!("cannot seek: {e}")))
    }

    fn set_gain(&mut self, db: f32) {
        // dB to linear amplitude. Zero dB is unity, which is what an untagged
        // track gets (§6.7).
        self.gain = if db.is_finite() {
            10.0_f32.powf(db / 20.0)
        } else {
            1.0
        };
        self.apply_amplitude();
    }

    fn set_volume(&mut self, linear: f32) {
        self.volume = linear.clamp(0.0, 1.0);
        self.apply_amplitude();
    }

    fn position(&self) -> Duration {
        self.player.get_pos()
    }

    fn finished(&self) -> bool {
        self.player.empty()
    }
}
