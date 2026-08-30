//! Measures the scan path, which is where the library-sized work happens.
//!
//! A rescan is the common case -- the watcher wakes on any change under the
//! root, and the scanner has no mtime shortcut, so it reads every file's tags
//! every time. What it must not do is write every row back.
//!
//! Measured at 10,000 tracks, before and after the pre-1.0 review, three runs
//! each and within about 1% of each other:
//!
//! | | before | after |
//! |---|---|---|
//! | first scan, all new | 313 ms | 318 ms |
//! | rescan, nothing changed | 381 ms | 108 ms |
//! | 1000 `record_play` calls | 40.5 ms | 5.9 ms |
//!
//! The first scan is marginally slower and that is the trade: matching each
//! file against what is stored costs a little on the pass where nothing can
//! match, and saves three quarters of every pass afterwards. `record_play` is
//! the full-text trigger no longer firing on a column it does not index.

use lofty::config::WriteOptions;
use lofty::prelude::{ItemKey, TagExt};
use lofty::tag::{Tag, TagType};
use scoria_core::db::{SqliteDatabase, TrackRepository};
use scoria_core::library::Scanner;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SILENCE: &[u8] = include_bytes!("../tests/fixtures/silence.flac");
const DEFAULT_TRACKS: usize = 2000;

fn main() {
    let count: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(DEFAULT_TRACKS);

    let root = std::env::temp_dir().join(format!("scoria-scanbench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    build_library(&root, count);

    let mut db = match SqliteDatabase::open_in_memory() {
        Ok(db) => db,
        Err(e) => {
            eprintln!("cannot open database: {e}");
            return;
        }
    };

    let first = time(|| scan(&root, &mut db));
    let unchanged = time(|| scan(&root, &mut db));
    let unchanged_again = time(|| scan(&root, &mut db));

    // One file retagged, which is the ordinary way a library changes.
    write_track(&root, "a000/00000.flac", "Retitled");
    let after_retag = time(|| scan(&root, &mut db));

    // Counting plays, which fires the full-text update trigger.
    let ids: Vec<_> = db
        .all_tracks()
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.id)
        .take(1000)
        .collect();
    let plays = time(|| {
        for id in &ids {
            let _ = db.record_play(*id);
        }
    });

    println!("{count} tracks");
    println!("  first scan (all new)      {}", ms(first));
    println!("  rescan, nothing changed   {}", ms(unchanged));
    println!("  rescan again              {}", ms(unchanged_again));
    println!("  rescan, one file retagged {}", ms(after_retag));
    println!("  {} record_play calls    {}", ids.len(), ms(plays));

    let _ = std::fs::remove_dir_all(&root);
}

fn time<T>(f: impl FnOnce() -> T) -> Duration {
    let started = Instant::now();
    let _ = f();
    started.elapsed()
}

fn ms(d: Duration) -> String {
    format!("{:>9.1} ms", d.as_secs_f64() * 1000.0)
}

fn scan(root: &Path, db: &mut SqliteDatabase) {
    let _ = Scanner::new(root).scan(db, &mut |_| {});
}

fn build_library(root: &Path, count: usize) {
    for i in 0..count {
        // Twelve to an album, so the identity ladder and the album cache get
        // a realistic number of distinct keys.
        let relative = format!("a{:03}/{i:05}.flac", i / 12);
        write_track(root, &relative, &format!("Track {i}"));
    }
}

fn write_track(root: &Path, relative: &str, title: &str) -> PathBuf {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, SILENCE);

    let mut tag = Tag::new(TagType::VorbisComments);
    tag.insert_text(ItemKey::TrackTitle, title.to_owned());
    tag.insert_text(ItemKey::AlbumArtist, "Bench".to_owned());
    tag.insert_text(
        ItemKey::AlbumTitle,
        format!("Album {}", relative.split('/').next().unwrap_or("x")),
    );
    let _ = tag.save_to_path(&path, WriteOptions::default());
    path
}
