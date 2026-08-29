//! Schema and migrations.
//!
//! Migrations are an append-only list keyed by `PRAGMA user_version`. Editing
//! an existing entry changes what a fresh database gets without changing what
//! an existing one has, which is how schemas silently diverge; add a new entry
//! instead.

use crate::error::CoreError;
use rusqlite::Connection;

/// One forward step. There is deliberately no `down`: a music library is
/// rebuildable from the files it indexes, so a failed migration is recovered by
/// rescanning rather than by reversing.
struct Migration {
    version: i64,
    sql: &'static str,
}

/// Append only. Never edit an entry that has shipped.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: SCHEMA_V1,
    },
    Migration {
        version: 2,
        sql: SCHEMA_V2,
    },
];

/// Metadata that is shown but never structural.
///
/// Genre and composer are tags; the encoding figures are read from the file
/// itself, because a 24-bit master and a 16-bit rip of it carry the same tags
/// and are not the same file.
///
/// Nothing backfills these. The scanner reads every file's tags on every scan
/// -- it has no mtime shortcut -- so the next scan fills them in, and until
/// then the columns are empty rather than wrong.
const SCHEMA_V2: &str = "
ALTER TABLE tracks ADD COLUMN genre          TEXT;
ALTER TABLE tracks ADD COLUMN composer       TEXT;
ALTER TABLE tracks ADD COLUMN bitrate_kbps   INTEGER;
ALTER TABLE tracks ADD COLUMN sample_rate_hz INTEGER;
ALTER TABLE tracks ADD COLUMN bit_depth      INTEGER;
";

/// The initial schema.
///
/// Notes on the shape:
///
/// - There is no library id anywhere. §5.1 chose a single root deliberately,
///   and that decision removes a column, per-root settings and path
///   disambiguation.
/// - `tracks.path` is relative to the root (§4.3) and unique, which is what
///   makes rescanning an upsert rather than a diff.
/// - Album artist and title are denormalised onto `tracks`. Sorting, grouping
///   and the filter cascade all read them per track, and a join on every row of
///   a six-figure table to fetch a string that never varies within an album is
///   a cost with no benefit.
/// - The FTS index uses the `trigram` tokenizer. `unicode61` only matches whole
///   tokens, so `lect` would not find `Electric Callboy`; trigram matches
///   substrings in any script, which is what a search box is expected to do.
///   Its one limit is that queries shorter than three characters match nothing,
///   which the repository handles with a `LIKE` fallback.
const SCHEMA_V1: &str = r"
CREATE TABLE albums (
    id                INTEGER PRIMARY KEY,
    identity          TEXT    NOT NULL UNIQUE,
    title             TEXT    NOT NULL,
    album_artist      TEXT    NOT NULL,
    album_artist_sort TEXT,
    date_year         INTEGER,
    date_month        INTEGER,
    date_day          INTEGER
) STRICT;

CREATE TABLE tracks (
    id                INTEGER PRIMARY KEY,
    album_id          INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
    path              TEXT    NOT NULL UNIQUE,
    title             TEXT    NOT NULL,
    artist            TEXT    NOT NULL,
    album_artist      TEXT    NOT NULL,
    album_artist_sort TEXT,
    album             TEXT    NOT NULL,
    date_year         INTEGER,
    date_month        INTEGER,
    date_day          INTEGER,
    disc              INTEGER,
    track_number      INTEGER,
    duration_ms       INTEGER NOT NULL,
    rg_track_gain     REAL,
    rg_track_peak     REAL,
    rg_album_gain     REAL,
    rg_album_peak     REAL,
    added_at          INTEGER NOT NULL,
    available         INTEGER NOT NULL DEFAULT 1
) STRICT;

-- The default sort chain leads with album artist, then album, disc and track
-- (§7.3), so this covers the ordering the table opens on.
CREATE INDEX tracks_sort ON tracks (
    album_artist_sort, album_artist, date_year, date_month, date_day, disc, track_number, id
);
CREATE INDEX tracks_album ON tracks (album_id);
CREATE INDEX tracks_available ON tracks (available);
CREATE INDEX albums_artist ON albums (album_artist_sort, album_artist);

CREATE VIRTUAL TABLE tracks_fts USING fts5(
    title,
    artist,
    album_artist,
    album,
    content = 'tracks',
    content_rowid = 'id',
    tokenize = 'trigram'
);

-- External-content FTS holds no copy of the data, so it has to be told about
-- every change. Missing one of these is how a search index silently rots.
CREATE TRIGGER tracks_fts_insert AFTER INSERT ON tracks BEGIN
    INSERT INTO tracks_fts (rowid, title, artist, album_artist, album)
    VALUES (new.id, new.title, new.artist, new.album_artist, new.album);
END;

CREATE TRIGGER tracks_fts_delete AFTER DELETE ON tracks BEGIN
    INSERT INTO tracks_fts (tracks_fts, rowid, title, artist, album_artist, album)
    VALUES ('delete', old.id, old.title, old.artist, old.album_artist, old.album);
END;

CREATE TRIGGER tracks_fts_update AFTER UPDATE ON tracks BEGIN
    INSERT INTO tracks_fts (tracks_fts, rowid, title, artist, album_artist, album)
    VALUES ('delete', old.id, old.title, old.artist, old.album_artist, old.album);
    INSERT INTO tracks_fts (rowid, title, artist, album_artist, album)
    VALUES (new.id, new.title, new.artist, new.album_artist, new.album);
END;
";

/// The version a freshly migrated database ends at.
#[must_use]
pub fn latest_version() -> i64 {
    MIGRATIONS.iter().map(|m| m.version).max().unwrap_or(0)
}

/// Applies every migration the database has not yet seen.
///
/// Each runs in its own transaction, so a failure leaves the database at the
/// last version that fully applied rather than half-way through one.
///
/// # Errors
///
/// [`CoreError::Database`] if a migration fails, or if the database is newer
/// than this build understands.
pub fn migrate(conn: &mut Connection) -> Result<(), CoreError> {
    let current: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| CoreError::Database(format!("cannot read schema version: {e}")))?;

    if current > latest_version() {
        return Err(CoreError::Database(format!(
            "database is at schema version {current}, but this build only understands {}. \
             It was probably written by a newer Scoria.",
            latest_version()
        )));
    }

    for migration in MIGRATIONS.iter().filter(|m| m.version > current) {
        let transaction = conn
            .transaction()
            .map_err(|e| CoreError::Database(format!("cannot begin migration: {e}")))?;

        transaction.execute_batch(migration.sql).map_err(|e| {
            CoreError::Database(format!("migration {} failed: {e}", migration.version))
        })?;

        // PRAGMA does not accept a bound parameter, and the value is one of our
        // own integer constants rather than anything user-supplied.
        transaction
            .pragma_update(None, "user_version", migration.version)
            .map_err(|e| CoreError::Database(format!("cannot record schema version: {e}")))?;

        transaction.commit().map_err(|e| {
            CoreError::Database(format!(
                "cannot commit migration {}: {e}",
                migration.version
            ))
        })?;
    }

    Ok(())
}
