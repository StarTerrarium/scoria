//! Query engine integration tests: the filter cascade, sorting and grouping.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use scoria_core::AlbumId;
use scoria_core::date::Date;
use scoria_core::db::{AlbumRepository, NewAlbum, NewTrack, SqliteDatabase, TrackRepository};
use scoria_core::library::AlbumIdentity;
use scoria_core::model::{ReplayGain, Timestamp};
use scoria_core::path::RelativePath;
use scoria_core::query::sort::{SortField, SortKey, SortPreset};
use scoria_core::query::{LibraryIndex, QueryEngine, Selection};
use std::time::Duration;

/// A small library that exercises the cases the design keeps citing.
struct Library {
    db: SqliteDatabase,
    albums: Vec<(String, AlbumId)>,
}

impl Library {
    fn new() -> Self {
        let mut db = SqliteDatabase::open_in_memory().unwrap();
        let mut albums = Vec::new();
        let mut next_path = 0;

        // (album artist, sort name, album, date, track titles)
        let catalogue: &[(&str, &str, &str, &str, &[&str])] = &[
            (
                "Ghost",
                "Ghost",
                "Opus Eponymous",
                "2010",
                &["Con Clavi Con Dio", "Ritual"],
            ),
            (
                "Ghost",
                "Ghost",
                "Meliora",
                "2015",
                &["Spirit", "From the Pinnacle", "Cirice"],
            ),
            (
                "The Omnific",
                "Omnific, The",
                "Escapades",
                "2021",
                &["Kismet"],
            ),
            (
                "花冷え。",
                "HANABIE.",
                "乙女改革",
                "2021",
                &["SUNRISE 味噌SOUP", "Tokyo Kawaii"],
            ),
        ];

        for (artist, sort, album, date, titles) in catalogue {
            let album_id = db
                .upsert_album(&NewAlbum {
                    identity: AlbumIdentity::resolve(
                        None,
                        Some(artist),
                        Some(album),
                        Date::parse(date),
                        None,
                    ),
                    title: (*album).to_owned(),
                    album_artist: (*artist).to_owned(),
                    album_artist_sort: Some((*sort).to_owned()),
                    date: Date::parse(date),
                })
                .unwrap();
            albums.push(((*album).to_owned(), album_id));

            for (n, title) in titles.iter().enumerate() {
                next_path += 1;
                db.upsert_track(&NewTrack {
                    album_id,
                    path: RelativePath::new(format!("lib/{next_path:03}.flac")).unwrap(),
                    title: (*title).to_owned(),
                    artist: (*artist).to_owned(),
                    album_artist: (*artist).to_owned(),
                    album_artist_sort: Some((*sort).to_owned()),
                    album: (*album).to_owned(),
                    date: Date::parse(date),
                    disc: Some(1),
                    track_number: Some(u16::try_from(n + 1).unwrap()),
                    duration: Duration::from_secs(200 + n as u64),
                    replay_gain: ReplayGain::default(),
                    added_at: Timestamp(1_700_000_000 + next_path as i64),
                })
                .unwrap();
            }
        }
        Self { db, albums }
    }

    fn album(&self, title: &str) -> AlbumId {
        self.albums
            .iter()
            .find(|(t, _)| t == title)
            .expect("no such album")
            .1
    }

    fn run(&self, selection: &Selection) -> scoria_core::QueryOutcome {
        let index = LibraryIndex::load(&self.db).unwrap();
        QueryEngine::new().run(&index, &self.db, selection).unwrap()
    }

    fn index(&self) -> LibraryIndex {
        LibraryIndex::load(&self.db).unwrap()
    }
}

fn titles(library: &Library, outcome: &scoria_core::QueryOutcome) -> Vec<String> {
    library
        .index()
        .rows(&outcome.tracks)
        .iter()
        .map(|t| t.title.clone())
        .collect()
}

fn artist_labels(outcome: &scoria_core::QueryOutcome) -> Vec<(String, usize)> {
    outcome
        .album_artists
        .rows
        .iter()
        .map(|r| (r.label.clone(), r.count))
        .collect()
}

// ------------------------------------------------------------ the basics --

#[test]
fn an_unfiltered_selection_returns_the_whole_library() {
    let library = Library::new();
    let outcome = library.run(&Selection::all());
    assert_eq!(outcome.tracks.len(), 8);
    assert_eq!(outcome.total, 8);
}

#[test]
fn results_come_back_in_the_default_sort_order() {
    // §6.5 leans on this: the playing track lands inside its album, with the
    // artist's other albums immediately above and below.
    let library = Library::new();
    let outcome = library.run(&Selection::all());

    assert_eq!(
        titles(&library, &outcome),
        vec![
            // Ghost, oldest album first, then by track number.
            "Con Clavi Con Dio",
            "Ritual",
            "Spirit",
            "From the Pinnacle",
            "Cirice",
            // HANABIE. sorts under H, not by codepoint.
            "SUNRISE 味噌SOUP",
            "Tokyo Kawaii",
            // Omnific, The sorts under O, not under T.
            "Kismet",
        ]
    );
}

// ------------------------------------------------------- §7.2 the cascade --

#[test]
fn every_pane_row_sums_to_its_all_count() {
    // The contract a filter pane makes by existing. §7.2 rejects a genre pane
    // precisely because it cannot keep this promise.
    let library = Library::new();
    let outcome = library.run(&Selection::all());

    let summed: usize = outcome.album_artists.rows.iter().map(|r| r.count).sum();
    assert_eq!(summed, outcome.album_artists.total);

    let summed: usize = outcome.albums.rows.iter().map(|r| r.count).sum();
    assert_eq!(summed, outcome.albums.total);
}

#[test]
fn the_artist_pane_is_ordered_by_sort_name() {
    let library = Library::new();
    let outcome = library.run(&Selection::all());

    assert_eq!(
        artist_labels(&outcome),
        vec![
            ("Ghost".to_owned(), 5),
            ("花冷え。".to_owned(), 2),    // HANABIE.
            ("The Omnific".to_owned(), 1), // Omnific, The
        ]
    );
}

#[test]
fn the_pane_shows_the_display_name_while_ordering_by_the_sort_name() {
    // §4.1 in one assertion: structural value orders, display value is shown.
    let library = Library::new();
    let outcome = library.run(&Selection::all());
    let labels: Vec<&str> = outcome
        .album_artists
        .rows
        .iter()
        .map(|r| r.label.as_str())
        .collect();
    assert!(
        labels.contains(&"花冷え。"),
        "should display the Japanese name"
    );
    assert!(
        labels.contains(&"The Omnific"),
        "should display the leading article"
    );
}

#[test]
fn selecting_an_artist_narrows_the_tracks() {
    let library = Library::new();
    let selection = Selection {
        album_artists: vec!["Ghost".to_owned()],
        ..Selection::all()
    };
    let outcome = library.run(&selection);

    assert_eq!(outcome.tracks.len(), 5);
    assert!(
        library
            .index()
            .rows(&outcome.tracks)
            .iter()
            .all(|t| t.album_artist == "Ghost")
    );
}

#[test]
fn selecting_an_artist_does_not_empty_the_pane_it_was_selected_in() {
    // This is what makes it a cascade rather than a set of independent
    // filters. A pane whose counts collapsed to just the selection would be
    // unusable: there would be no way to change your mind.
    let library = Library::new();
    let selection = Selection {
        album_artists: vec!["Ghost".to_owned()],
        ..Selection::all()
    };
    let outcome = library.run(&selection);

    assert_eq!(
        outcome.album_artists.rows.len(),
        3,
        "all artists should still be listed"
    );
    assert_eq!(
        outcome.album_artists.total, 8,
        "All (n) counts the whole library"
    );
    assert_eq!(artist_labels(&outcome)[0], ("Ghost".to_owned(), 5));
}

#[test]
fn the_selected_row_is_marked() {
    let library = Library::new();
    let selection = Selection {
        album_artists: vec!["Ghost".to_owned()],
        ..Selection::all()
    };
    let outcome = library.run(&selection);

    let selected = outcome.album_artists.selected();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].label, "Ghost");
}

#[test]
fn the_album_pane_narrows_to_the_selected_artist() {
    // Downstream panes *do* narrow: that is the direction the cascade flows.
    let library = Library::new();
    let selection = Selection {
        album_artists: vec!["Ghost".to_owned()],
        ..Selection::all()
    };
    let outcome = library.run(&selection);

    assert_eq!(outcome.albums.rows.len(), 2, "only Ghost's albums");
    assert_eq!(outcome.albums.total, 5);
    assert_eq!(
        outcome.albums.rows[0].label, "Opus Eponymous",
        "oldest first"
    );
    assert_eq!(outcome.albums.rows[1].label, "Meliora");
}

#[test]
fn selecting_an_album_narrows_the_tracks_but_not_its_own_pane() {
    let library = Library::new();
    let selection = Selection {
        album_artists: vec!["Ghost".to_owned()],
        albums: vec![library.album("Meliora")],
        ..Selection::all()
    };
    let outcome = library.run(&selection);

    assert_eq!(outcome.tracks.len(), 3);
    assert!(
        library
            .index()
            .rows(&outcome.tracks)
            .iter()
            .all(|t| t.album == "Meliora")
    );
    assert_eq!(
        outcome.albums.rows.len(),
        2,
        "the album pane still lists both"
    );
}

#[test]
fn selecting_several_artists_unions_them() {
    let library = Library::new();
    let selection = Selection {
        album_artists: vec!["Ghost".to_owned(), "The Omnific".to_owned()],
        ..Selection::all()
    };
    let outcome = library.run(&selection);
    assert_eq!(outcome.tracks.len(), 6);
}

#[test]
fn a_filter_matching_nothing_yields_nothing_without_failing() {
    let library = Library::new();
    let selection = Selection {
        album_artists: vec!["Nobody At All".to_owned()],
        ..Selection::all()
    };
    let outcome = library.run(&selection);

    assert!(outcome.is_empty());
    assert!(outcome.albums.rows.is_empty());
    assert_eq!(
        outcome.album_artists.rows.len(),
        3,
        "the upstream pane is unaffected"
    );
}

// ------------------------------------------------------------- search --

#[test]
fn search_narrows_the_tracks() {
    let library = Library::new();
    let selection = Selection {
        search: Some("Cirice".to_owned()),
        ..Selection::all()
    };
    let outcome = library.run(&selection);

    assert_eq!(titles(&library, &outcome), vec!["Cirice"]);
}

#[test]
fn search_narrows_the_panes_too() {
    // Search sits above the cascade: it changes what every pane counts against.
    let library = Library::new();
    let selection = Selection {
        search: Some("Ghost".to_owned()),
        ..Selection::all()
    };
    let outcome = library.run(&selection);

    assert_eq!(outcome.album_artists.rows.len(), 1);
    assert_eq!(outcome.album_artists.rows[0].label, "Ghost");
    assert_eq!(outcome.album_artists.total, 5);
    assert_eq!(outcome.total, 8, "the library size is unchanged");
}

#[test]
fn search_combines_with_the_cascade() {
    let library = Library::new();
    let selection = Selection {
        album_artists: vec!["Ghost".to_owned()],
        search: Some("Ritual".to_owned()),
        ..Selection::all()
    };
    assert_eq!(titles(&library, &library.run(&selection)), vec!["Ritual"]);
}

#[test]
fn a_blank_search_is_not_a_filter() {
    let library = Library::new();
    let selection = Selection {
        search: Some("   ".to_owned()),
        ..Selection::all()
    };
    assert_eq!(library.run(&selection).tracks.len(), 8);
}

#[test]
fn search_finds_japanese_text() {
    let library = Library::new();
    let selection = Selection {
        search: Some("味噌".to_owned()),
        ..Selection::all()
    };
    assert_eq!(
        titles(&library, &library.run(&selection)),
        vec!["SUNRISE 味噌SOUP"]
    );
}

// ------------------------------------------------------- §7.4 availability --

#[test]
fn unavailable_tracks_are_included_by_default() {
    let mut library = Library::new();
    let id = library.db.all_tracks().unwrap()[0].id;
    library.db.set_available(&[id], false).unwrap();

    let outcome = library.run(&Selection::all());
    assert_eq!(
        outcome.tracks.len(),
        8,
        "§7.4: shown struck through, not dropped"
    );
    assert_eq!(
        library
            .index()
            .rows(&outcome.tracks)
            .iter()
            .filter(|t| !t.available)
            .count(),
        1
    );
}

#[test]
fn unavailable_tracks_can_be_excluded_when_asked() {
    let mut library = Library::new();
    let id = library.db.all_tracks().unwrap()[0].id;
    library.db.set_available(&[id], false).unwrap();

    let selection = Selection {
        include_unavailable: false,
        ..Selection::all()
    };
    let outcome = library.run(&selection);
    assert_eq!(outcome.tracks.len(), 7);
    assert_eq!(outcome.total, 7);
}

// ------------------------------------------------------- §7.3 grouping --

#[test]
fn group_headers_follow_the_primary_sort_key() {
    let library = Library::new();
    let outcome = library.run(&Selection::all());

    let labels: Vec<&str> = outcome.groups.iter().map(|g| g.label.as_str()).collect();
    assert_eq!(labels, vec!["Ghost", "花冷え。", "The Omnific"]);

    assert_eq!(outcome.groups[0].start, 0);
    assert_eq!(outcome.groups[0].len, 5);
    assert_eq!(outcome.groups[0].end(), 5);
}

#[test]
fn groups_cover_every_row_exactly_once() {
    let library = Library::new();
    let outcome = library.run(&Selection::all());

    let covered: usize = outcome.groups.iter().map(|g| g.len).sum();
    assert_eq!(covered, outcome.tracks.len());

    let mut next = 0;
    for group in &outcome.groups {
        assert_eq!(group.start, next, "groups must be contiguous");
        next = group.end();
    }
}

#[test]
fn sorting_by_duration_switches_grouping_off() {
    // §7.3 states the outcome; it falls out of the field rather than a case.
    let library = Library::new();
    let selection = Selection {
        sort: scoria_core::SortChain::new(vec![SortKey::asc(SortField::Duration)]),
        ..Selection::all()
    };
    let outcome = library.run(&selection);

    assert_eq!(outcome.tracks.len(), 8);
    assert!(outcome.groups.is_empty());
}

#[test]
fn grouping_by_album_shows_the_album_title_not_its_date() {
    // Album sorts on date and displays the title -- §4.4's whole point.
    let library = Library::new();
    let selection = Selection {
        sort: scoria_core::SortChain::new(vec![SortKey::asc(SortField::Album)]),
        ..Selection::all()
    };
    let outcome = library.run(&selection);

    let labels: Vec<&str> = outcome.groups.iter().map(|g| g.label.as_str()).collect();
    assert_eq!(labels[0], "Opus Eponymous", "2010, the oldest");
}

// ---------------------------------------------------------- sort presets --

#[test]
fn every_preset_returns_the_whole_library_in_some_order() {
    let library = Library::new();
    for preset in SortPreset::all() {
        let selection = Selection {
            sort: preset.chain(),
            ..Selection::all()
        };
        let outcome = library.run(&selection);
        assert_eq!(outcome.tracks.len(), 8, "{} lost rows", preset.label());

        let mut ids = outcome.tracks.clone();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 8, "{} duplicated rows", preset.label());
    }
}

#[test]
fn the_same_selection_always_returns_the_same_order() {
    // Reproducibility is what makes a queue snapshot reproducible (§4.4).
    let library = Library::new();
    let first = library.run(&Selection::all()).tracks;
    let second = library.run(&Selection::all()).tracks;
    assert_eq!(first, second);
}

// --------------------------------------------------------- storage filter --

#[test]
fn an_artist_name_containing_a_quote_is_a_value_not_syntax() {
    let mut library = Library::new();
    let album_id = library
        .db
        .upsert_album(&NewAlbum {
            identity: AlbumIdentity::resolve(None, Some("O'Hara's"), Some("Quote'd"), None, None),
            title: "Quote'd".to_owned(),
            album_artist: "O'Hara's".to_owned(),
            album_artist_sort: None,
            date: None,
        })
        .unwrap();
    library
        .db
        .upsert_track(&NewTrack {
            album_id,
            path: RelativePath::new("q/1.flac").unwrap(),
            title: "Quoted".to_owned(),
            artist: "O'Hara's".to_owned(),
            album_artist: "O'Hara's".to_owned(),
            album_artist_sort: None,
            album: "Quote'd".to_owned(),
            date: None,
            disc: None,
            track_number: Some(1),
            duration: Duration::from_secs(100),
            replay_gain: ReplayGain::default(),
            added_at: Timestamp(0),
        })
        .unwrap();

    let selection = Selection {
        album_artists: vec!["O'Hara's".to_owned()],
        ..Selection::all()
    };
    assert_eq!(titles(&library, &library.run(&selection)), vec!["Quoted"]);
}

#[test]
fn an_empty_library_queries_cleanly() {
    let db = SqliteDatabase::open_in_memory().unwrap();
    let index = LibraryIndex::load(&db).unwrap();
    let outcome = QueryEngine::new()
        .run(&index, &db, &Selection::all())
        .unwrap();

    assert!(outcome.is_empty());
    assert!(outcome.groups.is_empty());
    assert_eq!(outcome.album_artists.total, 0);
    assert_eq!(outcome.total, 0);
}

// ------------------------------------------- what a column-header sort does --

#[test]
fn sorting_by_album_title_is_alphabetical_not_chronological() {
    // The two fields exist because they answer different questions, and using
    // the wrong one is invisible until you read the result: chronological order
    // shown as album titles looks like no order at all.
    let library = Library::new();

    let alphabetical = Selection {
        sort: scoria_core::SortChain::new(vec![SortKey::asc(SortField::AlbumTitle)]),
        ..Selection::all()
    };
    let albums: Vec<String> = library
        .index()
        .rows(&library.run(&alphabetical).tracks)
        .iter()
        .map(|t| t.album.clone())
        .collect();

    let mut expected = albums.clone();
    expected.sort_by_key(|a| a.to_lowercase());
    assert_eq!(albums, expected, "album titles should read A to Z");

    // And the other field still does what the default preset needs.
    let chronological = Selection {
        sort: scoria_core::SortChain::new(vec![SortKey::asc(SortField::Album)]),
        ..Selection::all()
    };
    let years: Vec<i16> = library
        .index()
        .rows(&library.run(&chronological).tracks)
        .iter()
        .filter_map(|t| t.date.map(|d| d.year()))
        .collect();

    let mut ordered = years.clone();
    ordered.sort_unstable();
    assert_eq!(years, ordered, "the Album field should still order by date");
}
