//! Background watching and rescanning.
//!
//! These drive `Session` the way a frontend does -- start watching, change the
//! library on disk, wait for events -- because the two bugs that reached the
//! window at M7 were both wiring rather than logic, and unit tests over pure
//! functions could not have caught either.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use scoria_core::config::Paths;
use scoria_core::event::{Event, LibraryDelta};
use scoria_core::session::Session;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

mod common;
use common::Scratch;

/// A scratch home: settings, database and library root all under one
/// directory, so a session here touches nothing real.
struct Home {
    scratch: Scratch,
}

impl Home {
    fn new(name: &str) -> Self {
        Self {
            scratch: Scratch::new(name),
        }
    }

    fn root(&self) -> PathBuf {
        self.scratch.root()
    }

    fn session(&self) -> Session {
        let mut session = Session::open(Paths::rooted_at(self.scratch.path())).unwrap();
        session.config_mut().root = Some(self.root());
        session
    }

    fn write_track(&self, relative: &str, title: &str) -> PathBuf {
        self.scratch.write_titled(&self.root(), relative, title)
    }
}

/// Waits for a library change, reloading the index the way a frontend does.
///
/// Generous: filesystem watching involves a settling period by design (§5.2),
/// and on a loaded machine the platform's own notification can lag too.
fn wait_for_change(session: &mut Session, what: &str) -> LibraryDelta {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Some(event) = session.next_event(Duration::from_millis(200))
            && let Event::LibraryChanged(delta) = event
        {
            session.reload_index().unwrap();
            return delta;
        }
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn a_new_file_is_indexed_without_being_asked() {
    let home = Home::new("added");
    home.write_track("a/01.flac", "First");

    let mut session = home.session();
    session.request_rescan().unwrap_err(); // Not watching yet.
    session.start_watching().unwrap();
    wait_for_change(&mut session, "the initial scan");
    assert_eq!(session.index().len(), 1);

    home.write_track("a/02.flac", "Second");
    wait_for_change(&mut session, "the new file");

    assert_eq!(
        session.index().len(),
        2,
        "the added track should be indexed"
    );
    assert!(session.index().tracks().iter().any(|t| t.title == "Second"));
}

#[test]
fn a_deleted_file_is_marked_missing_rather_than_dropped() {
    // §7.4, end to end and unattended: a vanished file is struck through, not
    // removed, so playback can say what will happen to it.
    let home = Home::new("deleted");
    let path = home.write_track("a/01.flac", "First");
    home.write_track("a/02.flac", "Second");

    let mut session = home.session();
    session.start_watching().unwrap();
    wait_for_change(&mut session, "the initial scan");
    assert_eq!(session.index().len(), 2);

    std::fs::remove_file(&path).unwrap();
    wait_for_change(&mut session, "the deletion");

    assert_eq!(session.index().len(), 2, "the row must survive the file");
    let gone = session
        .index()
        .tracks()
        .iter()
        .find(|t| t.title == "First")
        .unwrap();
    assert!(!gone.available);
}

#[test]
fn a_retagged_file_shows_its_new_title() {
    // Retagging in Picard is the ordinary way a library changes after it is
    // first indexed, and the scanner reads every file's tags on every scan --
    // so the new title reaches storage. It has to reach the frontend too, and
    // the only thing that prompts a frontend to re-read is LibraryChanged.
    let home = Home::new("retag");
    home.write_track("a/01.flac", "Original");

    let mut session = home.session();
    session.start_watching().unwrap();
    wait_for_change(&mut session, "the initial scan");
    assert!(
        session
            .index()
            .tracks()
            .iter()
            .any(|t| t.title == "Original"),
        "the fixture did not index"
    );

    home.write_track("a/01.flac", "Retitled");
    wait_for_change(&mut session, "the retag");

    assert!(
        session
            .index()
            .tracks()
            .iter()
            .any(|t| t.title == "Retitled"),
        "a retagged file was written to storage but never announced, so \
         nothing told the frontend to look again"
    );
}

#[test]
fn an_explicit_rescan_works_even_when_nothing_changed() {
    // §5.2: watching *and* an explicit rescan. Watching cannot see everything --
    // a network mount, a filesystem that reports no events -- so the manual
    // path is not a convenience.
    let home = Home::new("explicit");
    home.write_track("a/01.flac", "First");

    let mut session = home.session();
    session.start_watching().unwrap();
    wait_for_change(&mut session, "the initial scan");

    // Written without touching the watched tree's mtimes in a way we rely on:
    // the point is that the request alone is enough.
    home.write_track("a/02.flac", "Second");
    session.request_rescan().unwrap();
    wait_for_change(&mut session, "the requested scan");

    assert_eq!(session.index().len(), 2);
}

#[test]
fn watching_twice_is_harmless() {
    let home = Home::new("twice");
    home.write_track("a/01.flac", "First");

    let mut session = home.session();
    session.start_watching().unwrap();
    session.start_watching().unwrap();
    wait_for_change(&mut session, "the initial scan");
    assert_eq!(session.index().len(), 1);
}

#[test]
fn watching_without_a_root_is_refused_rather_than_started() {
    let home = Home::new("norot");
    let mut session = Session::open(Paths::rooted_at(home.scratch.path())).unwrap();
    assert!(matches!(
        session.start_watching(),
        Err(scoria_core::error::CoreError::NoLibraryRoot)
    ));
}

#[test]
fn dropping_the_session_stops_the_scan_thread() {
    // The handle joins on drop, so this hanging is the failure mode -- a thread
    // left scanning a library nothing is looking at any more.
    let home = Home::new("shutdown");
    home.write_track("a/01.flac", "First");

    let mut session = home.session();
    session.start_watching().unwrap();
    wait_for_change(&mut session, "the initial scan");

    let started = Instant::now();
    drop(session);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "shutdown took {:?}, which suggests the thread was not asked to stop",
        started.elapsed()
    );
}

#[test]
fn the_index_is_stale_until_it_is_reloaded() {
    // The contract `start_watching` documents: the scan runs against its own
    // database handle, so the session's index is a copy that must be told to
    // re-read. A frontend that ignores this shows a library that never changes.
    let home = Home::new("stale");
    home.write_track("a/01.flac", "First");

    let mut session = home.session();
    session.start_watching().unwrap();
    wait_for_change(&mut session, "the initial scan");

    home.write_track("a/02.flac", "Second");

    // Wait for the change without reloading.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seen = false;
    while Instant::now() < deadline && !seen {
        if let Some(Event::LibraryChanged(_)) = session.next_event(Duration::from_millis(200)) {
            seen = true;
        }
    }
    assert!(seen, "timed out waiting for the change");

    assert_eq!(
        session.index().len(),
        1,
        "the index should not update itself"
    );
    session.reload_index().unwrap();
    assert_eq!(
        session.index().len(),
        2,
        "reloading should pick the change up"
    );
}

#[test]
fn scan_progress_is_reported_while_it_runs() {
    let home = Home::new("progress");
    for n in 1..=5 {
        home.write_track(&format!("a/{n:02}.flac"), &format!("Track {n}"));
    }

    let mut session = home.session();
    session.start_watching().unwrap();

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut progress = false;
    while Instant::now() < deadline && !progress {
        if let Some(Event::ScanProgress { total, .. }) =
            session.next_event(Duration::from_millis(200))
        {
            progress = total.is_some();
        }
    }
    assert!(progress, "a background scan should report progress");
}

/// The fixture has to be a real FLAC for any of this to mean anything.
#[test]
fn the_fixture_is_readable() {
    let home = Home::new("fixture");
    let path = home.write_track("a/01.flac", "First");
    assert!(Path::new(&path).exists());
    assert!(scoria_core::library::tags::read(&path).is_ok());
}

#[test]
fn a_scan_that_finds_nothing_still_says_it_finished() {
    // The first-run case: someone points Scoria at a folder with no music in
    // it. There are no deltas, so `LibraryChanged` never arrives -- and without
    // a terminating event a frontend that started saying "scanning" would have
    // no way to learn that it had stopped, and would say it forever.
    let home = Home::new("empty");

    let mut session = home.session();
    session.start_watching().unwrap();

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut finished = false;
    while Instant::now() < deadline && !finished {
        match session.next_event(Duration::from_millis(200)) {
            Some(Event::ScanFinished) => finished = true,
            Some(Event::LibraryChanged(delta)) => {
                panic!("an empty library reported {delta:?}");
            }
            _ => {}
        }
    }

    assert!(finished, "the scan never reported that it had finished");
    assert_eq!(session.index().len(), 0);
}

#[test]
fn a_scan_that_found_something_finishes_after_saying_so() {
    // Order matters: a frontend takes `ScanFinished` as the end of the
    // sequence, so a delta arriving after it would be applied to a library it
    // had already decided was settled.
    let home = Home::new("ordering");
    home.write_track("a/01.flac", "First");

    let mut session = home.session();
    session.start_watching().unwrap();

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    while Instant::now() < deadline && !seen.contains(&Event::ScanFinished) {
        if let Some(event) = session.next_event(Duration::from_millis(200))
            && matches!(event, Event::LibraryChanged(_) | Event::ScanFinished)
        {
            seen.push(event);
        }
    }

    assert_eq!(
        seen.last(),
        Some(&Event::ScanFinished),
        "the sequence was {seen:?}"
    );
    assert!(
        seen.iter().any(|e| matches!(e, Event::LibraryChanged(_))),
        "the scan found a track but reported no change: {seen:?}"
    );
}

#[test]
fn choosing_a_root_starts_the_watching_that_was_asked_for_too_early() {
    // First run, exactly as a frontend does it: ask to watch before there is
    // anything to watch, be refused, and then choose a folder. The request
    // stands, so setting the root is what finally honours it -- and the
    // catch-up scan is what indexes the library.
    //
    // Without this, choosing a folder saved the setting and started nothing:
    // the window sat on "Reading your library. 0 files so far." forever.
    let home = Home::new("firstrun");
    home.write_track("a/01.flac", "First");

    let mut session = Session::open(Paths::rooted_at(home.scratch.path())).unwrap();
    assert!(matches!(
        session.start_watching(),
        Err(scoria_core::error::CoreError::NoLibraryRoot)
    ));

    session.set_root(home.root()).unwrap();
    wait_for_change(&mut session, "the scan of the newly chosen library");

    assert_eq!(session.index().len(), 1);
    // And it keeps watching the new root, rather than scanning it once.
    home.write_track("a/02.flac", "Second");
    wait_for_change(&mut session, "a file added after the root was chosen");
    assert_eq!(session.index().len(), 2);
}

#[test]
fn a_session_that_never_asked_to_watch_does_not_start_on_its_own() {
    // The command line sets a root and scans synchronously. Spawning a watch
    // thread it never asked for would scan the library twice.
    let home = Home::new("unwatched");
    home.write_track("a/01.flac", "First");

    let mut session = Session::open(Paths::rooted_at(home.scratch.path())).unwrap();
    session.set_root(home.root()).unwrap();

    assert!(
        session.request_rescan().is_err(),
        "watching started without being asked for"
    );
}
