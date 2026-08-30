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
    Migration {
        version: 3,
        sql: SCHEMA_V3,
    },
    Migration {
        version: 4,
        sql: SCHEMA_V4,
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

/// How many times a track has been played (§6.8).
///
/// `NOT NULL DEFAULT 0`, so a library that predates the column starts everyone
/// at nought rather than at "unknown" -- there is no honest number to
/// reconstruct, and an empty cell would be indistinguishable from a track that
/// has genuinely never been played.
const SCHEMA_V3: &str = "
ALTER TABLE tracks ADD COLUMN play_count INTEGER NOT NULL DEFAULT 0;
";

/// Narrows the full-text update trigger to the columns it indexes.
///
/// As written in v1 it fired on any `UPDATE tracks` at all, so counting a play
/// (§6.8) or marking a file missing (§7.4) tore the row out of the search index
/// and put an identical one back. Correct, and pure waste: neither statement
/// touches a column `tracks_fts` contains. A library where every track has been
/// played is a search index that has been rebuilt a track at a time for nothing.
///
/// v1 is not edited to fix this, because editing a shipped migration changes
/// what a new database gets without changing what an existing one has -- which
/// is how two schemas that are supposed to be the same quietly stop being. A
/// fresh database therefore builds the v1 trigger and replaces it here.
const SCHEMA_V4: &str = "
DROP TRIGGER tracks_fts_update;

CREATE TRIGGER tracks_fts_update
AFTER UPDATE OF title, artist, album_artist, album ON tracks BEGIN
    INSERT INTO tracks_fts (tracks_fts, rowid, title, artist, album_artist, album)
    VALUES ('delete', old.id, old.title, old.artist, old.album_artist, old.album);
    INSERT INTO tracks_fts (rowid, title, artist, album_artist, album)
    VALUES (new.id, new.title, new.artist, new.album_artist, new.album);
END;
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn migrated() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn
    }

    fn trigger(conn: &Connection, name: &str) -> String {
        conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
            [name],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn the_full_text_index_is_only_disturbed_by_the_columns_it_holds() {
        // As written in v1 the update trigger fired on any UPDATE at all, so
        // counting a play (§6.8) or marking a file missing (§7.4) tore the row
        // out of the search index and put an identical one back. Neither
        // statement touches a column tracks_fts contains.
        let sql = trigger(&migrated(), "tracks_fts_update");
        assert!(
            sql.contains("UPDATE OF"),
            "the trigger fires on every column:\n{sql}"
        );
        for column in ["title", "artist", "album_artist", "album"] {
            assert!(
                sql.contains(column),
                "{column} is in the index but not in the trigger:\n{sql}"
            );
        }
    }

    #[test]
    fn a_fresh_database_lands_on_the_latest_version() {
        let conn = migrated();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, latest_version());
    }

    #[test]
    fn migrating_twice_changes_nothing() {
        // Each migration runs only when its version is ahead of the file's, so
        // re-running must be a no-op rather than a second DROP of a trigger
        // that is no longer there.
        let mut conn = migrated();
        let before = trigger(&conn, "tracks_fts_update");
        migrate(&mut conn).unwrap();
        assert_eq!(trigger(&conn, "tracks_fts_update"), before);
    }

    #[test]
    fn every_migration_has_a_version_ahead_of_the_one_before_it() {
        // The list is append-only and applied in order; a version that went
        // backwards or repeated would silently skip a step.
        let versions: Vec<i64> = MIGRATIONS.iter().map(|m| m.version).collect();
        let mut sorted = versions.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(versions, sorted, "migrations are out of order or repeated");
    }
}
