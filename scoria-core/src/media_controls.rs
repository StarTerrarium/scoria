//! Desktop media controls.
//!
//! `DESIGN.md` §12: souvlaki, for media keys, `playerctl` and whatever the
//! desktop shows in its own panel. On Linux that is MPRIS over D-Bus.
//!
//! This lives in core rather than the frontend because it is not a frontend
//! thing: a headless session should still answer the play button on a keyboard,
//! and the TUI should not have to reimplement it. souvlaki is not a UI crate,
//! so invariant 1 is untouched.
//!
//! The desktop's controls are just another source of [`Command`]s. Everything
//! they can ask for is something a frontend can already ask for, which is why
//! this needs no new vocabulary -- it translates into the seam that exists.

use crate::command::Command;
use crate::error::CoreError;
use crate::event::PlaybackState;
use crate::model::Track;
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

/// How far `Seek` moves when the desktop does not say how far.
const SEEK_STEP: Duration = Duration::from_secs(10);

/// The bridge between the desktop's controls and the session.
pub struct MediaBridge {
    /// An `Option` only so that [`Drop`] can move it out. It is `Some` for the
    /// whole life of the bridge.
    controls: Option<MediaControls>,
    commands: Receiver<Command>,
    /// What was last published, so unchanged state is not re-sent. Position
    /// moves every tick, and telling D-Bus about it forty times a second would
    /// be forty times more traffic than anything needs.
    published: Option<Published>,
}

#[derive(PartialEq)]
struct Published {
    track: Option<crate::id::TrackId>,
    state: PlaybackState,
    /// Whole seconds only: the desktop shows seconds.
    seconds: u64,
}

impl MediaBridge {
    /// Registers with the desktop.
    ///
    /// # Errors
    ///
    /// [`CoreError::Desktop`] if the platform's controls are unavailable --
    /// no session bus, or a desktop with no MPRIS support. That is a normal
    /// thing to run into and not a reason to fail to start.
    pub fn new() -> Result<Self, CoreError> {
        let config = PlatformConfig {
            dbus_name: "scoria",
            display_name: "Scoria",
            hwnd: None,
        };

        let mut controls = MediaControls::new(config)
            .map_err(|e| CoreError::Desktop(format!("cannot reach the desktop controls: {e:?}")))?;

        let (sender, commands) = channel();
        controls
            .attach(move |event| {
                if let Some(command) = translate(&event) {
                    // A closed receiver means the session is gone; there is
                    // nothing left to tell.
                    let _ = sender.send(command);
                }
            })
            .map_err(|e| {
                CoreError::Desktop(format!("cannot attach to the desktop controls: {e:?}"))
            })?;

        Ok(Self {
            controls: Some(controls),
            commands,
            published: None,
        })
    }

    /// Whatever the desktop has asked for since the last call.
    pub fn take_commands(&self) -> Vec<Command> {
        let mut commands = Vec::new();
        while let Ok(command) = self.commands.try_recv() {
            commands.push(command);
        }
        commands
    }

    /// Tells the desktop what is playing.
    ///
    /// Does nothing when nothing has changed, which is most calls.
    pub fn publish(
        &mut self,
        state: PlaybackState,
        track: Option<&Track>,
        position: Duration,
        artwork: Option<&std::path::Path>,
    ) {
        let now = Published {
            track: track.map(|t| t.id),
            state,
            seconds: position.as_secs(),
        };
        if self.published.as_ref() == Some(&now) {
            return;
        }
        let track_changed = self.published.as_ref().map(|p| p.track) != Some(now.track);
        self.published = Some(now);

        if track_changed {
            // A `file://` URL, which is what the MPRIS specification asks for
            // and what a desktop panel will actually load.
            let cover = artwork.map(|path| format!("file://{}", path.display()));
            let metadata = track.map_or_else(MediaMetadata::default, |track| MediaMetadata {
                title: Some(&track.title),
                album: Some(&track.album),
                // The display artist, with its full credits (§4.1). The desktop
                // is showing it to a person, not sorting by it.
                artist: Some(&track.artist),
                cover_url: cover.as_deref(),
                duration: Some(track.duration),
            });
            if let Some(controls) = self.controls.as_mut() {
                let _ = controls.set_metadata(metadata);
            }
        }

        let progress = Some(MediaPosition(position));
        let playback = match state {
            PlaybackState::Playing => MediaPlayback::Playing { progress },
            PlaybackState::Paused => MediaPlayback::Paused { progress },
            PlaybackState::Stopped => MediaPlayback::Stopped,
        };
        if let Some(controls) = self.controls.as_mut() {
            let _ = controls.set_playback(playback);
        }
    }
}

/// Turns a desktop control into a command.
///
/// `None` for the ones this application has no answer to. Ignoring them is
/// right: a desktop may offer a raise or a shuffle button whether or not the
/// player implements it.
fn translate(event: &MediaControlEvent) -> Option<Command> {
    Some(match event {
        MediaControlEvent::Play => Command::Resume,
        MediaControlEvent::Pause => Command::Pause,
        // The session decides which, because only it knows the current state.
        MediaControlEvent::Toggle => Command::PlayPause,
        MediaControlEvent::Next => Command::Next,
        MediaControlEvent::Previous => Command::Previous,
        MediaControlEvent::Stop | MediaControlEvent::Quit => Command::Stop,
        MediaControlEvent::SetPosition(MediaPosition(position)) => Command::Seek(*position),
        MediaControlEvent::SeekBy(direction, by) => Command::SeekBy(signed(*direction, *by)),
        MediaControlEvent::Seek(direction) => Command::SeekBy(signed(*direction, SEEK_STEP)),
        MediaControlEvent::SetVolume(volume) => Command::SetVolume(volume.clamp(0.0, 1.0) as f32),
        MediaControlEvent::OpenUri(_) | MediaControlEvent::Raise => return None,
    })
}

/// Milliseconds, negative for backwards.
fn signed(direction: SeekDirection, by: Duration) -> i64 {
    let millis = i64::try_from(by.as_millis()).unwrap_or(i64::MAX);
    match direction {
        SeekDirection::Forward => millis,
        SeekDirection::Backward => -millis,
    }
}

/// Lets go of the desktop's controls without making anyone wait for it.
///
/// souvlaki's own `Drop` tells its D-Bus service thread to stop and then joins
/// it, and that thread finishes what it had queued first -- which costs about a
/// second per property change still in flight. On the way out of an
/// application that is a window that will not go away: measured at 300ms to
/// 850ms with the controls attached, against 15ms without them.
///
/// So the waiting is moved somewhere nobody is looking. Nothing is skipped --
/// the thread still stops and the bus name is still released -- it just happens
/// off the path between clicking the close button and the window going. If the
/// process exits first, the kernel closes the connection, which releases the
/// name anyway.
///
/// A thread that cannot be spawned drops the controls here instead, which is
/// the old blocking behaviour rather than a leak.
impl Drop for MediaBridge {
    fn drop(&mut self) {
        let Some(controls) = self.controls.take() else {
            return;
        };
        let _ = std::thread::Builder::new()
            .name("scoria-media-shutdown".to_owned())
            .spawn(move || drop(controls));
        // Deliberately not joined. The window is going, and nothing left in
        // this process depends on the answer.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_control_this_player_answers_becomes_a_command() {
        assert_eq!(translate(&MediaControlEvent::Play), Some(Command::Resume));
        assert_eq!(translate(&MediaControlEvent::Pause), Some(Command::Pause));
        assert_eq!(
            translate(&MediaControlEvent::Toggle),
            Some(Command::PlayPause)
        );
        assert_eq!(translate(&MediaControlEvent::Next), Some(Command::Next));
        assert_eq!(
            translate(&MediaControlEvent::Previous),
            Some(Command::Previous)
        );
        assert_eq!(translate(&MediaControlEvent::Stop), Some(Command::Stop));
    }

    #[test]
    fn controls_with_no_answer_are_ignored_rather_than_guessed_at() {
        // A desktop offers what it offers, whether or not the player has an
        // answer. Inventing one would be worse than doing nothing.
        assert_eq!(translate(&MediaControlEvent::Raise), None);
        assert_eq!(translate(&MediaControlEvent::OpenUri("x".to_owned())), None);
    }

    #[test]
    fn seeking_backwards_is_negative() {
        let back = translate(&MediaControlEvent::SeekBy(
            SeekDirection::Backward,
            Duration::from_secs(5),
        ));
        assert_eq!(back, Some(Command::SeekBy(-5000)));

        let forward = translate(&MediaControlEvent::SeekBy(
            SeekDirection::Forward,
            Duration::from_secs(5),
        ));
        assert_eq!(forward, Some(Command::SeekBy(5000)));
    }

    #[test]
    fn an_unqualified_seek_moves_a_sensible_amount() {
        let forward = translate(&MediaControlEvent::Seek(SeekDirection::Forward));
        assert_eq!(forward, Some(Command::SeekBy(SEEK_STEP.as_millis() as i64)));
    }

    #[test]
    fn volume_from_the_desktop_is_clamped() {
        // MediaControlEvent::SetVolume is documented as accepting values
        // outside 0..=1, and it is up to us to decide what that means.
        assert_eq!(
            translate(&MediaControlEvent::SetVolume(2.0)),
            Some(Command::SetVolume(1.0))
        );
        assert_eq!(
            translate(&MediaControlEvent::SetVolume(-1.0)),
            Some(Command::SetVolume(0.0))
        );
    }
}
