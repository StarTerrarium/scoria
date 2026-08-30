//! What one frame of the track table costs to prepare.
//!
//! The GUI polls at 25ms, so `view` runs about forty times a second whether or
//! not anything changed, and a virtualised table draws a few dozen rows out of
//! a library §14 says to assume is six figures. This measures the two ways of
//! getting those rows, and what looking for artwork costs.
//!
//! At 100,000 tracks, for one second of frames:
//!
//! | | per second |
//! |---|---|
//! | resolving the whole result | 81 ms |
//! | resolving what is drawn | 0.03 ms |
//! | 120 artwork lookups | 0.6 ms |
//!
//! So preparing rows was about 8% of a core, continuously, and it grew with
//! the library while the drawing did not. The artwork lookup was worth caching
//! but was never the expensive one -- it sits two orders of magnitude below
//! the row resolution, and any ranking that put it first was a guess.

use scoria_core::date::Date;
use scoria_core::library::AlbumIdentity;
use scoria_core::library::artwork::{ArtworkCache, ArtworkKey};
use scoria_core::model::{Album, AudioProperties, ReplayGain, Timestamp, Track};
use scoria_core::path::RelativePath;
use scoria_core::query::LibraryIndex;
use scoria_core::{AlbumId, TrackId};
use std::time::{Duration, Instant};

/// A viewport of 800px at 22px a row, plus the overscan either side.
const ON_SCREEN: usize = 48;
const FRAMES: usize = 40;

fn main() {
    let count: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(100_000);

    let index = build(count);
    let ids: Vec<TrackId> = index.tracks().iter().map(|t| t.id).collect();

    // What `view` used to do: resolve the whole result, every frame, so that
    // the rows on screen could be drawn from it.
    let whole = time(FRAMES, || {
        let rows = index.rows(&ids);
        std::hint::black_box(rows.len());
    });

    // What it does now: resolve the rows the table is about to build.
    let visible = time(FRAMES, || {
        for id in ids.iter().take(ON_SCREEN) {
            std::hint::black_box(index.track(*id));
        }
    });

    println!("{count} tracks, {ON_SCREEN} rows on screen, {FRAMES} frames (one second)");
    println!("  resolving the whole result   {}", ms(whole));
    println!("  resolving what is drawn      {}", ms(visible));

    // Artwork was looked up twice per frame in `view` and once per pass over
    // the event stream. A miss stats every candidate extension.
    let cache = ArtworkCache::new(std::env::temp_dir().join("scoria-frame-cost-nothing-here"));
    let key = ArtworkKey::for_album(&AlbumIdentity::Directory("absent".to_owned()));
    let lookups = FRAMES * 3;
    let artwork = time(lookups, || {
        std::hint::black_box(cache.get(&key));
    });
    println!("  {lookups} artwork lookups        {}", ms(artwork));
}

fn time(iterations: usize, mut f: impl FnMut()) -> Duration {
    let started = Instant::now();
    for _ in 0..iterations {
        f();
    }
    started.elapsed()
}

fn ms(d: Duration) -> String {
    format!("{:>8.2} ms", d.as_secs_f64() * 1000.0)
}

fn build(count: usize) -> LibraryIndex {
    let mut tracks = Vec::with_capacity(count);
    let mut albums = Vec::new();

    for i in 0..count {
        let album_index = i / 12;
        if i % 12 == 0 {
            albums.push(Album {
                id: AlbumId(album_index as u64 + 1),
                identity: format!("bench:{album_index}"),
                title: format!("Album {album_index:05}"),
                album_artist: format!("Artist {:05}", album_index / 8),
                album_artist_sort: None,
                date: Some(Date::from_year(2000)),
            });
        }
        let Ok(path) = RelativePath::new(format!("{album_index:05}/{i:06}.flac")) else {
            continue;
        };
        tracks.push(Track {
            id: TrackId(i as u64 + 1),
            album_id: AlbumId(album_index as u64 + 1),
            path,
            title: format!("Track {i}"),
            artist: format!("Artist {:05}", album_index / 8),
            album_artist: format!("Artist {:05}", album_index / 8),
            album_artist_sort: None,
            album: format!("Album {album_index:05}"),
            date: Some(Date::from_year(2000)),
            disc: Some(1),
            track_number: None,
            duration: Duration::from_secs(200),
            replay_gain: ReplayGain::default(),
            added_at: Timestamp(0),
            play_count: 0,
            genre: None,
            composer: None,
            audio: AudioProperties::default(),
            available: true,
        });
    }
    LibraryIndex::from_parts(tracks, albums)
}
