//! Measures the query engine at the scale `DESIGN.md` §14 designs for.
//!
//! The architecture of `LibraryIndex` rests on the numbers in its module doc:
//! going to storage for every query costs about half a second at six figures,
//! and it costs that whether the query returns the whole library or fifty rows.
//! A search box runs a query per keystroke, so that is not a slow path, it is
//! the normal one.
//!
//! This is what produced those numbers, and what should reproduce them after
//! the engine, the comparator or the index is touched. Run it with
//! `just bench-query`, optionally with a track count.
//!
//! Both columns are measured the same way they arise in practice. "Via
//! storage" is what the first version of the engine did: read the library out
//! of SQLite, then do the work -- so it pays for constructing every `Track`,
//! with its own string allocations, on every query. "Via the index" is the
//! work alone, against a library already in memory.

use scoria_core::date::Date;
use scoria_core::db::{AlbumRepository, NewAlbum, NewTrack, SqliteDatabase, TrackRepository};
use scoria_core::error::CoreError;
use scoria_core::library::AlbumIdentity;
use scoria_core::model::{ReplayGain, Timestamp};
use scoria_core::path::RelativePath;
use scoria_core::query::{LibraryIndex, QueryEngine, Selection};
use std::time::{Duration, Instant};

/// What §14 says to design for.
const DEFAULT_TRACKS: usize = 100_000;

/// Tracks per album, so the album pane has a realistic number of rows.
const PER_ALBUM: usize = 12;

/// Albums per artist, likewise for the artist pane.
const PER_ARTIST: usize = 8;

fn main() -> Result<(), CoreError> {
    let count = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse().ok())
        .unwrap_or(DEFAULT_TRACKS);

    println!("building a library of {count} tracks…");
    let built = Instant::now();
    let db = populate(count)?;
    println!("built in {}\n", millis(built.elapsed()));

    let index = LibraryIndex::load(&db)?;
    let engine = QueryEngine::new();

    // The artist in the middle of the library, so neither end of the sort is
    // being measured by accident.
    let artist = artist_name(count / PER_ALBUM / PER_ARTIST / 2);

    let cases: &[(&str, Selection)] = &[
        ("Unfiltered library", Selection::all()),
        (
            "One artist",
            Selection {
                album_artists: vec![artist],
                ..Selection::all()
            },
        ),
        (
            "Search",
            Selection {
                search: Some("Track 4242".to_owned()),
                ..Selection::all()
            },
        ),
    ];

    println!(
        "{:<22} {:>12} {:>12} {:>8}",
        "Query", "Via storage", "Via index", "Rows"
    );
    println!("{}", "-".repeat(58));

    for (name, selection) in cases {
        // Via storage: the library is read back before the work, which is what
        // the first version of the engine did on every single query.
        let started = Instant::now();
        let reloaded = LibraryIndex::load(&db)?;
        let outcome = engine.run(&reloaded, &db, selection)?;
        let via_storage = started.elapsed();

        // Via the index: the same work against a library already in memory.
        let started = Instant::now();
        let outcome = {
            let fresh = engine.run(&index, &db, selection)?;
            debug_assert_eq!(fresh.len(), outcome.len());
            fresh
        };
        let via_index = started.elapsed();

        println!(
            "{:<22} {:>12} {:>12} {:>8}",
            name,
            millis(via_storage),
            millis(via_index),
            outcome.len()
        );
    }

    println!(
        "\n{} tracks, {} albums, {} artists.",
        index.len(),
        db.album_count()?,
        count / PER_ALBUM / PER_ARTIST
    );
    Ok(())
}

/// Milliseconds, to the tenth. Whole milliseconds hide the difference between
/// 4 ms and 9 ms, which is the interesting end of this table.
fn millis(elapsed: Duration) -> String {
    format!("{:.1} ms", elapsed.as_secs_f64() * 1000.0)
}

fn artist_name(index: usize) -> String {
    format!("Artist {index:05}")
}

/// A synthetic library with a realistic shape: many artists, several albums
/// each, a dozen tracks per album.
fn populate(count: usize) -> Result<SqliteDatabase, CoreError> {
    let mut db = SqliteDatabase::open_in_memory()?;
    let mut batch: Vec<NewTrack> = Vec::new();
    let mut album_id = None;

    for track in 0..count {
        let album_index = track / PER_ALBUM;
        let artist_index = album_index / PER_ARTIST;

        if track % PER_ALBUM == 0 {
            let album_artist = artist_name(artist_index);
            // Year varies within an artist so the chronological default sort
            // has something to order by.
            let year = 1990_i16.saturating_add((album_index % 30) as i16);
            album_id = Some(db.upsert_album(&NewAlbum {
                identity: AlbumIdentity::Tagged {
                    album_artist: album_artist.clone(),
                    album: format!("Album {album_index:05}"),
                    date: Some(Date::from_year(year)),
                },
                title: format!("Album {album_index:05}"),
                album_artist,
                album_artist_sort: None,
                date: Some(Date::from_year(year)),
            })?);
        }
        let Some(album_id) = album_id else { continue };

        batch.push(NewTrack {
            album_id,
            path: RelativePath::new(format!(
                "{artist_index:05}/{album_index:05}/{track:06}.flac"
            ))?,
            title: format!("Track {track}"),
            artist: artist_name(artist_index),
            album_artist: artist_name(artist_index),
            album_artist_sort: None,
            album: format!("Album {album_index:05}"),
            date: Some(Date::from_year(
                1990_i16.saturating_add((album_index % 30) as i16),
            )),
            disc: Some(1),
            track_number: u16::try_from(track % PER_ALBUM).ok().map(|n| n + 1),
            duration: std::time::Duration::from_secs(200),
            replay_gain: ReplayGain::default(),
            added_at: Timestamp(1_700_000_000),
            genre: None,
            composer: None,
            audio: scoria_core::model::AudioProperties::default(),
        });

        // The same batch size the scanner writes in, so the population cost
        // resembles a real scan rather than one enormous transaction.
        if batch.len() >= 256 {
            db.upsert_tracks(&batch)?;
            batch.clear();
        }
    }
    if !batch.is_empty() {
        db.upsert_tracks(&batch)?;
    }
    Ok(db)
}
