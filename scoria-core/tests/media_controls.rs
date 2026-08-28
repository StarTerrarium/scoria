//! The desktop's media controls, against the real session bus.
//!
//! Skipped rather than failed where there is no bus to talk to: a build
//! machine has no desktop, and neither does a headless server, which are both
//! places this must still be able to run.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use scoria_core::event::PlaybackState;
use scoria_core::media_controls::MediaBridge;
use std::time::{Duration, Instant};

/// How long letting go of the controls may take before someone notices.
///
/// The blocking version cost about a second for each property change still in
/// flight, so anything in that neighbourhood is the bug returning rather than a
/// slow machine.
const PROMPT: Duration = Duration::from_millis(250);

#[test]
fn letting_go_of_the_controls_does_not_hold_up_the_way_out() {
    let mut bridge = match MediaBridge::new() {
        Ok(bridge) => bridge,
        // No session bus, or another Scoria already holds the name.
        Err(e) => {
            eprintln!("skipped: no desktop controls to test against ({e})");
            return;
        }
    };

    // Long enough for the connection to be established and for the desktop's
    // own panel to have subscribed. Before that the service thread has nothing
    // to do and lets go immediately, which is not the state that hurts.
    std::thread::sleep(Duration::from_secs(2));

    // Two property changes, published and immediately abandoned -- which is
    // what closing the window during playback does. souvlaki's own `Drop` joins
    // its D-Bus thread, and that thread finishes its queue first, so this is
    // the state that used to leave the window on screen for the best part of a
    // second.
    bridge.publish(PlaybackState::Playing, None, Duration::from_secs(1), None);
    bridge.publish(PlaybackState::Paused, None, Duration::from_secs(2), None);

    let started = Instant::now();
    drop(bridge);
    let took = started.elapsed();

    assert!(
        took < PROMPT,
        "letting go of the desktop controls took {took:?}, which is long enough to see"
    );
}
