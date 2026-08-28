//! The audio thread.
//!
//! `DESIGN.md` §3.3: a dedicated OS thread fed by `std::sync::mpsc`, never a
//! tokio task. Scheduler jitter that is invisible in a web handler is audible
//! in an audio callback.
//!
//! The engine owns the [`PlaybackSource`] and the backend, and is the only
//! thing that touches either. Everything else talks to it by message, which is
//! what makes "browsing never disturbs playback" (§6.1) structural rather than
//! a matter of care.

use crate::error::CoreError;
use crate::event::{Event, PlaybackState};
use crate::id::TrackId;
use crate::model::GainMode;
use crate::playback::backend::{AudioBackend, TrackRef};
use crate::playback::rng::Rng;
use crate::playback::source::{PlaybackSource, RepeatMode};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;

/// How often the engine wakes to poll position and notice a finished track.
///
/// Fast enough that the elapsed time does not visibly stutter, slow enough that
/// an idle player is not busy. This is a poll rather than a callback because
/// the backend trait deliberately does not hand out callbacks: a callback would
/// run on the backend's own thread, and then two threads would own the source.
const TICK: Duration = Duration::from_millis(100);

/// How close to the end of a track the next one is prepared.
///
/// §3.4: the next decoder must be initialised and its first packets decoded
/// *before* the current tail drains, or the result is queueing rather than
/// gapless.
const PRELOAD_LEAD: Duration = Duration::from_secs(5);

/// Turns a [`TrackId`] into something playable.
///
/// The engine cannot hold the library itself: the library changes while music
/// plays, and §6.1 requires that not to disturb playback. So it holds this, and
/// the session swaps in a new one when the library changes.
pub trait TrackResolver: Send + Sync + std::fmt::Debug {
    /// `None` if the track is unknown or its file no longer resolves, which is
    /// what makes §6.4's skip rule work.
    fn resolve(&self, id: TrackId) -> Option<TrackRef>;
}

/// Messages into the audio thread.
///
/// Distinct from the public [`Command`](crate::command::Command): that is the
/// seam a frontend talks to, this is internal plumbing. Keeping them separate
/// means the engine can grow a message without widening the public contract.
#[derive(Debug)]
pub enum EngineCommand {
    /// Replace the snapshot and start playing it.
    PlaySource(Box<PlaybackSource>),
    Pause,
    Resume,
    Stop,
    Next,
    Previous,
    Seek(Duration),
    SetVolume(f32),
    /// Jump to a row of the current snapshot (§9.2).
    SetPosition(usize),
    SetShuffle(bool),
    SetRepeat(RepeatMode),
    /// The library changed; resolve against this from now on.
    SetResolver(Arc<dyn TrackResolver>),
    Shutdown,
}

/// A handle to a running audio thread.
///
/// Dropping it shuts the thread down and waits for it, so an engine cannot
/// outlive the thing that started it.
#[derive(Debug)]
pub struct PlaybackHandle {
    commands: Sender<EngineCommand>,
    thread: Option<JoinHandle<()>>,
}

impl PlaybackHandle {
    /// Sends a command to the audio thread.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the audio thread has stopped.
    pub fn send(&self, command: EngineCommand) -> Result<(), CoreError> {
        self.commands
            .send(command)
            .map_err(|_| CoreError::Database("the audio thread has stopped".to_owned()))
    }
}

impl Drop for PlaybackHandle {
    fn drop(&mut self) {
        let _ = self.commands.send(EngineCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            // A panicking audio thread is already reported as an Error event by
            // `run`; there is nothing useful to do with the join result here.
            let _ = thread.join();
        }
    }
}

/// Starts the audio thread.
///
/// Events go to `events` rather than to a channel of the engine's own, so a
/// session can merge playback events with query and scan events into the single
/// stream a frontend consumes. §3.2 describes one event stream, not two.
///
/// # Errors
///
/// [`CoreError::Database`] if the thread cannot be spawned.
pub fn spawn(
    backend: Box<dyn AudioBackend>,
    resolver: Arc<dyn TrackResolver>,
    events: Sender<Event>,
) -> Result<PlaybackHandle, CoreError> {
    let (command_tx, command_rx) = channel();
    let event_tx = events;

    let thread = std::thread::Builder::new()
        .name("scoria-audio".to_owned())
        .spawn(move || {
            // §3.3, failure containment. The engine is written not to panic,
            // but a panic here would otherwise kill audio silently and leave a
            // UI showing a playing track with a transport that does nothing.
            // Caught, it becomes an error and a stopped transport, which is at
            // least honest.
            let events = event_tx.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                Engine::new(backend, resolver, event_tx).run(&command_rx);
            }));
            if result.is_err() {
                let _ = events.send(Event::Error(CoreError::Database(
                    "the audio thread stopped unexpectedly; playback has ended".to_owned(),
                )));
                let _ = events.send(Event::PlaybackStateChanged(PlaybackState::Stopped));
            }
        })
        .map_err(|e| CoreError::Database(format!("cannot start the audio thread: {e}")))?;

    Ok(PlaybackHandle {
        commands: command_tx,
        thread: Some(thread),
    })
}

struct Engine {
    backend: Box<dyn AudioBackend>,
    resolver: Arc<dyn TrackResolver>,
    events: Sender<Event>,
    source: PlaybackSource,
    state: PlaybackState,
    volume: f32,
    /// Shuffle and repeat belong to the player, not to any one snapshot.
    ///
    /// They are stored on `PlaybackSource` because that is where `advance`
    /// needs them, but a user who turned shuffle on expects it to still be on
    /// after clicking a different track -- exactly as they expect of volume.
    /// So the engine keeps the preference and stamps it onto each new source.
    repeat: RepeatMode,
    shuffled: bool,
    rng: Rng,
    /// Whether the next track has been handed to the backend already.
    preloaded: bool,
    last_reported_position: Duration,
}

impl Engine {
    fn new(
        backend: Box<dyn AudioBackend>,
        resolver: Arc<dyn TrackResolver>,
        events: Sender<Event>,
    ) -> Self {
        Self {
            backend,
            resolver,
            events,
            source: PlaybackSource::new(Vec::new(), crate::playback::Origin::Library),
            state: PlaybackState::Stopped,
            volume: 1.0,
            repeat: RepeatMode::Off,
            shuffled: false,
            rng: Rng::from_clock(),
            preloaded: false,
            last_reported_position: Duration::MAX,
        }
    }

    fn run(&mut self, commands: &Receiver<EngineCommand>) {
        loop {
            match commands.recv_timeout(TICK) {
                Ok(EngineCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                    self.backend.stop();
                    return;
                }
                Ok(command) => self.handle(command),
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.tick();
        }
    }

    fn emit(&self, event: Event) {
        // A closed receiver means the session is gone and shutdown is already
        // under way; there is nothing to report it to.
        let _ = self.events.send(event);
    }

    fn handle(&mut self, command: EngineCommand) {
        match command {
            EngineCommand::PlaySource(source) => {
                self.source = *source;
                // Carry the player's shuffle and repeat onto the new snapshot.
                self.source.set_repeat(self.repeat);
                if self.shuffled {
                    self.source.shuffle(&mut self.rng);
                }
                self.start_current();
                self.emit(Event::QueueChanged(self.source.clone()));
            }
            EngineCommand::Pause => {
                if self.state == PlaybackState::Playing {
                    self.backend.pause();
                    self.set_state(PlaybackState::Paused);
                }
            }
            EngineCommand::Resume => {
                if self.state == PlaybackState::Paused {
                    self.backend.resume();
                    self.set_state(PlaybackState::Playing);
                }
            }
            EngineCommand::Stop => {
                self.backend.stop();
                self.preloaded = false;
                self.set_state(PlaybackState::Stopped);
            }
            EngineCommand::Next => self.advance(SkipReason::User),
            EngineCommand::Previous => {
                if self.source.previous().is_some() {
                    self.start_current();
                }
            }
            EngineCommand::Seek(position) => {
                if let Err(e) = self.backend.seek(position) {
                    self.emit(Event::Error(e));
                } else {
                    self.emit(Event::PositionChanged(position));
                }
            }
            EngineCommand::SetVolume(linear) => {
                self.volume = linear.clamp(0.0, 1.0);
                self.backend.set_volume(self.volume);
            }
            EngineCommand::SetPosition(position) => {
                if self.source.set_position(position) {
                    self.start_current();
                    self.emit(Event::QueueChanged(self.source.clone()));
                } else {
                    self.emit(Event::Error(CoreError::PositionOutOfRange {
                        position,
                        len: self.source.len(),
                    }));
                }
            }
            EngineCommand::SetShuffle(shuffled) => {
                self.shuffled = shuffled;
                self.source.set_shuffled(shuffled, &mut self.rng);
                // What plays next just changed, so anything prepared is wrong.
                self.invalidate_preload();
                // Gain mode depends on shuffle (§6.7), so the current track's
                // correction may have changed even though the track has not.
                // The gain mode depends on shuffle (§6.7), and gain now rides
                // on the source rather than being a backend setting, so the
                // current track's correction cannot be changed in place. It
                // keeps the gain it started with, and the new mode applies from
                // the next track on. Restarting the track to correct a fraction
                // of a decibel would interrupt what the user is listening to,
                // which is a far worse trade than a slightly wrong gain for the
                // remainder of one song.
                self.emit(Event::QueueChanged(self.source.clone()));
            }
            EngineCommand::SetRepeat(repeat) => {
                self.repeat = repeat;
                self.source.set_repeat(repeat);
                self.invalidate_preload();
                self.emit(Event::QueueChanged(self.source.clone()));
            }
            EngineCommand::SetResolver(resolver) => {
                self.resolver = resolver;
                // A rescan may have changed where the next track lives.
                self.invalidate_preload();
            }
            EngineCommand::Shutdown => {}
        }
    }

    fn tick(&mut self) {
        if self.state != PlaybackState::Playing {
            return;
        }

        // With gapless, the backend moves into the queued track by itself:
        // there is no moment where nothing is playing for the engine to notice.
        // So the engine follows what is actually being heard rather than
        // deciding it, and only takes over once the backend has run out.
        match self.backend.current() {
            None => {
                self.advance(SkipReason::TrackEnded);
                return;
            }
            Some(playing) if Some(playing) != self.source.current() => {
                self.follow_backend(playing);
            }
            Some(_) => {}
        }

        let position = self.backend.position();
        // Only report whole seconds: the transport shows seconds, and emitting
        // ten events a second to redraw the same string is waste the UI would
        // have to filter out anyway.
        if position.as_secs() != self.last_reported_position.as_secs() {
            self.last_reported_position = position;
            self.emit(Event::PositionChanged(position));
        }

        self.maybe_preload(position);
    }

    /// Brings the snapshot into step with a gapless transition the backend
    /// made without being told to.
    ///
    /// The queued track is by construction `peek_next`, so advancing once
    /// should land on it. If it does not -- because a preload was retracted too
    /// late to take effect -- the snapshot is authoritative and the engine
    /// restarts from it, accepting a gap in exchange for playing the right
    /// thing.
    fn follow_backend(&mut self, playing: TrackId) {
        self.preloaded = false;
        self.source.advance();

        if self.source.current() == Some(playing) {
            self.last_reported_position = Duration::MAX;
            if let Some(position) = self.source.position() {
                self.emit(Event::TrackChanged {
                    track: playing,
                    position,
                });
            }
        } else {
            self.start_current();
        }
    }

    /// Hands the following track to the backend as the current one nears its
    /// end (§3.4).
    fn maybe_preload(&mut self, position: Duration) {
        if self.preloaded {
            return;
        }
        let Some(current) = self.current_ref() else {
            return;
        };
        if current.duration.saturating_sub(position) > PRELOAD_LEAD {
            return;
        }
        // Repeat-one replays the same source rather than flowing into another,
        // so there is nothing to prepare.
        if self.source.repeat() == RepeatMode::One {
            return;
        }
        let Some(next) = self.source.peek_next() else {
            return;
        };
        let Some(next) = self.resolver.resolve(next) else {
            return;
        };

        // A preload that fails is not worth reporting: the track will be opened
        // again on advance, and that attempt will report properly. Failing
        // loudly here would put an error in front of the user for a file that
        // then plays fine.
        let gain = self.gain_for(&next);
        if self.backend.preload_next(&next, gain).is_ok() {
            self.preloaded = true;
        }
    }

    fn invalidate_preload(&mut self) {
        if self.preloaded {
            self.backend.clear_preload();
            self.preloaded = false;
        }
    }

    /// Moves to the next track, honouring repeat and skipping what will not
    /// play (§6.4).
    fn advance(&mut self, reason: SkipReason) {
        // Repeat-one replays the current track when it ends, but Next still
        // means skip: the user pressing Next plainly means to move on (§6.6).
        if reason == SkipReason::TrackEnded && self.source.repeat() == RepeatMode::One {
            self.start_current();
            return;
        }

        self.invalidate_preload();
        if self.source.advance().is_some() {
            self.start_current();
        } else {
            // §6.4: the snapshot stops when it runs out. A snapshot that
            // silently expands is not a snapshot.
            self.backend.stop();
            self.set_state(PlaybackState::Stopped);
        }
    }

    /// Plays whatever the source currently points at, stepping over tracks that
    /// will not resolve or will not open.
    ///
    /// Bounded by the length of the snapshot, so a wholly missing library stops
    /// rather than spinning -- which matters under repeat-all, where advancing
    /// never runs out on its own.
    fn start_current(&mut self) {
        for _ in 0..=self.source.len() {
            let Some(id) = self.source.current() else {
                break;
            };

            match self.resolver.resolve(id) {
                Some(track) => {
                    let gain = self.gain_for(&track);
                    match self.backend.play(&track, gain) {
                        Ok(()) => {
                            self.preloaded = false;
                            self.last_reported_position = Duration::MAX;
                            self.set_state(PlaybackState::Playing);
                            if let Some(position) = self.source.position() {
                                self.emit(Event::TrackChanged {
                                    track: id,
                                    position,
                                });
                            }
                            return;
                        }
                        Err(e) => self.emit(Event::Error(e)),
                    }
                }
                None => self.emit(Event::Error(CoreError::NoSuchTrack(id.get()))),
            }

            // §6.4: on advance, tracks whose files no longer resolve are
            // skipped rather than stopping playback.
            if self.source.advance().is_none() {
                break;
            }
        }

        self.backend.stop();
        self.set_state(PlaybackState::Stopped);
    }

    /// §6.7: album gain when playing in album order, track gain when shuffled.
    /// Derived from how playback is ordered, never configured.
    fn gain_for(&self, track: &TrackRef) -> f32 {
        let mode = if self.source.is_shuffled() {
            GainMode::Track
        } else {
            GainMode::Album
        };
        track.replay_gain.gain_db(mode)
    }

    fn current_ref(&self) -> Option<TrackRef> {
        self.resolver.resolve(self.source.current()?)
    }

    fn set_state(&mut self, state: PlaybackState) {
        if self.state != state {
            self.state = state;
            self.emit(Event::PlaybackStateChanged(state));
        }
    }
}

/// Why the engine is moving on, which decides whether repeat-one applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkipReason {
    TrackEnded,
    User,
}
