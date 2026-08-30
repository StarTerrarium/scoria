//! Integration tests for storage: schema, repositories, search and config.
//!
//! These drive `SqliteDatabase` through the repository traits only, which is
//! how everything above `db/` will use it.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use scoria_core::config::{LibraryConfig, Paths, PlayBehaviour};
use scoria_core::date::Date;
use scoria_core::db::{AlbumRepository, NewAlbum, NewTrack, SqliteDatabase, TrackRepository};
use scoria_core::error::CoreError;
use scoria_core::id::TrackId;
use scoria_core::library::AlbumIdentity;
use scoria_core::model::{ReplayGain, Timestamp};
use scoria_core::path::RelativePath;
use scoria_core::query::sort::SortPreset;
use std::path::Path;
use std::time::Duration;

mod common;
use common::{Scratch, db};

fn album(db: &mut SqliteDatabase, artist: &str, title: &str, date: &str) -> scoria_core::AlbumId {
    db.upsert_album(&NewAlbum {
        identity: AlbumIdentity::resolve(None, Some(artist), Some(title), Date::parse(date), None),
        title: title.to_owned(),
        album_artist: artist.to_owned(),
        album_artist_sort: Some(artist.to_owned()),
        date: Date::parse(date),
    })
    .unwrap()
}

fn new_track(album_id: scoria_core::AlbumId, path: &str, title: &str) -> NewTrack {
    NewTrack {
        album_id,
        path: RelativePath::new(path).unwrap(),
        title: title.to_owned(),
        artist: "Artist".to_owned(),
        album_artist: "Artist".to_owned(),
        album_artist_sort: Some("Artist".to_owned()),
        album: "Album".to_owned(),
        date: Date::parse("2020"),
        disc: Some(1),
        track_number: Some(1),
        duration: Duration::from_secs(200),
        replay_gain: ReplayGain::default(),
        added_at: Timestamp(1_700_000_000),
        genre: None,
        composer: None,
        audio: scoria_core::model::AudioProperties::default(),
    }
}

// ------------------------------------------------------------------- schema --

#[test]
fn a_fresh_database_is_migrated_to_the_latest_schema() {
    let db = db();
    assert_eq!(
        db.schema_version().unwrap(),
        scoria_core::db::schema::latest_version()
    );
}

#[test]
fn counting_a_play_leaves_the_track_findable() {
    // The trigger was narrowed; searching must still work either side of the
    // statements that no longer fire it.
    let mut db = searchable();
    let id = db.search_tracks("Hurrikan").unwrap()[0];

    db.record_play(id).unwrap();
    db.set_available(&[id], false).unwrap();
    db.set_available(&[id], true).unwrap();

    assert_eq!(
        db.search_tracks("Hurrikan").unwrap(),
        vec![id],
        "the search index lost a track that was only played and re-marked"
    );
}

#[test]
fn a_database_written_by_an_older_version_keeps_its_tracks() {
    // Every column since v1 arrived by ALTER TABLE, which is the migration
    // shape most likely to be got wrong -- a rebuild would take the library
    // with it. What has to survive is the rows: they cost a full rescan to
    // replace, and an append-only migration list exists so they never have to
    // be.
    //
    // The old shape is produced by dropping the v2 columns from a current
    // database and winding the version back, which is closer to a real upgrade
    // than a hand-copied v1 schema that would drift from the real one.
    let scratch = Scratch::new("db");
    let dir = scratch.path();
    let path = dir.join("library.db");

    {
        let mut db = SqliteDatabase::open(&path).unwrap();
        let album_id = album(&mut db, "Artist", "Album", "2020");
        db.upsert_track(&new_track(album_id, "a/01.flac", "Before"))
            .unwrap();
    }
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        // Everything added since v1. One line to extend when a migration adds
        // a column, and the test then covers that migration too.
        for column in [
            "genre",
            "composer",
            "bitrate_kbps",
            "sample_rate_hz",
            "bit_depth",
            "play_count",
        ] {
            conn.execute_batch(&format!("ALTER TABLE tracks DROP COLUMN {column}"))
                .unwrap();
        }
        conn.execute_batch("PRAGMA user_version = 1").unwrap();
    }

    let db = SqliteDatabase::open(&path).unwrap();
    assert_eq!(
        db.schema_version().unwrap(),
        scoria_core::db::schema::latest_version(),
        "the database was not brought forward"
    );

    let tracks = db.all_tracks().unwrap();
    assert_eq!(tracks.len(), 1, "the migration lost the library");
    assert_eq!(tracks[0].title, "Before");
    // Nothing backfills. The scanner reads every file's tags on every scan, so
    // the next one fills these in; until then they are empty rather than wrong.
    assert_eq!(tracks[0].genre, None);
    assert_eq!(tracks[0].audio.sample_rate_hz, None);
    // Play counts start at nought rather than at "unknown": there is no honest
    // number to reconstruct for a library that predates the column.
    assert_eq!(tracks[0].play_count, 0);
}

#[test]
fn a_rescan_cannot_reset_a_play_count() {
    // The whole reason play_count is not a field on NewTrack. A scan writes
    // every other column, and a counter that resets whenever the library is
    // rescanned -- which happens on its own, in the background -- would be
    // worse than not keeping one.
    let mut db = db();
    let album_id = album(&mut db, "Artist", "Album", "2020");
    let id = db
        .upsert_track(&new_track(album_id, "a/01.flac", "Title"))
        .unwrap();

    assert_eq!(db.record_play(id).unwrap(), 1);
    assert_eq!(db.record_play(id).unwrap(), 2);

    // The same file seen again by a scan, retagged.
    let mut retagged = new_track(album_id, "a/01.flac", "Retitled");
    retagged.artist = "Someone Else".to_owned();
    let same = db.upsert_track(&retagged).unwrap();
    assert_eq!(same, id, "the upsert should have matched on path");

    let track = db.track(id).unwrap().unwrap();
    assert_eq!(track.title, "Retitled", "the rescan did not update the row");
    assert_eq!(track.play_count, 2, "the rescan reset the play count");
}

#[test]
fn counting_a_play_of_a_track_that_is_gone_is_an_error_not_a_silent_nothing() {
    let mut db = db();
    let missing = TrackId(9999);
    assert!(matches!(
        db.record_play(missing),
        Err(CoreError::NoSuchTrack(_))
    ));
}

#[test]
fn migration_is_idempotent() {
    let scratch = Scratch::new("db");
    let dir = scratch.path();
    let path = dir.join("library.db");

    let first = SqliteDatabase::open(&path).unwrap();
    let version = first.schema_version().unwrap();
    drop(first);

    let second = SqliteDatabase::open(&path).unwrap();
    assert_eq!(second.schema_version().unwrap(), version);
}

#[test]
fn a_database_from_a_newer_build_is_refused_rather_than_mangled() {
    let scratch = Scratch::new("db");
    let dir = scratch.path();
    let path = dir.join("library.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 9999_i64).unwrap();
    }
    let err = SqliteDatabase::open(&path).unwrap_err();
    assert!(matches!(err, CoreError::Database(_)), "got {err:?}");
    assert!(format!("{err}").contains("newer Scoria"));
}

// ------------------------------------------------------- tracks and albums --

#[test]
fn a_track_survives_a_round_trip_through_storage() {
    let mut db = db();
    let album_id = album(&mut db, "花冷え。", "乙女改革", "2021-01-14");

    let mut track = new_track(
        album_id,
        "花冷え。/乙女改革/01 SUNRISE 味噌SOUP.flac",
        "SUNRISE 味噌SOUP",
    );
    track.album_artist = "花冷え。".to_owned();
    track.album_artist_sort = Some("HANABIE.".to_owned());
    track.album = "乙女改革".to_owned();
    track.date = Date::parse("2021-01-14");
    track.replay_gain = ReplayGain {
        track_gain_db: Some(-9.95),
        track_peak: Some(1.0),
        album_gain_db: Some(-9.69),
        album_peak: Some(1.0),
    };

    let id = db.upsert_track(&track).unwrap();
    let stored = db.track(id).unwrap().unwrap();

    assert_eq!(stored.title, "SUNRISE 味噌SOUP");
    assert_eq!(stored.album_artist, "花冷え。");
    assert_eq!(stored.album_artist_sort.as_deref(), Some("HANABIE."));
    assert_eq!(stored.date, Date::parse("2021-01-14"));
    assert_eq!(stored.duration, Duration::from_secs(200));
    assert_eq!(stored.replay_gain.track_gain_db, Some(-9.95));
    assert_eq!(stored.replay_gain.album_gain_db, Some(-9.69));
    assert!(stored.available);
}

#[test]
fn a_track_with_no_optional_tags_round_trips_as_absent() {
    let mut db = db();
    let album_id = album(&mut db, "Unknown", "Untitled", "2020");

    let mut track = new_track(album_id, "misc/rip.flac", "Rip");
    track.album_artist_sort = None;
    track.date = None;
    track.disc = None;
    track.track_number = None;
    track.replay_gain = ReplayGain::default();

    let id = db.upsert_track(&track).unwrap();
    let stored = db.track(id).unwrap().unwrap();

    assert_eq!(stored.album_artist_sort, None);
    assert_eq!(stored.date, None);
    assert_eq!(stored.disc, None);
    assert_eq!(stored.track_number, None);
    assert!(!stored.replay_gain.is_tagged());
}

#[test]
fn rescanning_the_same_path_updates_rather_than_duplicates() {
    let mut db = db();
    let album_id = album(&mut db, "Ghost", "Meliora", "2015");

    let first = db
        .upsert_track(&new_track(album_id, "Ghost/Meliora/01.flac", "Spirit"))
        .unwrap();

    let mut retagged = new_track(album_id, "Ghost/Meliora/01.flac", "Spirit (Remastered)");
    retagged.title = "Spirit (Remastered)".to_owned();
    let second = db.upsert_track(&retagged).unwrap();

    assert_eq!(
        first, second,
        "the same file should keep its id across a rescan"
    );
    assert_eq!(db.track_count().unwrap(), 1);
    assert_eq!(
        db.track(first).unwrap().unwrap().title,
        "Spirit (Remastered)"
    );
}

#[test]
fn an_album_identity_resolves_to_one_album_however_often_it_is_seen() {
    let mut db = db();
    let first = album(&mut db, "Ghost", "Meliora", "2015");
    let second = album(&mut db, "Ghost", "Meliora", "2015");
    assert_eq!(first, second);
    assert_eq!(db.album_count().unwrap(), 1);
}

#[test]
fn albums_can_be_looked_up_by_identity() {
    let mut db = db();
    let id = album(&mut db, "Ghost", "Meliora", "2015");
    let identity = AlbumIdentity::resolve(
        None,
        Some("Ghost"),
        Some("Meliora"),
        Date::parse("2015"),
        None,
    );
    assert_eq!(db.album_by_identity(&identity).unwrap(), Some(id));

    let absent = AlbumIdentity::resolve(None, Some("Nobody"), Some("Nothing"), None, None);
    assert_eq!(db.album_by_identity(&absent).unwrap(), None);
}

#[test]
fn missing_files_are_marked_unavailable_not_deleted() {
    // §7.4: silently dropping rows makes users doubt the software.
    let mut db = db();
    let album_id = album(&mut db, "Artist", "Album", "2020");
    let id = db
        .upsert_track(&new_track(album_id, "a/b/1.flac", "t"))
        .unwrap();

    db.set_available(&[id], false).unwrap();
    let stored = db.track(id).unwrap().unwrap();
    assert!(!stored.available);
    assert_eq!(db.track_count().unwrap(), 1, "the row must still be there");
}

#[test]
fn a_reappearing_file_becomes_available_again_on_rescan() {
    let mut db = db();
    let album_id = album(&mut db, "Artist", "Album", "2020");
    let track = new_track(album_id, "a/b/1.flac", "t");
    let id = db.upsert_track(&track).unwrap();
    db.set_available(&[id], false).unwrap();

    db.upsert_track(&track).unwrap();
    assert!(db.track(id).unwrap().unwrap().available);
}

#[test]
fn deleting_an_album_takes_its_tracks_with_it() {
    let mut db = db();
    let album_id = album(&mut db, "Artist", "Album", "2020");
    let id = db
        .upsert_track(&new_track(album_id, "a/b/1.flac", "t"))
        .unwrap();

    db.delete_tracks(&[id]).unwrap();
    assert_eq!(db.remove_empty_albums().unwrap(), 1);
    assert_eq!(db.album_count().unwrap(), 0);
}

#[test]
fn albums_that_still_have_tracks_are_kept() {
    let mut db = db();
    let album_id = album(&mut db, "Artist", "Album", "2020");
    db.upsert_track(&new_track(album_id, "a/b/1.flac", "t"))
        .unwrap();
    assert_eq!(db.remove_empty_albums().unwrap(), 0);
}

// ---------------------------------------------------------------- §4.3 paths --

#[test]
fn no_absolute_path_can_reach_storage() {
    // The type makes this unrepresentable, so there is no NewTrack that could
    // carry one. This asserts the boundary rather than the constructor.
    assert!(RelativePath::new("/home/someone/Music/a.flac").is_err());
    assert!(RelativePath::new("../escape.flac").is_err());

    let mut db = db();
    let album_id = album(&mut db, "Artist", "Album", "2020");
    let id = db
        .upsert_track(&new_track(album_id, "Artist/Album/01.flac", "t"))
        .unwrap();

    for (_, path) in db.all_track_paths().unwrap() {
        assert!(!path.as_path().is_absolute());
    }
    assert_eq!(
        db.track(id).unwrap().unwrap().path.to_string(),
        "Artist/Album/01.flac"
    );
}

#[test]
fn a_hand_edited_absolute_path_is_reported_not_panicked_on() {
    let scratch = Scratch::new("db");
    let dir = scratch.path();
    let path = dir.join("library.db");
    let db = SqliteDatabase::open(&path).unwrap();
    drop(db);

    // Bypass the repository the way a corrupted file or a foreign tool would.
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "INSERT INTO albums (id, identity, title, album_artist) VALUES (1, 'x', 't', 'a')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tracks (id, album_id, path, title, artist, album_artist, album,
                                 duration_ms, added_at, available)
             VALUES (1, 1, '/etc/passwd', 't', 'a', 'a', 'al', 1000, 0, 1)",
            [],
        )
        .unwrap();
    }

    let db = SqliteDatabase::open(&path).unwrap();
    let err = db.track(TrackId(1)).unwrap_err();
    assert!(matches!(err, CoreError::CorruptData(_)), "got {err:?}");
}

// ---------------------------------------------------------------- search --

fn searchable() -> SqliteDatabase {
    let mut db = db();
    let a = album(&mut db, "Electric Callboy", "TEKKNO", "2022");
    let b = album(&mut db, "花冷え。", "乙女改革", "2021");

    for (path, title, artist, album_artist, album_title) in [
        (
            "ec/tekkno/01.flac",
            "Hurrikan",
            "Electric Callboy",
            "Electric Callboy",
            "TEKKNO",
        ),
        (
            "ec/tekkno/02.flac",
            "Pump It",
            "Electric Callboy",
            "Electric Callboy",
            "TEKKNO",
        ),
        (
            "hb/otome/01.flac",
            "SUNRISE 味噌SOUP",
            "花冷え。",
            "花冷え。",
            "乙女改革",
        ),
    ] {
        let album_id = if path.starts_with("ec/") { a } else { b };
        let mut track = new_track(album_id, path, title);
        track.artist = artist.to_owned();
        track.album_artist = album_artist.to_owned();
        track.album = album_title.to_owned();
        db.upsert_track(&track).unwrap();
    }
    db
}

#[test]
fn search_matches_whole_words() {
    let db = searchable();
    assert_eq!(db.search_tracks("Hurrikan").unwrap().len(), 1);
    assert_eq!(db.search_tracks("callboy").unwrap().len(), 2);
}

#[test]
fn search_matches_substrings() {
    // What the trigram tokenizer buys over unicode61, and why it was chosen:
    // a search box that needs whole words is not a search box.
    let db = searchable();
    assert_eq!(db.search_tracks("lect").unwrap().len(), 2);
    assert_eq!(db.search_tracks("urrika").unwrap().len(), 1);
}

#[test]
fn search_ignores_case() {
    let db = searchable();
    assert_eq!(db.search_tracks("HURRIKAN").unwrap().len(), 1);
    assert_eq!(db.search_tracks("hurrikan").unwrap().len(), 1);
}

#[test]
fn search_finds_japanese_titles() {
    let db = searchable();
    assert_eq!(db.search_tracks("花冷え").unwrap().len(), 1);
    assert_eq!(db.search_tracks("乙女改革").unwrap().len(), 1);
}

#[test]
fn short_queries_still_work_despite_the_trigram_minimum() {
    // The index cannot match fewer than three characters, so the repository
    // falls back to a scan. Two-character queries are ordinary in Japanese.
    let db = searchable();
    assert_eq!(
        db.search_tracks("味噌").unwrap().len(),
        1,
        "two-character CJK query"
    );
    assert_eq!(
        db.search_tracks("宇").unwrap().len(),
        0,
        "absent single character"
    );
    assert_eq!(
        db.search_tracks("It").unwrap().len(),
        1,
        "two-character latin query"
    );
}

#[test]
fn search_treats_punctuation_as_text_not_syntax() {
    // FTS5 MATCH has its own query language; a user typing these means the
    // characters, and an unquoted query would be a syntax error.
    let db = searchable();
    for query in ["\"", "AC/DC", "-", "OR", "a OR b", "NEAR(", "*"] {
        assert!(
            db.search_tracks(query).is_ok(),
            "query {query:?} should not error"
        );
    }
}

#[test]
fn an_empty_search_returns_nothing() {
    let db = searchable();
    assert!(db.search_tracks("").unwrap().is_empty());
    assert!(db.search_tracks("   ").unwrap().is_empty());
}

#[test]
fn the_search_index_follows_retagging() {
    // External-content FTS holds no copy of the data, so it only stays correct
    // if every write is mirrored by a trigger.
    let mut db = searchable();
    let id = db.search_tracks("Hurrikan").unwrap()[0];
    let stored = db.track(id).unwrap().unwrap();

    let mut retagged = new_track(stored.album_id, "ec/tekkno/01.flac", "Spaceman");
    retagged.title = "Spaceman".to_owned();
    retagged.artist = stored.artist.clone();
    retagged.album_artist = stored.album_artist.clone();
    retagged.album = stored.album.clone();
    db.upsert_track(&retagged).unwrap();

    assert!(
        db.search_tracks("Hurrikan").unwrap().is_empty(),
        "stale entry left in the index"
    );
    assert_eq!(db.search_tracks("Spaceman").unwrap().len(), 1);
}

#[test]
fn the_search_index_follows_deletion() {
    let mut db = searchable();
    let id = db.search_tracks("Hurrikan").unwrap()[0];
    db.delete_tracks(&[id]).unwrap();
    assert!(db.search_tracks("Hurrikan").unwrap().is_empty());
}

// ---------------------------------------------------------------- config --

#[test]
fn a_missing_config_file_yields_defaults() {
    let scratch = Scratch::new("definitely-absent");
    let path = scratch.join("definitely-absent.toml");
    let _ = std::fs::remove_file(&path);

    let config = scoria_core::config::load_library(&path).unwrap();
    assert_eq!(config, LibraryConfig::default());
    assert_eq!(
        config.play_behaviour,
        PlayBehaviour::WhatsShown,
        "§6.3 default"
    );
    assert_eq!(config.sort, SortPreset::ArtistChronological, "§7.3 default");
}

#[test]
fn config_survives_a_round_trip() {
    let scratch = Scratch::new("round-trip");
    let path = scratch.join("round-trip.toml");
    // Built through the setter rather than as a literal: volume is private so
    // that nothing can store a value outside the range.
    let mut config = LibraryConfig::default();
    config.root = Some("/home/someone/Music".into());
    config.play_behaviour = PlayBehaviour::WholeLibrary;
    config.sort = SortPreset::Year;
    config.set_volume(0.4);
    scoria_core::config::save_library(&path, &config).unwrap();
    assert_eq!(scoria_core::config::load_library(&path).unwrap(), config);
}

#[test]
fn a_malformed_config_is_reported_rather_than_silently_reset() {
    // Quietly discarding settings because of a typo is worse than refusing to
    // start and saying which line is wrong.
    let scratch = Scratch::new("malformed");
    let path = scratch.join("malformed.toml");
    std::fs::write(&path, "root = [this is not toml").unwrap();

    let err = scoria_core::config::load_library(&path).unwrap_err();
    assert!(matches!(err, CoreError::Config(_)), "got {err:?}");
}

#[test]
fn an_unset_root_is_an_actionable_error() {
    let err = LibraryConfig::default().root().unwrap_err();
    assert!(matches!(err, CoreError::NoLibraryRoot));
}

#[test]
fn saving_creates_the_config_directory() {
    let scratch = Scratch::new("nested-config");
    let path = scratch.join("nested").join("deeper").join("config.toml");
    let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());

    scoria_core::config::save_library(&path, &LibraryConfig::default()).unwrap();
    assert!(path.exists());
}

#[test]
fn saving_leaves_no_temporary_file_behind() {
    let scratch = Scratch::new("atomic");
    let path = scratch.join("atomic.toml");
    scoria_core::config::save_library(&path, &LibraryConfig::default()).unwrap();
    assert!(!path.with_extension("toml.tmp").exists());
}

#[test]
fn a_rooted_install_keeps_everything_under_one_directory() {
    let paths = Paths::rooted_at("/tmp/scoria-example");
    assert_eq!(
        paths.config_file(),
        Path::new("/tmp/scoria-example/config/config.toml")
    );
    assert_eq!(
        paths.database(),
        Path::new("/tmp/scoria-example/data/library.db")
    );
    assert_eq!(
        paths.artwork_cache(),
        Path::new("/tmp/scoria-example/cache/artwork")
    );
}

#[test]
fn xdg_paths_are_namespaced_and_absolute() {
    // from_env reads the process environment, which tests cannot safely mutate
    // in parallel, so this asserts the shape rather than a specific value.
    let Ok(paths) = Paths::from_env() else {
        return; // No HOME and no XDG_* -- nothing to check.
    };
    for dir in [paths.config_dir(), paths.data_dir(), paths.cache_dir()] {
        assert!(dir.is_absolute(), "{} should be absolute", dir.display());
        assert!(
            dir.ends_with("scoria"),
            "{} should be namespaced",
            dir.display()
        );
    }
    assert!(paths.database().ends_with("scoria/library.db"));
}

// --------------------------------------------------- sections (one file, two owners) --

#[test]
fn writing_one_section_leaves_the_others_alone() {
    // The property that lets core and a frontend share a settings file. Neither
    // can name the other's types, so neither can serialise the whole document;
    // writing it wholesale from either side would delete the other's settings.
    let scratch = Scratch::new("sections");
    let path = scratch.join("sections.toml");
    let _ = std::fs::remove_file(&path);

    let mut library = LibraryConfig::default();
    library.root = Some("/home/someone/Music".into());
    library.play_behaviour = PlayBehaviour::WholeLibrary;
    library.sort = SortPreset::Year;
    scoria_core::config::save_library(&path, &library).unwrap();

    #[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    struct Ui {
        theme: String,
        columns: Vec<String>,
    }
    let ui = Ui {
        theme: "Scoria Dark".to_owned(),
        columns: vec!["title".to_owned()],
    };
    scoria_core::config::save_section(&path, "ui", &ui).unwrap();

    // Both survive.
    assert_eq!(scoria_core::config::load_library(&path).unwrap(), library);
    assert_eq!(
        scoria_core::config::load_section::<Ui>(&path, "ui").unwrap(),
        ui
    );
}

#[test]
fn saving_the_library_settings_leaves_the_frontend_s_alone() {
    // The direction the original test did not cover, and the one that
    // happens: the frontend saves [ui] as the window is used, and core saves
    // whenever the volume settles. Core used to serialise a bare
    // LibraryConfig over the whole document, so every volume change took the
    // theme, placement, columns, sort and drawer heights with it.
    let scratch = Scratch::new("ui-then-library");
    let path = scratch.join("ui-then-library.toml");
    let _ = std::fs::remove_file(&path);

    #[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    struct Ui {
        theme: String,
    }
    let ui = Ui {
        theme: "Scoria Dark".to_owned(),
    };
    scoria_core::config::save_section(&path, "ui", &ui).unwrap();

    // Now core saves, as it does on every settled volume change.
    let mut library = LibraryConfig::default();
    library.set_volume(0.4);
    scoria_core::config::save_library(&path, &library).unwrap();

    assert_eq!(
        scoria_core::config::load_section::<Ui>(&path, "ui").unwrap(),
        ui,
        "saving the library settings deleted the frontend's"
    );
    let saved = scoria_core::config::load_library(&path).unwrap().volume();
    assert!(
        (saved - 0.4).abs() < f32::EPSILON,
        "volume came back as {saved}"
    );
}

#[test]
fn a_file_written_before_the_library_section_existed_is_moved_into_it() {
    // Core used to write its settings at the top level. Such a file has to
    // keep working, and the stray keys have to go -- a dead `root` sitting
    // above the live one is a trap for anyone editing this by hand (§7.3).
    let scratch = Scratch::new("legacy-layout");
    let path = scratch.join("legacy-layout.toml");
    std::fs::write(
        &path,
        "root = \"/home/someone/Music\"\nplay-behaviour = \"whole-library\"\nvolume = 0.25\n\n[ui]\ntheme = \"Scoria Dark\"\n",
    )
    .unwrap();

    let config = scoria_core::config::load_library(&path).unwrap();
    assert_eq!(config.root.as_deref(), Some(Path::new("/home/someone/Music")));
    assert_eq!(config.play_behaviour, PlayBehaviour::WholeLibrary);
    assert!((config.volume() - 0.25).abs() < f32::EPSILON);

    // Moved, not copied: reading it rewrites the file, and the strays are
    // gone. Asked of the parsed document rather than of the text, so this
    // says what it means -- nothing but sections at the top level.
    let text = std::fs::read_to_string(&path).unwrap();
    let document: toml::Table = toml::from_str(&text).unwrap();
    assert!(document.contains_key("library"), "not sectioned:\n{text}");
    let strays: Vec<&String> = document
        .iter()
        .filter(|(_, value)| !value.is_table())
        .map(|(key, _)| key)
        .collect();
    assert!(
        strays.is_empty(),
        "stray top-level keys {strays:?}:\n{text}"
    );

    // And the frontend's section came through untouched.
    #[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    struct Ui {
        theme: String,
    }
    assert_eq!(
        scoria_core::config::load_section::<Ui>(&path, "ui")
            .unwrap()
            .theme,
        "Scoria Dark"
    );

    // Idempotent: a second read finds the section and changes nothing.
    let again = scoria_core::config::load_library(&path).unwrap();
    assert!((again.volume() - 0.25).abs() < f32::EPSILON);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn a_session_saving_its_settings_leaves_the_frontend_s_alone() {
    // The seam the bug actually lived at. The helper-level test above would
    // have passed throughout: Session was the thing calling the wrong
    // function, so that is where this has to be asserted.
    let scratch = Scratch::new("session-settings");
    let dir = scratch.join("home");
    let _ = std::fs::remove_dir_all(&dir);
    let paths = Paths::rooted_at(&dir);

    #[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    struct Ui {
        theme: String,
    }
    let ui = Ui {
        theme: "Scoria Dark".to_owned(),
    };
    scoria_core::config::save_section(&paths.config_file(), "ui", &ui).unwrap();

    let mut session = scoria_core::session::Session::open(paths.clone()).unwrap();
    session.config_mut().set_volume(0.3);
    session.save_config().unwrap();

    assert_eq!(
        scoria_core::config::load_section::<Ui>(&paths.config_file(), "ui").unwrap(),
        ui,
        "a session saving its settings deleted the frontend's"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_section_yields_defaults() {
    let scratch = Scratch::new("no-section");
    let path = scratch.join("no-section.toml");
    let _ = std::fs::remove_file(&path);
    scoria_core::config::save_library(&path, &LibraryConfig::default()).unwrap();

    #[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    struct Ui {
        theme: String,
    }
    assert_eq!(
        scoria_core::config::load_section::<Ui>(&path, "ui").unwrap(),
        Ui::default()
    );
}

#[test]
fn a_section_can_be_rewritten_without_growing_the_file() {
    let scratch = Scratch::new("rewrite");
    let path = scratch.join("rewrite.toml");
    let _ = std::fs::remove_file(&path);

    #[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    #[serde(default)]
    struct Ui {
        width: u32,
    }
    scoria_core::config::save_section(&path, "ui", &Ui { width: 100 }).unwrap();
    scoria_core::config::save_section(&path, "ui", &Ui { width: 200 }).unwrap();

    assert_eq!(
        scoria_core::config::load_section::<Ui>(&path, "ui").unwrap(),
        Ui { width: 200 }
    );
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        text.matches("[ui]").count(),
        1,
        "the section was duplicated:\n{text}"
    );
}
