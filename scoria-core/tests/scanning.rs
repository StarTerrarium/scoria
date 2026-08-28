//! Scan-then-query integration tests.
//!
//! These build a real library tree on disk from a 55-byte silent FLAC, retagged
//! per file with lofty, then scan it. That means the tag path under test is the
//! real one -- lofty writing Vorbis comments and lofty reading them back -- and
//! the fixture stays small enough to belong in the repository.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use lofty::config::WriteOptions;
use lofty::prelude::{ItemKey, TagExt};
use lofty::tag::{Tag, TagType};
use scoria_core::db::{AlbumRepository, SqliteDatabase, TrackRepository};
use scoria_core::library::artwork::{self, Artwork, ArtworkCache, ArtworkKey};
use scoria_core::library::{ScanProgress, Scanner, tags};
use scoria_core::query::sort::{SortChain, SortPreset};
use std::path::{Path, PathBuf};

const SILENCE: &[u8] = include_bytes!("fixtures/silence.flac");

/// A unique scratch directory per test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("scoria-scan-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes a real FLAC with the given Vorbis comments.
fn write_track(root: &Path, relative: &str, fields: &[(ItemKey, &str)]) -> PathBuf {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, SILENCE).unwrap();

    let mut tag = Tag::new(TagType::VorbisComments);
    for (key, value) in fields {
        tag.insert_text(*key, (*value).to_owned());
    }
    tag.save_to_path(&path, WriteOptions::default()).unwrap();
    path
}

fn db() -> SqliteDatabase {
    SqliteDatabase::open_in_memory().unwrap()
}

fn scan(root: &Path, db: &mut SqliteDatabase) -> scoria_core::ScanOutcome {
    Scanner::new(root).scan(db, &mut |_| {}).unwrap()
}

// --------------------------------------------------------------- round trip --

#[test]
fn a_scanned_file_can_be_read_back_with_its_tags() {
    let root = scratch("roundtrip");
    write_track(
        &root,
        "花冷え。/乙女改革/01 SUNRISE.flac",
        &[
            (ItemKey::TrackTitle, "SUNRISE 味噌SOUP"),
            (ItemKey::TrackArtist, "花冷え。"),
            (ItemKey::AlbumArtist, "花冷え。"),
            (ItemKey::AlbumArtistSortOrder, "HANABIE."),
            (ItemKey::AlbumTitle, "乙女改革"),
            (ItemKey::RecordingDate, "2021-01-14"),
            (ItemKey::TrackNumber, "1"),
            (ItemKey::DiscNumber, "1"),
        ],
    );

    let mut db = db();
    let outcome = scan(&root, &mut db);
    assert_eq!(outcome.added.len(), 1);
    assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);

    let track = &db.all_tracks().unwrap()[0];
    assert_eq!(track.title, "SUNRISE 味噌SOUP");
    assert_eq!(track.album_artist, "花冷え。");
    assert_eq!(track.album_artist_sort.as_deref(), Some("HANABIE."));
    assert_eq!(track.album, "乙女改革");
    assert_eq!(track.date, scoria_core::Date::parse("2021-01-14"));
    assert_eq!(track.track_number, Some(1));
    assert!(track.available);
    // The fixture is 2205 samples at 44.1 kHz: 50 ms.
    assert!(
        track.duration.as_millis() > 0,
        "duration should come from the file"
    );
}

#[test]
fn paths_are_stored_relative_to_the_root() {
    // §4.3, end to end: the scanner sees absolute paths and must store none.
    let root = scratch("relative");
    write_track(
        &root,
        "Ghost/Meliora/01.flac",
        &[(ItemKey::TrackTitle, "Spirit")],
    );

    let mut db = db();
    scan(&root, &mut db);

    let (_, path) = db.all_track_paths().unwrap().into_iter().next().unwrap();
    assert_eq!(path.to_string(), "Ghost/Meliora/01.flac");
    assert!(!path.as_path().is_absolute());
}

#[test]
fn scanning_is_idempotent() {
    let root = scratch("idempotent");
    write_track(&root, "a/b/01.flac", &[(ItemKey::TrackTitle, "One")]);
    write_track(&root, "a/b/02.flac", &[(ItemKey::TrackTitle, "Two")]);

    let mut db = db();
    let first = scan(&root, &mut db);
    assert_eq!(first.added.len(), 2);

    let second = scan(&root, &mut db);
    assert!(second.added.is_empty(), "a rescan should add nothing");
    assert_eq!(second.updated.len(), 2);
    assert_eq!(
        db.track_count().unwrap(),
        2,
        "rescanning must not duplicate"
    );
}

#[test]
fn non_audio_files_are_ignored() {
    let root = scratch("nonaudio");
    write_track(&root, "a/b/01.flac", &[(ItemKey::TrackTitle, "One")]);
    std::fs::write(root.join("a/b/cover.jpg"), b"not audio").unwrap();
    std::fs::write(root.join("a/b/notes.txt"), b"hello").unwrap();

    let mut db = db();
    scan(&root, &mut db);
    assert_eq!(db.track_count().unwrap(), 1);
}

#[test]
fn an_unreadable_file_is_reported_without_failing_the_scan() {
    // One corrupt file must not cost the library.
    let root = scratch("corrupt");
    write_track(
        &root,
        "good/album/01.flac",
        &[(ItemKey::TrackTitle, "Fine")],
    );
    std::fs::create_dir_all(root.join("bad/album")).unwrap();
    std::fs::write(root.join("bad/album/02.flac"), b"this is not a FLAC file").unwrap();

    let mut db = db();
    let outcome = scan(&root, &mut db);

    assert_eq!(
        outcome.added.len(),
        1,
        "the good file should still be indexed"
    );
    assert_eq!(outcome.failed.len(), 1, "the bad file should be reported");
    assert!(outcome.failed[0].0.ends_with("02.flac"));
}

// ------------------------------------------------------------- §4.2 identity --

#[test]
fn tracks_sharing_a_musicbrainz_id_land_in_one_album() {
    let root = scratch("mbid");
    let mbid = "1adca70c-fdd0-4ee3-9020-3556c04fba58";
    for (n, title) in [("01", "One"), ("02", "Two")] {
        write_track(
            &root,
            &format!("artist/album/{n}.flac"),
            &[
                (ItemKey::TrackTitle, title),
                (ItemKey::AlbumArtist, "Artist"),
                (ItemKey::AlbumTitle, "Album"),
                (ItemKey::MusicBrainzReleaseId, mbid),
            ],
        );
    }

    let mut db = db();
    scan(&root, &mut db);
    assert_eq!(db.album_count().unwrap(), 1);
}

#[test]
fn a_reissue_with_a_different_mbid_is_a_different_album() {
    let root = scratch("reissue");
    for (dir, mbid) in [("original", "aaaa"), ("reissue", "bbbb")] {
        write_track(
            &root,
            &format!("artist/{dir}/01.flac"),
            &[
                (ItemKey::AlbumArtist, "Artist"),
                (ItemKey::AlbumTitle, "Album"),
                (ItemKey::MusicBrainzReleaseId, mbid),
            ],
        );
    }

    let mut db = db();
    scan(&root, &mut db);
    assert_eq!(db.album_count().unwrap(), 2);
}

#[test]
fn untagged_files_fall_back_to_their_directory() {
    let root = scratch("untagged");
    write_track(&root, "rips/disc one/01.flac", &[]);
    write_track(&root, "rips/disc one/02.flac", &[]);
    write_track(&root, "rips/disc two/01.flac", &[]);

    let mut db = db();
    let outcome = scan(&root, &mut db);
    assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
    assert_eq!(db.album_count().unwrap(), 2, "one album per directory");
}

#[test]
fn a_file_with_no_title_is_labelled_by_its_filename_not_left_blank() {
    let root = scratch("notitle");
    write_track(&root, "rips/album/09 Mystery Track.flac", &[]);

    let mut db = db();
    scan(&root, &mut db);
    assert_eq!(db.all_tracks().unwrap()[0].title, "09 Mystery Track");
}

// ------------------------------------------------------------ §6.7 gain --

#[test]
fn replay_gain_is_read_from_tags() {
    let root = scratch("gain");
    write_track(
        &root,
        "a/b/01.flac",
        &[
            (ItemKey::ReplayGainTrackGain, "-9.95 dB"),
            (ItemKey::ReplayGainTrackPeak, "1.000000"),
            (ItemKey::ReplayGainAlbumGain, "-9.69 dB"),
            (ItemKey::ReplayGainAlbumPeak, "0.988"),
        ],
    );

    let mut db = db();
    scan(&root, &mut db);
    let gain = db.all_tracks().unwrap()[0].replay_gain;

    assert_eq!(gain.track_gain_db, Some(-9.95));
    assert_eq!(gain.album_gain_db, Some(-9.69));
    assert_eq!(gain.track_peak, Some(1.0));
    assert!(gain.is_tagged());
}

#[test]
fn a_file_without_gain_tags_is_untagged_not_zero() {
    // §6.7 needs to tell "no data" apart from "measured at 0 dB": the first
    // plays at unity, the second is a real measurement.
    let root = scratch("nogain");
    write_track(&root, "a/b/01.flac", &[(ItemKey::TrackTitle, "One")]);

    let mut db = db();
    scan(&root, &mut db);
    assert!(!db.all_tracks().unwrap()[0].replay_gain.is_tagged());
}

// ---------------------------------------------------------- §7.4 availability --

#[test]
fn a_vanished_file_is_marked_unavailable_not_deleted() {
    let root = scratch("vanished");
    let path = write_track(&root, "a/b/01.flac", &[(ItemKey::TrackTitle, "One")]);
    write_track(&root, "a/b/02.flac", &[(ItemKey::TrackTitle, "Two")]);

    let mut db = db();
    scan(&root, &mut db);

    std::fs::remove_file(&path).unwrap();
    let outcome = scan(&root, &mut db);

    assert_eq!(outcome.now_unavailable.len(), 1);
    assert_eq!(db.track_count().unwrap(), 2, "the row must still exist");
    let gone = db.track(outcome.now_unavailable[0]).unwrap().unwrap();
    assert!(!gone.available);
    assert_eq!(gone.title, "One");
}

#[test]
fn a_restored_file_becomes_available_again() {
    let root = scratch("restored");
    let path = write_track(&root, "a/b/01.flac", &[(ItemKey::TrackTitle, "One")]);

    let mut db = db();
    scan(&root, &mut db);

    let saved = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    scan(&root, &mut db);
    assert!(!db.all_tracks().unwrap()[0].available);

    std::fs::write(&path, saved).unwrap();
    scan(&root, &mut db);
    assert!(db.all_tracks().unwrap()[0].available);
}

// ------------------------------------------------------------- §5.3 artwork --

#[test]
fn a_sidecar_cover_is_found_and_cached() {
    let root = scratch("artwork-sidecar");
    write_track(
        &root,
        "artist/album/01.flac",
        &[(ItemKey::AlbumTitle, "Album")],
    );
    std::fs::write(root.join("artist/album/cover.jpg"), b"pretend jpeg").unwrap();

    let cache = ArtworkCache::new(scratch("artwork-cache").join("art"));
    let mut db = db();
    Scanner::new(&root)
        .with_artwork(&cache)
        .scan(&mut db, &mut |_| {})
        .unwrap();

    let album = &db.all_albums().unwrap()[0];
    let cached = cache
        .get(&ArtworkKey::from_stored(&album.identity))
        .expect("artwork should be cached");
    assert_eq!(std::fs::read(cached).unwrap(), b"pretend jpeg");
}

#[test]
fn sidecar_lookup_is_case_insensitive() {
    let root = scratch("artwork-case");
    write_track(&root, "artist/album/01.flac", &[]);
    std::fs::write(root.join("artist/album/Folder.JPG"), b"art").unwrap();

    let found = artwork::sidecar(&root.join("artist/album"));
    assert!(matches!(found, Some(Artwork::Sidecar(_))), "got {found:?}");
}

#[test]
fn cover_is_preferred_over_other_sidecar_names() {
    let root = scratch("artwork-priority");
    std::fs::create_dir_all(root.join("album")).unwrap();
    std::fs::write(root.join("album/artwork.png"), b"wrong").unwrap();
    std::fs::write(root.join("album/cover.jpg"), b"right").unwrap();

    let Some(Artwork::Sidecar(path)) = artwork::sidecar(&root.join("album")) else {
        panic!("expected a sidecar");
    };
    assert_eq!(std::fs::read(path).unwrap(), b"right");
}

#[test]
fn an_album_without_artwork_simply_has_none() {
    let root = scratch("artwork-none");
    write_track(&root, "artist/album/01.flac", &[]);

    let cache = ArtworkCache::new(scratch("artwork-none-cache").join("art"));
    let mut db = db();
    Scanner::new(&root)
        .with_artwork(&cache)
        .scan(&mut db, &mut |_| {})
        .unwrap();

    let album = &db.all_albums().unwrap()[0];
    assert!(!cache.has(&ArtworkKey::from_stored(&album.identity)));
}

#[test]
fn replacing_artwork_does_not_leave_the_old_file_behind() {
    let cache = ArtworkCache::new(scratch("artwork-replace").join("art"));
    let key = ArtworkKey::from_stored("some-album-identity");

    cache
        .store(
            &key,
            &Artwork::Embedded {
                data: b"png".to_vec(),
                extension: "png",
            },
        )
        .unwrap();
    cache
        .store(
            &key,
            &Artwork::Embedded {
                data: b"jpg".to_vec(),
                extension: "jpg",
            },
        )
        .unwrap();

    assert!(
        !cache.directory().join(format!("{key}.png")).exists(),
        "stale artwork left behind"
    );
    assert_eq!(std::fs::read(cache.get(&key).unwrap()).unwrap(), b"jpg");
}

#[test]
fn artwork_survives_the_database_being_rebuilt() {
    // The hazard this keying exists for. An album id is a rowid, and a rebuild
    // reassigns them -- art filed under one would resurface attached to
    // whichever album inherited the number. Identity does not move.
    let root = scratch("artwork-rebuild");
    write_track(
        &root,
        "artist/album/01.flac",
        &[
            (ItemKey::AlbumArtist, "Artist"),
            (ItemKey::AlbumTitle, "Album"),
        ],
    );
    std::fs::write(root.join("artist/album/cover.jpg"), b"the right cover").unwrap();
    let cache = ArtworkCache::new(scratch("artwork-rebuild-cache").join("art"));

    let mut first = db();
    Scanner::new(&root)
        .with_artwork(&cache)
        .scan(&mut first, &mut |_| {})
        .unwrap();
    let before = first.all_albums().unwrap()[0].clone();

    // A second, entirely separate database: every rowid is assigned afresh.
    let mut second = db();
    Scanner::new(&root)
        .with_artwork(&cache)
        .scan(&mut second, &mut |_| {})
        .unwrap();
    let after = second.all_albums().unwrap()[0].clone();

    assert_eq!(before.identity, after.identity, "identity should not move");
    let art = cache
        .get(&ArtworkKey::from_stored(&after.identity))
        .expect("artwork should still be found");
    assert_eq!(std::fs::read(art).unwrap(), b"the right cover");
}

#[test]
fn artwork_for_an_album_that_is_gone_is_pruned() {
    // Derived data that only ever grows is a leak, and stale entries are also
    // what would let an old naming scheme linger.
    let root = scratch("artwork-prune");
    write_track(
        &root,
        "artist/album/01.flac",
        &[(ItemKey::AlbumTitle, "Album")],
    );
    std::fs::write(root.join("artist/album/cover.jpg"), b"art").unwrap();
    let cache = ArtworkCache::new(scratch("artwork-prune-cache").join("art"));

    let mut db = db();
    Scanner::new(&root)
        .with_artwork(&cache)
        .scan(&mut db, &mut |_| {})
        .unwrap();

    // Something from a scheme this cache no longer uses.
    std::fs::write(cache.directory().join("7.jpg"), b"stale").unwrap();

    let outcome = Scanner::new(&root)
        .with_artwork(&cache)
        .scan(&mut db, &mut |_| {})
        .unwrap();
    assert_eq!(outcome.artwork_pruned, 1);
    assert!(!cache.directory().join("7.jpg").exists());
}

// --------------------------------------------------------- scan then query --

#[test]
fn a_scanned_library_sorts_the_way_the_design_describes() {
    // The milestone's real question: does what came off disk order correctly?
    let root = scratch("sortquery");
    for (path, artist, sort, album, date, n) in [
        (
            "o/1.flac",
            "The Omnific",
            "Omnific, The",
            "Escapades",
            "2021",
            "1",
        ),
        ("h/1.flac", "花冷え。", "HANABIE.", "乙女改革", "2021", "1"),
        ("g/1.flac", "Ghost", "Ghost", "Meliora", "2015", "2"),
        ("g/2.flac", "Ghost", "Ghost", "Meliora", "2015", "1"),
        ("g/3.flac", "Ghost", "Ghost", "Opus Eponymous", "2010", "1"),
    ] {
        write_track(
            &root,
            path,
            &[
                (ItemKey::TrackArtist, artist),
                (ItemKey::AlbumArtist, artist),
                (ItemKey::AlbumArtistSortOrder, sort),
                (ItemKey::AlbumTitle, album),
                (ItemKey::RecordingDate, date),
                (ItemKey::TrackNumber, n),
            ],
        );
    }

    let mut db = db();
    scan(&root, &mut db);

    let mut tracks = db.all_tracks().unwrap();
    SortPreset::ArtistChronological.chain().sort(&mut tracks);

    let order: Vec<String> = tracks
        .iter()
        .map(|t| {
            format!(
                "{}/{}/{}",
                t.album_artist_key(),
                t.album,
                t.track_number.unwrap_or(0)
            )
        })
        .collect();

    assert_eq!(
        order,
        vec![
            "Ghost/Opus Eponymous/1", // Ghost's albums oldest first
            "Ghost/Meliora/1",
            "Ghost/Meliora/2",
            "HANABIE./乙女改革/1",      // sorts under H, not by codepoint
            "Omnific, The/Escapades/1", // sorts under O, not under T
        ]
    );
}

#[test]
fn a_scanned_library_is_searchable() {
    let root = scratch("searchquery");
    write_track(
        &root,
        "ec/tekkno/01.flac",
        &[
            (ItemKey::TrackTitle, "Hurrikan"),
            (ItemKey::AlbumArtist, "Electric Callboy"),
        ],
    );
    write_track(
        &root,
        "hb/otome/01.flac",
        &[
            (ItemKey::TrackTitle, "SUNRISE 味噌SOUP"),
            (ItemKey::AlbumArtist, "花冷え。"),
        ],
    );

    let mut db = db();
    scan(&root, &mut db);

    assert_eq!(db.search_tracks("callboy").unwrap().len(), 1);
    assert_eq!(db.search_tracks("lect").unwrap().len(), 1);
    assert_eq!(db.search_tracks("味噌").unwrap().len(), 1);
}

#[test]
fn progress_is_reported_and_ends_at_the_total() {
    let root = scratch("progress");
    for n in 1..=5 {
        write_track(&root, &format!("a/b/{n}.flac"), &[]);
    }

    let mut db = db();
    let mut reports: Vec<ScanProgress> = Vec::new();
    Scanner::new(&root)
        .scan(&mut db, &mut |p| reports.push(p))
        .unwrap();

    assert!(reports.len() >= 2);
    let last = reports.last().unwrap();
    assert_eq!(last.total, Some(5));
    assert_eq!(last.seen, 5);
}

#[test]
fn deltas_name_the_tracks_that_changed() {
    // The §14 question this milestone closes: the scanner knows exactly which
    // tracks changed, so it says so rather than invalidating everything.
    let root = scratch("deltas");
    write_track(&root, "a/b/01.flac", &[]);

    let mut db = db();
    let outcome = scan(&root, &mut db);
    let deltas = outcome.deltas();

    assert_eq!(deltas.len(), 1);
    assert!(matches!(
        &deltas[0],
        scoria_core::LibraryDelta::TracksAdded(ids) if ids.len() == 1
    ));
}

#[test]
fn an_empty_root_scans_cleanly() {
    let root = scratch("empty");
    let mut db = db();
    let outcome = scan(&root, &mut db);
    assert!(outcome.is_empty());
    assert!(outcome.failed.is_empty());
}

#[test]
fn a_missing_root_does_not_fail_the_scan() {
    let root = scratch("missing").join("does-not-exist");
    let mut db = db();
    let outcome = scan(&root, &mut db);
    assert!(outcome.is_empty());
}

#[test]
fn the_default_sort_chain_is_the_one_a_scan_feeds() {
    assert_eq!(
        SortChain::default(),
        SortPreset::ArtistChronological.chain()
    );
}

#[test]
fn tag_reading_tolerates_a_track_number_with_a_total() {
    let root = scratch("tracktotal");
    write_track(&root, "a/b/01.flac", &[(ItemKey::TrackNumber, "4/12")]);

    let tags = tags::read(&root.join("a/b/01.flac")).unwrap();
    assert_eq!(tags.track_number, Some(4));
}
