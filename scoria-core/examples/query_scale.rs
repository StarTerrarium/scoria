//! Temporary: does load-filter-sort-in-Rust hold at six figures (§14)?
use scoria_core::date::Date;
use scoria_core::db::{AlbumRepository, NewAlbum, NewTrack, SqliteDatabase, TrackRepository};
use scoria_core::library::AlbumIdentity;
use scoria_core::model::{ReplayGain, Timestamp};
use scoria_core::path::RelativePath;
use scoria_core::query::{LibraryIndex, QueryEngine, Selection};
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(100_000);
    let mut db = SqliteDatabase::open_in_memory()?;

    let build = Instant::now();
    let mut batch = Vec::with_capacity(1000);
    let artists = 2_000;
    for i in 0..n {
        let artist = i % artists;
        let album = i / 12;
        if i % 12 == 0 {
            db.upsert_album(&NewAlbum {
                identity: AlbumIdentity::resolve(
                    None,
                    Some(&format!("Artist {artist}")),
                    Some(&format!("Album {album}")),
                    None,
                    None,
                ),
                title: format!("Album {album}"),
                album_artist: format!("Artist {artist}"),
                album_artist_sort: Some(format!("Artist {artist:05}")),
                date: Date::parse(&format!("{}", 1970 + (album % 50))),
            })?;
        }
        batch.push(NewTrack {
            album_id: scoria_core::AlbumId((album + 1) as u64),
            path: RelativePath::new(format!("a/{:06}.flac", i))?,
            title: format!("Track {i} of something reasonably long"),
            artist: format!("Artist {artist}"),
            album_artist: format!("Artist {artist}"),
            album_artist_sort: Some(format!("Artist {artist:05}")),
            album: format!("Album {album}"),
            date: Date::parse(&format!("{}", 1970 + (album % 50))),
            disc: Some(1),
            track_number: Some((i % 12 + 1) as u16),
            duration: Duration::from_secs(180 + (i % 120) as u64),
            replay_gain: ReplayGain::default(),
            added_at: Timestamp(1_700_000_000 + i as i64),
        });
        if batch.len() == 1000 {
            db.upsert_tracks(&batch)?;
            batch.clear();
        }
    }
    db.upsert_tracks(&batch)?;
    println!("built {n} tracks in {:.2?}", build.elapsed());

    let load = Instant::now();
    let index = LibraryIndex::load(&db)?;
    println!(
        "index loaded in {:.2?} ({} tracks)",
        load.elapsed(),
        index.len()
    );

    let engine = QueryEngine::new();
    let bench = |label: &str, selection: &Selection| -> Result<(), Box<dyn std::error::Error>> {
        // Warm once, then take the best of three.
        let _ = engine.run(&index, &db, selection)?;
        let mut best = Duration::MAX;
        let mut rows = 0;
        for _ in 0..3 {
            let t = Instant::now();
            let outcome = engine.run(&index, &db, selection)?;
            best = best.min(t.elapsed());
            rows = outcome.tracks.len();
        }
        println!("  {label:38} {best:>10.2?}  ({rows} rows)");
        Ok(())
    };

    println!("\nquery timings (best of 3):");
    bench("unfiltered whole library", &Selection::all())?;
    bench(
        "one artist selected",
        &Selection {
            album_artists: vec!["Artist 7".to_owned()],
            ..Selection::all()
        },
    )?;
    bench(
        "search",
        &Selection {
            search: Some("Track 4242".to_owned()),
            ..Selection::all()
        },
    )?;
    bench(
        "search + artist",
        &Selection {
            album_artists: vec!["Artist 7".to_owned()],
            search: Some("Track".to_owned()),
            ..Selection::all()
        },
    )?;
    Ok(())
}
