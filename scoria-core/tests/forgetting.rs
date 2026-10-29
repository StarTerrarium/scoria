//! Forgetting missing tracks (§7.4).
//!
//! Driven through `Session` against a real library on disk, because what has
//! to hold is the whole sequence a frontend sees: scan, a file goes, rescan
//! marks it, forgetting removes it and says so -- and a library that is only
//! unplugged is refused rather than emptied.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lofty::prelude::ItemKey;
use scoria_core::config::Paths;
use scoria_core::error::CoreError;
use scoria_core::event::{Event, LibraryDelta};
use scoria_core::session::{Forgotten, Session};
use std::path::PathBuf;
use std::time::Duration;

mod common;
use common::Scratch;

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

    fn write(&self, relative: &str, album: &str, title: &str) -> PathBuf {
        self.scratch.write_track(
            &self.root(),
            relative,
            &[
                (ItemKey::TrackTitle, title),
                (ItemKey::AlbumArtist, "Someone"),
                (ItemKey::AlbumTitle, album),
            ],
        )
    }
}

fn rescan(session: &mut Session) {
    session.rescan(&mut |_| {}).unwrap();
    drain(session);
}

/// Every event already queued.
fn drain(session: &mut Session) -> Vec<Event> {
    std::iter::from_fn(|| session.next_event(Duration::from_millis(10))).collect()
}

fn albums(session: &Session) -> Vec<String> {
    let mut albums: Vec<String> = session
        .index()
        .tracks()
        .iter()
        .map(|t| t.album.clone())
        .collect();
    albums.sort();
    albums.dedup();
    albums
}

#[test]
fn forgetting_removes_only_the_missing_tracks_and_the_albums_they_empty() {
    let home = Home::new("forget-some");
    let old_a = home.write("old/01.flac", "Old", "Gone One");
    let old_b = home.write("old/02.flac", "Old", "Gone Two");
    home.write("new/01.flac", "New", "Kept");
    let partial = home.write("mixed/01.flac", "Mixed", "Half Gone");
    home.write("mixed/02.flac", "Mixed", "Half Kept");

    let mut session = home.session();
    rescan(&mut session);
    assert_eq!(session.index().len(), 5);

    for path in [&old_a, &old_b, &partial] {
        std::fs::remove_file(path).unwrap();
    }
    rescan(&mut session);
    assert_eq!(session.index().len(), 5, "a scan marks, it never deletes");

    let forgotten = session.forget_unavailable().unwrap();
    assert_eq!(
        forgotten,
        Forgotten {
            tracks: 3,
            albums: 1
        }
    );

    let titles: Vec<&str> = session
        .index()
        .tracks()
        .iter()
        .map(|t| t.title.as_str())
        .collect();
    assert_eq!(titles.len(), 2);
    assert!(titles.contains(&"Kept"));
    assert!(titles.contains(&"Half Kept"));
    assert!(session.index().tracks().iter().all(|t| t.available));
    assert_eq!(albums(&session), ["Mixed", "New"]);

    let removed: Vec<_> = drain(&mut session)
        .into_iter()
        .filter_map(|e| match e {
            Event::LibraryChanged(LibraryDelta::TracksRemoved(ids)) => Some(ids.len()),
            _ => None,
        })
        .collect();
    assert_eq!(removed, [3], "a frontend learns what went, once");
}

#[test]
fn forgetting_when_nothing_is_missing_does_nothing_and_says_nothing() {
    let home = Home::new("forget-none");
    home.write("a/01.flac", "A", "Here");

    let mut session = home.session();
    rescan(&mut session);

    assert_eq!(session.forget_unavailable().unwrap(), Forgotten::default());
    assert_eq!(session.index().len(), 1);
    assert!(
        drain(&mut session).is_empty(),
        "nothing changed, so a frontend has nothing to reload"
    );
}

#[test]
fn an_unreachable_library_is_refused_rather_than_emptied() {
    // An unplugged drive makes every track read as missing. That is the one
    // case where forgetting them would be a mistake nobody meant to make.
    let home = Home::new("forget-unplugged");
    home.write("a/01.flac", "A", "One");
    home.write("a/02.flac", "A", "Two");

    let mut session = home.session();
    rescan(&mut session);

    // Rescanning an absent root may fail or mark everything missing; either
    // way, nothing may be forgotten while the root is gone.
    std::fs::remove_dir_all(home.root()).unwrap();
    assert!(!session.library_root_reachable());
    let _ = session.rescan(&mut |_| {});
    drain(&mut session);

    assert!(matches!(
        session.forget_unavailable(),
        Err(CoreError::LibraryUnreachable { .. })
    ));
    assert_eq!(session.index().len(), 2, "the rows survive the refusal");
}

#[test]
fn a_forgotten_file_that_comes_back_is_found_again() {
    let home = Home::new("forget-return");
    let path = home.write("a/01.flac", "A", "Wanderer");
    let mut session = home.session();
    rescan(&mut session);

    let saved = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    rescan(&mut session);
    session.forget_unavailable().unwrap();
    assert_eq!(session.index().len(), 0);

    std::fs::write(&path, saved).unwrap();
    rescan(&mut session);
    let back = session.index().tracks().first().unwrap();
    assert_eq!(back.title, "Wanderer");
    assert!(back.available);
}
