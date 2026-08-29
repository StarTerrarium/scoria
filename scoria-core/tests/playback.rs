//! Playback engine tests.
//!
//! Every rule in §6.4 and §6.6, and the gain rule in §6.7, asserted against
//! `MockBackend` with no audio device involved. That is the whole point of
//! §3.4's trait: playback behaviour is something you check, not something you
//! listen for.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use scoria_core::error::CoreError;
use scoria_core::event::{Event, PlaybackState};
use scoria_core::id::TrackId;
use scoria_core::model::ReplayGain;
use scoria_core::playback::backend::mock::{Call, MockControl};
use scoria_core::playback::engine::{EngineCommand, TrackResolver, spawn};
use scoria_core::playback::{
    MockBackend, Origin, PlaybackHandle, PlaybackSource, RepeatMode, TrackRef,
};
use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Resolves ids to synthetic tracks, with a configurable set that "does not
/// exist" so §6.4's skip rule can be exercised.
#[derive(Debug, Default)]
struct TestResolver {
    missing: Mutex<HashSet<TrackId>>,
    gains: Mutex<Vec<(TrackId, ReplayGain)>>,
}

impl TestResolver {
    fn with_missing(ids: &[u64]) -> Arc<Self> {
        let resolver = Self::default();
        *resolver
            .missing
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = ids.iter().map(|i| TrackId(*i)).collect();
        Arc::new(resolver)
    }

    fn with_gain(id: u64, gain: ReplayGain) -> Arc<Self> {
        let resolver = Self::default();
        resolver
            .gains
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((TrackId(id), gain));
        Arc::new(resolver)
    }
}

impl TrackResolver for TestResolver {
    fn resolve(&self, id: TrackId) -> Result<TrackRef, scoria_core::error::CoreError> {
        if self
            .missing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&id)
        {
            return Err(scoria_core::error::CoreError::FileMissing {
                title: format!("track {id:?}"),
            });
        }
        let replay_gain = self
            .gains
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(t, _)| *t == id)
            .map_or_else(ReplayGain::default, |(_, g)| *g);

        Ok(TrackRef {
            id,
            path: format!("/nowhere/{id}.flac").into(),
            duration: Duration::from_secs(200),
            replay_gain,
        })
    }
}

struct Rig {
    handle: PlaybackHandle,
    events: Receiver<Event>,
    control: MockControl,
}

impl Rig {
    fn new(resolver: Arc<dyn TrackResolver>) -> Self {
        let backend = MockBackend::new();
        let control = backend.control();
        let (event_tx, events) = std::sync::mpsc::channel();
        let handle = spawn(Box::new(backend), resolver, event_tx).unwrap();
        Self {
            handle,
            events,
            control,
        }
    }

    fn plain() -> Self {
        Self::new(Arc::new(TestResolver::default()))
    }

    fn send(&self, command: EngineCommand) {
        self.handle.send(command).unwrap();
    }

    fn play(&self, ids: &[u64]) {
        let source =
            PlaybackSource::new(ids.iter().map(|i| TrackId(*i)).collect(), Origin::Library);
        self.send(EngineCommand::PlaySource(Box::new(source)));
    }

    /// Waits for a condition, so tests never depend on the engine's tick rate.
    fn until(&self, what: &str, mut predicate: impl FnMut(&MockControl) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if predicate(&self.control) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!(
            "timed out waiting for {what}; calls were {:?}",
            self.control.calls()
        );
    }

    fn until_played(&self, ids: &[u64]) {
        let expected: Vec<TrackId> = ids.iter().map(|i| TrackId(*i)).collect();
        self.until(&format!("played {ids:?}"), |c| c.played() == expected);
    }

    /// Ends the current track and waits for the engine to notice.
    fn finish_and_settle(&self) {
        let before = self.control.calls().len();
        let was_playing = self.control.now_playing();
        self.control.finish_track();

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            // Either the engine acted, or the backend flowed into a queued
            // track by itself and the engine has caught up with it.
            if self.control.calls().len() != before
                || (self.control.now_playing().is_some()
                    && self.control.now_playing() != was_playing)
            {
                std::thread::sleep(Duration::from_millis(120));
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Every track the backend actually rendered, whether the engine started it
    /// or it flowed in gaplessly.
    fn heard(&self) -> Vec<TrackId> {
        self.control
            .calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Play(id) | Call::Preload(id) => Some(id),
                _ => None,
            })
            .collect()
    }

    fn events(&self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            events.push(event);
        }
        events
    }

    fn wait_for_event(&self, what: &str, mut predicate: impl FnMut(&Event) -> bool) -> Event {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(event) = self.events.recv_timeout(Duration::from_millis(100))
                && predicate(&event)
            {
                return event;
            }
        }
        panic!("timed out waiting for {what}");
    }
}

// -------------------------------------------------------------- the basics --

#[test]
fn playing_a_source_starts_its_first_track() {
    let rig = Rig::plain();
    rig.play(&[1, 2, 3]);
    rig.until_played(&[1]);
    assert_eq!(rig.control.now_playing(), Some(TrackId(1)));
}

#[test]
fn a_track_ending_advances_to_the_next() {
    let rig = Rig::plain();
    rig.play(&[1, 2, 3]);
    rig.until_played(&[1]);

    rig.finish_and_settle();
    rig.until_played(&[1, 2]);
}

#[test]
fn pause_and_resume_reach_the_backend() {
    let rig = Rig::plain();
    rig.play(&[1]);
    rig.until_played(&[1]);

    rig.send(EngineCommand::Pause);
    rig.until("pause", |c| c.calls().contains(&Call::Pause));

    rig.send(EngineCommand::Resume);
    rig.until("resume", |c| c.calls().contains(&Call::Resume));
}

#[test]
fn pausing_when_stopped_does_nothing() {
    let rig = Rig::plain();
    rig.send(EngineCommand::Pause);
    std::thread::sleep(Duration::from_millis(150));
    assert!(!rig.control.calls().contains(&Call::Pause));
}

#[test]
fn stop_clears_what_is_playing() {
    let rig = Rig::plain();
    rig.play(&[1, 2]);
    rig.until_played(&[1]);

    rig.send(EngineCommand::Stop);
    rig.until("stop", |c| c.now_playing().is_none());
}

#[test]
fn seek_reaches_the_backend() {
    let rig = Rig::plain();
    rig.play(&[1]);
    rig.until_played(&[1]);

    rig.send(EngineCommand::Seek(Duration::from_secs(42)));
    rig.until("seek", |c| {
        c.calls().contains(&Call::Seek(Duration::from_secs(42)))
    });
}

#[test]
fn volume_is_clamped_to_a_sane_range() {
    let rig = Rig::plain();
    rig.send(EngineCommand::SetVolume(5.0));
    rig.until("clamped volume", |c| c.calls().contains(&Call::Volume(1.0)));

    rig.send(EngineCommand::SetVolume(-3.0));
    rig.until("clamped volume", |c| c.calls().contains(&Call::Volume(0.0)));
}

// ------------------------------------------------------------- §6.4 rules --

#[test]
fn the_snapshot_stops_when_it_runs_out() {
    // "A snapshot that silently expands is not a snapshot."
    let rig = Rig::plain();
    rig.play(&[1, 2]);
    rig.until_played(&[1]);

    rig.finish_and_settle();
    rig.until_played(&[1, 2]);

    rig.finish_and_settle();
    rig.until("stop", |c| c.calls().contains(&Call::Stop));
    assert_eq!(
        rig.control.played(),
        vec![TrackId(1), TrackId(2)],
        "nothing more should play"
    );
}

#[test]
fn reaching_the_end_reports_stopped() {
    let rig = Rig::plain();
    rig.play(&[1]);
    rig.until_played(&[1]);
    rig.finish_and_settle();

    rig.wait_for_event("stopped", |e| {
        matches!(e, Event::PlaybackStateChanged(PlaybackState::Stopped))
    });
}

#[test]
fn tracks_that_no_longer_resolve_are_skipped() {
    // §6.4: on advance, tracks whose files no longer resolve are skipped.
    let rig = Rig::new(TestResolver::with_missing(&[2, 3]));
    rig.play(&[1, 2, 3, 4]);
    rig.until_played(&[1]);

    rig.finish_and_settle();
    rig.until_played(&[1, 4]);
}

#[test]
fn a_skipped_track_says_its_file_is_missing_rather_than_that_it_does_not_exist() {
    // §7.4 keeps a vanished track visible so someone can go and look for the
    // file. "no such track: 2" does not help them do that, and it is not even
    // true -- the track is right there in the library.
    let rig = Rig::new(TestResolver::with_missing(&[1]));
    rig.play(&[1, 2]);
    rig.until_played(&[2]);

    let reported: Vec<_> = rig
        .events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Error(e) => Some(format!("{e}")),
            _ => None,
        })
        .collect();

    assert!(
        reported.iter().any(|e| e.contains("the file is missing")),
        "the skip was reported as {reported:?}"
    );
}

#[test]
fn a_source_whose_first_tracks_are_missing_starts_at_the_first_that_plays() {
    let rig = Rig::new(TestResolver::with_missing(&[1, 2]));
    rig.play(&[1, 2, 3]);
    rig.until_played(&[3]);
}

#[test]
fn a_wholly_missing_source_stops_rather_than_spinning() {
    // Under repeat-all, advancing never runs out on its own, so an unbounded
    // skip loop would spin forever on a library that has gone.
    let rig = Rig::new(TestResolver::with_missing(&[1, 2, 3]));
    rig.send(EngineCommand::SetRepeat(RepeatMode::All));
    rig.play(&[1, 2, 3]);

    rig.until("stop", |c| c.calls().contains(&Call::Stop));
    assert!(rig.control.played().is_empty());
}

#[test]
fn a_backend_that_cannot_open_a_file_skips_to_the_next() {
    let rig = Rig::plain();
    rig.control.set_unplayable(&[TrackId(1)]);
    rig.play(&[1, 2]);
    rig.until_played(&[2]);

    assert!(
        rig.events().iter().any(|e| matches!(e, Event::Error(_))),
        "the failure should still be reported"
    );
}

// ------------------------------------------------------ §6.6 shuffle/repeat --

#[test]
fn shuffle_and_repeat_survive_a_new_snapshot() {
    // They belong to the player, not to a snapshot: clicking a different track
    // must not silently turn shuffle off, any more than it resets the volume.
    let rig = Rig::plain();
    rig.send(EngineCommand::SetRepeat(RepeatMode::All));
    rig.play(&[1, 2]);
    rig.until_played(&[1]);

    rig.play(&[5, 6]);
    rig.until_played(&[1, 5]);

    rig.finish_and_settle();
    rig.until_played(&[1, 5, 6]);
    rig.finish_and_settle();
    rig.until_played(&[1, 5, 6, 5]);
}

#[test]
fn repeat_all_wraps_rather_than_stopping() {
    let rig = Rig::plain();
    rig.send(EngineCommand::SetRepeat(RepeatMode::All));
    rig.play(&[1, 2]);
    rig.until_played(&[1]);

    rig.finish_and_settle();
    rig.until_played(&[1, 2]);

    rig.finish_and_settle();
    rig.until_played(&[1, 2, 1]);
}

#[test]
fn repeat_one_replays_the_same_track_when_it_ends() {
    let rig = Rig::plain();
    rig.send(EngineCommand::SetRepeat(RepeatMode::One));
    rig.play(&[1, 2]);
    rig.until_played(&[1]);

    rig.finish_and_settle();
    rig.until_played(&[1, 1]);
}

#[test]
fn next_still_skips_under_repeat_one() {
    // §6.6: repeat-one is the engine replaying a track, but the user pressing
    // Next plainly means move on.
    let rig = Rig::plain();
    rig.send(EngineCommand::SetRepeat(RepeatMode::One));
    rig.play(&[1, 2]);
    rig.until_played(&[1]);

    rig.send(EngineCommand::Next);
    rig.until_played(&[1, 2]);
}

#[test]
fn shuffle_plays_every_track_exactly_once() {
    let rig = Rig::plain();
    rig.send(EngineCommand::SetShuffle(true));
    rig.play(&(1..=8).collect::<Vec<_>>());
    rig.until("first track", |c| c.now_playing().is_some());

    for _ in 0..7 {
        rig.finish_and_settle();
    }
    rig.until("the whole snapshot", |_| rig.heard().len() >= 8);

    let heard = rig.heard();
    let unique: HashSet<TrackId> = heard.iter().copied().collect();
    assert_eq!(
        unique.len(),
        8,
        "shuffle lost, duplicated or invented a track: {heard:?}"
    );
}

#[test]
fn toggling_shuffle_does_not_interrupt_the_current_track() {
    // The gain mode changes with shuffle (§6.7), but gain rides on the source,
    // so applying it to the current track would mean restarting it. Jumping
    // back to 0:00 mid-song is a far worse trade than a slightly wrong gain for
    // the rest of one track.
    let rig = Rig::plain();
    rig.play(&[1, 2, 3]);
    rig.until_played(&[1]);

    rig.send(EngineCommand::SetShuffle(true));
    std::thread::sleep(Duration::from_millis(200));

    assert_eq!(rig.control.now_playing(), Some(TrackId(1)));
    assert_eq!(
        rig.control.played(),
        vec![TrackId(1)],
        "the track was restarted"
    );
}

#[test]
fn previous_walks_back() {
    let rig = Rig::plain();
    rig.play(&[1, 2, 3]);
    rig.until_played(&[1]);
    rig.send(EngineCommand::Next);
    rig.until_played(&[1, 2]);

    rig.send(EngineCommand::Previous);
    rig.until_played(&[1, 2, 1]);
}

#[test]
fn setting_a_position_jumps_there() {
    // What double-clicking a row of the queue overlay does (§9.2).
    let rig = Rig::plain();
    rig.play(&[1, 2, 3, 4]);
    rig.until_played(&[1]);

    rig.send(EngineCommand::SetPosition(3));
    rig.until_played(&[1, 4]);
}

#[test]
fn a_position_outside_the_snapshot_is_reported_not_obeyed() {
    let rig = Rig::plain();
    rig.play(&[1, 2]);
    rig.until_played(&[1]);

    rig.send(EngineCommand::SetPosition(99));
    rig.wait_for_event("out-of-range error", |e| {
        matches!(e, Event::Error(CoreError::PositionOutOfRange { .. }))
    });
    assert_eq!(
        rig.control.now_playing(),
        Some(TrackId(1)),
        "playback should be undisturbed"
    );
}

// --------------------------------------------------------------- §6.7 gain --

#[test]
fn album_gain_applies_when_playing_in_order() {
    let gain = ReplayGain {
        track_gain_db: Some(-9.95),
        album_gain_db: Some(-9.69),
        ..ReplayGain::default()
    };
    let rig = Rig::new(TestResolver::with_gain(1, gain));
    rig.play(&[1]);
    rig.until_played(&[1]);

    assert_eq!(
        rig.control.last_gain(),
        Some(-9.69),
        "album order should use album gain"
    );
}

#[test]
fn track_gain_applies_when_shuffled() {
    // §6.7: shuffle destroys the loudness relationship album gain preserves, so
    // track gain is the correct choice there. Derived, never configured.
    let gain = ReplayGain {
        track_gain_db: Some(-9.95),
        album_gain_db: Some(-9.69),
        ..ReplayGain::default()
    };
    let rig = Rig::new(TestResolver::with_gain(1, gain));
    rig.send(EngineCommand::SetShuffle(true));
    rig.play(&[1, 2, 3]);
    rig.until("track gain", |c| c.last_gain() == Some(-9.95));
}

#[test]
fn an_untagged_track_plays_at_unity() {
    let rig = Rig::plain();
    rig.play(&[1]);
    rig.until_played(&[1]);
    assert_eq!(
        rig.control.last_gain(),
        Some(0.0),
        "no tags means 0 dB, never a guess"
    );
}

// -------------------------------------------------------------- the events --

#[test]
fn starting_a_track_reports_which_and_where() {
    let rig = Rig::plain();
    rig.play(&[1, 2, 3]);

    let event = rig.wait_for_event("track change", |e| matches!(e, Event::TrackChanged { .. }));
    assert_eq!(
        event,
        Event::TrackChanged {
            track: TrackId(1),
            position: 0
        }
    );
}

#[test]
fn the_queue_is_reported_when_it_changes() {
    let rig = Rig::plain();
    rig.play(&[1, 2]);

    let event = rig.wait_for_event("queue change", |e| matches!(e, Event::QueueChanged(_)));
    let Event::QueueChanged(source) = event else {
        panic!("wrong event")
    };
    assert_eq!(source.tracks(), &[TrackId(1), TrackId(2)]);
}

#[test]
fn position_is_reported_whenever_it_moves() {
    // This test used to assert the opposite: that position was reported once
    // per whole second, on the reasoning that the transport only shows seconds.
    // That reasoning stopped being true when the transport grew a seek bar,
    // which moves continuously -- throttling to one report a second made the
    // handle lurch a second's width at a time.
    let rig = Rig::plain();
    rig.play(&[1]);
    rig.until_played(&[1]);
    let _ = rig.events();

    for millis in [100, 250, 400, 550] {
        rig.control.set_position(Duration::from_millis(millis));
        std::thread::sleep(Duration::from_millis(120));
    }

    let reported: Vec<Duration> = rig
        .events()
        .into_iter()
        .filter_map(|e| match e {
            Event::PositionChanged(p) => Some(p),
            _ => None,
        })
        .collect();

    assert!(
        reported.len() >= 4,
        "sub-second movement should be reported, got {reported:?}"
    );
}

#[test]
fn an_unchanged_position_is_not_reported_again() {
    // Reporting movement is not the same as reporting every tick: a paused or
    // stalled track should go quiet rather than repeat itself.
    let rig = Rig::plain();
    rig.play(&[1]);
    rig.until_played(&[1]);
    rig.control.set_position(Duration::from_millis(1234));
    std::thread::sleep(Duration::from_millis(150));
    let _ = rig.events();

    std::thread::sleep(Duration::from_millis(300));
    let repeats = rig
        .events()
        .into_iter()
        .filter(|e| matches!(e, Event::PositionChanged(_)))
        .count();
    assert_eq!(repeats, 0, "a still position should not be re-reported");
}

// ------------------------------------------------------------- §3.4 preload --

#[test]
fn the_next_track_is_prepared_before_the_current_one_ends() {
    let rig = Rig::plain();
    rig.play(&[1, 2]);
    rig.until_played(&[1]);

    // Within the preload lead of a 200-second track.
    rig.control.set_position(Duration::from_secs(197));
    rig.until("preload", |c| c.preloaded() == Some(TrackId(2)));
}

#[test]
fn nothing_is_prepared_early_in_a_track() {
    let rig = Rig::plain();
    rig.play(&[1, 2]);
    rig.until_played(&[1]);

    rig.control.set_position(Duration::from_secs(10));
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(rig.control.preloaded(), None);
}

#[test]
fn changing_what_plays_next_discards_the_preload() {
    let rig = Rig::plain();
    rig.play(&[1, 2, 3]);
    rig.until_played(&[1]);
    rig.control.set_position(Duration::from_secs(197));
    rig.until("preload", |c| c.preloaded().is_some());

    rig.send(EngineCommand::SetShuffle(true));

    // The prepared track is discarded, then re-prepared for the order that now
    // applies. Asserting on *which* track that turns out to be would be flaky
    // rather than strict: shuffle keeps the current track first and picks the
    // rest at random, so the new next may legitimately be the same one. What
    // has to hold is that the stale preload was dropped rather than played.
    rig.until("preload discarded", |c| {
        c.calls().contains(&Call::ClearPreload)
    });
}

#[test]
fn nothing_is_prepared_at_the_end_of_a_snapshot() {
    let rig = Rig::plain();
    rig.play(&[1]);
    rig.until_played(&[1]);

    rig.control.set_position(Duration::from_secs(197));
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(
        rig.control.preloaded(),
        None,
        "there is no next track to prepare"
    );
}

// ------------------------------------------------------------- lifecycle --

#[test]
fn dropping_the_handle_stops_the_thread() {
    let rig = Rig::plain();
    rig.play(&[1]);
    rig.until_played(&[1]);

    let Rig { handle, events, .. } = rig;
    drop(handle);

    // The thread is joined by the drop, so the sender is gone by now.
    while events.try_recv().is_ok() {}
    assert!(
        events.try_recv().is_err(),
        "the event channel should be closed"
    );
}

#[test]
fn an_empty_source_stops_without_complaint() {
    let rig = Rig::plain();
    rig.play(&[]);
    std::thread::sleep(Duration::from_millis(200));
    assert!(rig.control.played().is_empty());
}

// -------------------------------------------------------------- §3.4 gapless --

#[test]
fn a_preloaded_transition_never_reopens_the_track() {
    // This is what gapless *is*, expressed as an assertion. The gap in a naive
    // player comes from tearing down the output and constructing a new decoder
    // at the boundary. If the engine calls `play` for the second track, that
    // teardown happened and the transition was not gapless -- whatever it
    // sounded like on the machine that tested it.
    let rig = Rig::plain();
    rig.play(&[1, 2]);
    rig.until_played(&[1]);

    rig.control.set_position(Duration::from_secs(197));
    rig.until("preload", |c| c.preloaded() == Some(TrackId(2)));

    rig.finish_and_settle();

    assert_eq!(
        rig.control.now_playing(),
        Some(TrackId(2)),
        "should have flowed into track 2"
    );
    assert_eq!(
        rig.control.played(),
        vec![TrackId(1)],
        "track 2 was re-opened rather than flowed into, so the transition had a gap"
    );
}

#[test]
fn the_snapshot_stays_in_step_with_a_transition_it_did_not_drive() {
    // The backend advances by itself, so the engine has to follow rather than
    // decide. If the snapshot fell behind, Next would skip the wrong track and
    // the queue overlay would mark the wrong row.
    let rig = Rig::plain();
    rig.play(&[1, 2, 3]);
    rig.until_played(&[1]);
    rig.control.set_position(Duration::from_secs(197));
    rig.until("preload", |c| c.preloaded() == Some(TrackId(2)));

    rig.finish_and_settle();
    rig.wait_for_event(
        "track change to 2",
        |e| matches!(e, Event::TrackChanged { track, .. } if *track == TrackId(2)),
    );

    // The snapshot now points at 2, so Next must reach 3 rather than 2 again.
    rig.send(EngineCommand::Next);
    rig.until("next reaches track 3", |c| {
        c.now_playing() == Some(TrackId(3))
    });
}

#[test]
fn a_retracted_preload_does_not_get_played() {
    // rodio cannot un-append, so a cancelled source is one that yields nothing
    // and is skipped. Without that, changing shuffle in the last seconds of a
    // track would play whatever had already been queued.
    let rig = Rig::plain();
    rig.play(&[1, 2, 3, 4, 5, 6, 7, 8]);
    rig.until_played(&[1]);
    rig.control.set_position(Duration::from_secs(197));
    rig.until("preload", |c| c.preloaded() == Some(TrackId(2)));

    rig.send(EngineCommand::SetPosition(5));
    rig.until("jumped", |c| c.now_playing() == Some(TrackId(6)));

    assert_eq!(
        rig.control.queue(),
        vec![TrackId(6)],
        "a stale track was left queued"
    );
}

#[test]
fn each_queued_track_carries_its_own_gain() {
    // rodio's player volume applies to everything queued, so per-track
    // ReplayGain cannot go through it. Two tracks in flight means two
    // corrections in flight.
    let quiet = ReplayGain {
        album_gain_db: Some(-12.0),
        ..ReplayGain::default()
    };
    let resolver = TestResolver::default();
    resolver
        .gains
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push((TrackId(2), quiet));
    let rig = Rig::new(Arc::new(resolver));

    rig.play(&[1, 2]);
    rig.until_played(&[1]);
    assert_eq!(rig.control.last_gain(), Some(0.0), "track 1 is untagged");

    rig.control.set_position(Duration::from_secs(197));
    rig.until("preload", |c| c.preloaded() == Some(TrackId(2)));
    assert_eq!(
        rig.control.last_gain(),
        Some(-12.0),
        "track 2 should carry its own gain"
    );
}

#[test]
fn repeat_one_does_not_preload() {
    // It replays the same source rather than flowing into another, so there is
    // nothing to prepare -- and preparing the same track would queue a second
    // copy behind it.
    let rig = Rig::plain();
    rig.send(EngineCommand::SetRepeat(RepeatMode::One));
    rig.play(&[1, 2]);
    rig.until_played(&[1]);

    rig.control.set_position(Duration::from_secs(197));
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(rig.control.preloaded(), None);
}
