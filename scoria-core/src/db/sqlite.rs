//! rusqlite implementations of the repository traits.
//!
//! The only module in the crate containing SQL, which
//! `scripts/check-boundaries.sh` enforces.

use crate::date::Date;
use crate::db::repository::{AlbumRepository, NewAlbum, NewTrack, TrackRepository};
use crate::db::schema;
use crate::error::CoreError;
use crate::id::{AlbumId, TrackId};
use crate::library::AlbumIdentity;
use crate::model::{Album, AudioProperties, ReplayGain, Timestamp, Track};
use crate::path::RelativePath;
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::path::Path;
use std::time::Duration;

/// Below this many characters the trigram index cannot match anything, so
/// search falls back to a scan. Three is the tokenizer's own limit, not ours.
const TRIGRAM_MINIMUM: usize = 3;

fn db_err(context: &str) -> impl FnOnce(rusqlite::Error) -> CoreError + '_ {
    move |e| CoreError::Database(format!("{context}: {e}"))
}

/// A SQLite-backed library index.
#[derive(Debug)]
pub struct SqliteDatabase {
    conn: Connection,
}

impl SqliteDatabase {
    /// Opens (creating if needed) and migrates.
    ///
    /// # Errors
    /// [`CoreError::Database`] if the file cannot be opened or migrated.
    pub fn open(path: &Path) -> Result<Self, CoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CoreError::Database(format!("cannot create {}: {e}", parent.display()))
            })?;
        }
        let conn = Connection::open(path).map_err(db_err("cannot open database"))?;
        Self::prepare(conn)
    }

    /// An in-memory database, for tests.
    ///
    /// # Errors
    /// [`CoreError::Database`] if the database cannot be created.
    pub fn open_in_memory() -> Result<Self, CoreError> {
        let conn = Connection::open_in_memory().map_err(db_err("cannot open database"))?;
        Self::prepare(conn)
    }

    fn prepare(mut conn: Connection) -> Result<Self, CoreError> {
        // ON DELETE CASCADE on tracks.album_id does nothing without this;
        // SQLite has foreign keys off by default for backwards compatibility.
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(db_err("cannot enable foreign keys"))?;
        // WAL lets the UI read while a scan writes, which is the whole point of
        // scanning in the background.
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(db_err("cannot enable WAL"))?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(db_err("cannot set synchronous mode"))?;

        schema::migrate(&mut conn)?;
        Ok(Self { conn })
    }

    /// The schema version currently applied.
    ///
    /// # Errors
    /// [`CoreError::Database`] on failure.
    pub fn schema_version(&self) -> Result<i64, CoreError> {
        self.conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(db_err("cannot read schema version"))
    }
}

// ---------------------------------------------------------------- row mapping

/// Reassembles a `Date` from its three columns.
fn read_date(row: &Row<'_>, year: &str, month: &str, day: &str) -> rusqlite::Result<Option<Date>> {
    let Some(year) = row.get::<_, Option<i64>>(year)? else {
        return Ok(None);
    };
    let month = row.get::<_, Option<i64>>(month)?;
    let day = row.get::<_, Option<i64>>(day)?;

    // Rebuilt through the parser so a hand-edited database cannot produce a
    // Date the constructor would have rejected.
    let text = match (month, day) {
        (Some(m), Some(d)) => format!("{year:04}-{m:02}-{d:02}"),
        (Some(m), None) => format!("{year:04}-{m:02}"),
        _ => format!("{year:04}"),
    };
    Ok(Date::parse(&text))
}

fn date_columns(date: Option<Date>) -> (Option<i64>, Option<i64>, Option<i64>) {
    match date {
        None => (None, None, None),
        Some(d) => (
            Some(i64::from(d.year())),
            d.month().map(i64::from),
            d.day().map(i64::from),
        ),
    }
}

/// Narrows a stored integer to the model's width, treating anything out of
/// range as absent rather than wrapping it into a plausible-looking wrong value.
fn read_u16(row: &Row<'_>, column: &str) -> rusqlite::Result<Option<u16>> {
    Ok(row
        .get::<_, Option<i64>>(column)?
        .and_then(|v| u16::try_from(v).ok()))
}

fn read_u32(row: &Row<'_>, column: &str) -> rusqlite::Result<Option<u32>> {
    Ok(row
        .get::<_, Option<i64>>(column)?
        .and_then(|v| u32::try_from(v).ok()))
}

fn read_u8(row: &Row<'_>, column: &str) -> rusqlite::Result<Option<u8>> {
    Ok(row
        .get::<_, Option<i64>>(column)?
        .and_then(|v| u8::try_from(v).ok()))
}

fn read_gain(row: &Row<'_>) -> rusqlite::Result<ReplayGain> {
    Ok(ReplayGain {
        track_gain_db: row.get("rg_track_gain")?,
        track_peak: row.get("rg_track_peak")?,
        album_gain_db: row.get("rg_album_gain")?,
        album_peak: row.get("rg_album_peak")?,
    })
}

/// Maps a row to a `Track`.
///
/// Returns `CorruptData` rather than panicking if the stored path is not
/// library-relative, which is the one field the model refuses to represent
/// loosely.
fn read_track(row: &Row<'_>) -> Result<Track, CoreError> {
    let map = |e: rusqlite::Error| CoreError::Database(format!("cannot read track row: {e}"));

    let raw_path: String = row.get("path").map_err(map)?;
    let path = RelativePath::new(raw_path.clone()).map_err(|_| {
        CoreError::CorruptData(format!("stored path is not library-relative: {raw_path}"))
    })?;

    let duration_ms: i64 = row.get("duration_ms").map_err(map)?;

    Ok(Track {
        id: TrackId(row.get::<_, i64>("id").map_err(map)?.cast_unsigned()),
        album_id: AlbumId(row.get::<_, i64>("album_id").map_err(map)?.cast_unsigned()),
        path,
        title: row.get("title").map_err(map)?,
        artist: row.get("artist").map_err(map)?,
        album_artist: row.get("album_artist").map_err(map)?,
        album_artist_sort: row.get("album_artist_sort").map_err(map)?,
        album: row.get("album").map_err(map)?,
        date: read_date(row, "date_year", "date_month", "date_day").map_err(map)?,
        disc: read_u16(row, "disc").map_err(map)?,
        track_number: read_u16(row, "track_number").map_err(map)?,
        duration: Duration::from_millis(duration_ms.unsigned_abs()),
        replay_gain: read_gain(row).map_err(map)?,
        added_at: Timestamp(row.get("added_at").map_err(map)?),
        available: row.get::<_, i64>("available").map_err(map)? != 0,
        genre: row.get("genre").map_err(map)?,
        composer: row.get("composer").map_err(map)?,
        audio: AudioProperties {
            bitrate_kbps: read_u32(row, "bitrate_kbps").map_err(map)?,
            sample_rate_hz: read_u32(row, "sample_rate_hz").map_err(map)?,
            bit_depth: read_u8(row, "bit_depth").map_err(map)?,
        },
    })
}

fn read_album(row: &Row<'_>) -> Result<Album, CoreError> {
    let map = |e: rusqlite::Error| CoreError::Database(format!("cannot read album row: {e}"));
    Ok(Album {
        id: AlbumId(row.get::<_, i64>("id").map_err(map)?.cast_unsigned()),
        identity: row.get("identity").map_err(map)?,
        title: row.get("title").map_err(map)?,
        album_artist: row.get("album_artist").map_err(map)?,
        album_artist_sort: row.get("album_artist_sort").map_err(map)?,
        date: read_date(row, "date_year", "date_month", "date_day").map_err(map)?,
    })
}

const TRACK_COLUMNS: &str = "id, album_id, path, title, artist, album_artist, album_artist_sort, \
     album, date_year, date_month, date_day, disc, track_number, duration_ms, \
     rg_track_gain, rg_track_peak, rg_album_gain, rg_album_peak, added_at, available, \
     genre, composer, bitrate_kbps, sample_rate_hz, bit_depth";

const ALBUM_COLUMNS: &str =
    "id, identity, title, album_artist, album_artist_sort, date_year, date_month, date_day";

// -------------------------------------------------------------- TrackRepository

impl TrackRepository for SqliteDatabase {
    fn upsert_track(&mut self, track: &NewTrack) -> Result<TrackId, CoreError> {
        let ids = self.upsert_tracks(std::slice::from_ref(track))?;
        ids.into_iter()
            .next()
            .ok_or_else(|| CoreError::Database("upsert returned no id".to_owned()))
    }

    fn upsert_tracks(&mut self, tracks: &[NewTrack]) -> Result<Vec<TrackId>, CoreError> {
        let transaction = self
            .conn
            .transaction()
            .map_err(db_err("cannot begin transaction"))?;
        let mut ids = Vec::with_capacity(tracks.len());
        {
            let mut statement = transaction
                .prepare(
                    "INSERT INTO tracks (
                         album_id, path, title, artist, album_artist, album_artist_sort, album,
                         date_year, date_month, date_day, disc, track_number, duration_ms,
                         rg_track_gain, rg_track_peak, rg_album_gain, rg_album_peak,
                         added_at, genre, composer, bitrate_kbps, sample_rate_hz, bit_depth,
                         available
                     ) VALUES (
                         ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                         ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, 1
                     )
                     ON CONFLICT(path) DO UPDATE SET
                         album_id = excluded.album_id,
                         title = excluded.title,
                         artist = excluded.artist,
                         album_artist = excluded.album_artist,
                         album_artist_sort = excluded.album_artist_sort,
                         album = excluded.album,
                         date_year = excluded.date_year,
                         date_month = excluded.date_month,
                         date_day = excluded.date_day,
                         disc = excluded.disc,
                         track_number = excluded.track_number,
                         duration_ms = excluded.duration_ms,
                         rg_track_gain = excluded.rg_track_gain,
                         rg_track_peak = excluded.rg_track_peak,
                         rg_album_gain = excluded.rg_album_gain,
                         rg_album_peak = excluded.rg_album_peak,
                         genre = excluded.genre,
                         composer = excluded.composer,
                         bitrate_kbps = excluded.bitrate_kbps,
                         sample_rate_hz = excluded.sample_rate_hz,
                         bit_depth = excluded.bit_depth,
                         available = 1
                     RETURNING id",
                )
                .map_err(db_err("cannot prepare track upsert"))?;

            for track in tracks {
                let (year, month, day) = date_columns(track.date);
                let duration_ms = i64::try_from(track.duration.as_millis()).unwrap_or(i64::MAX);
                let id: i64 = statement
                    .query_row(
                        params![
                            track.album_id.get().cast_signed(),
                            track.path.to_string(),
                            track.title,
                            track.artist,
                            track.album_artist,
                            track.album_artist_sort,
                            track.album,
                            year,
                            month,
                            day,
                            track.disc.map(i64::from),
                            track.track_number.map(i64::from),
                            duration_ms,
                            track.replay_gain.track_gain_db,
                            track.replay_gain.track_peak,
                            track.replay_gain.album_gain_db,
                            track.replay_gain.album_peak,
                            track.added_at.seconds(),
                            track.genre,
                            track.composer,
                            track.audio.bitrate_kbps.map(i64::from),
                            track.audio.sample_rate_hz.map(i64::from),
                            track.audio.bit_depth.map(i64::from),
                        ],
                        |row| row.get(0),
                    )
                    .map_err(db_err("cannot upsert track"))?;
                ids.push(TrackId(id.cast_unsigned()));
            }
        }
        transaction
            .commit()
            .map_err(db_err("cannot commit tracks"))?;
        Ok(ids)
    }

    fn track(&self, id: TrackId) -> Result<Option<Track>, CoreError> {
        let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE id = ?1");
        self.conn
            .query_row(&sql, params![id.get().cast_signed()], |row| {
                Ok(read_track(row))
            })
            .optional()
            .map_err(db_err("cannot fetch track"))?
            .transpose()
    }

    fn tracks_by_ids(&self, ids: &[TrackId]) -> Result<Vec<Track>, CoreError> {
        // Fetched individually and reassembled in the caller's order. A single
        // `IN (...)` query would return storage order, which would silently
        // reorder a playback snapshot -- the one thing §6.2 exists to prevent.
        let mut tracks = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(track) = self.track(*id)? {
                tracks.push(track);
            }
        }
        Ok(tracks)
    }

    fn track_by_path(&self, path: &RelativePath) -> Result<Option<Track>, CoreError> {
        let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE path = ?1");
        self.conn
            .query_row(&sql, params![path.to_string()], |row| Ok(read_track(row)))
            .optional()
            .map_err(db_err("cannot fetch track by path"))?
            .transpose()
    }

    fn all_tracks(&self) -> Result<Vec<Track>, CoreError> {
        let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks");
        let mut statement = self
            .conn
            .prepare(&sql)
            .map_err(db_err("cannot prepare query"))?;
        let rows = statement
            .query_map([], |row| Ok(read_track(row)))
            .map_err(db_err("cannot list tracks"))?;

        let mut tracks = Vec::new();
        for row in rows {
            tracks.push(row.map_err(db_err("cannot read track"))??);
        }
        Ok(tracks)
    }

    fn track_count(&self) -> Result<usize, CoreError> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM tracks", [], |row| row.get(0))
            .map_err(db_err("cannot count tracks"))?;
        Ok(usize::try_from(n).unwrap_or(0))
    }

    fn set_available(&mut self, ids: &[TrackId], available: bool) -> Result<(), CoreError> {
        let transaction = self
            .conn
            .transaction()
            .map_err(db_err("cannot begin transaction"))?;
        {
            let mut statement = transaction
                .prepare("UPDATE tracks SET available = ?1 WHERE id = ?2")
                .map_err(db_err("cannot prepare availability update"))?;
            for id in ids {
                statement
                    .execute(params![i64::from(available), id.get().cast_signed()])
                    .map_err(db_err("cannot update availability"))?;
            }
        }
        transaction
            .commit()
            .map_err(db_err("cannot commit availability"))
    }

    fn all_track_paths(&self) -> Result<Vec<(TrackId, RelativePath)>, CoreError> {
        let mut statement = self
            .conn
            .prepare("SELECT id, path FROM tracks")
            .map_err(db_err("cannot prepare path query"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_err("cannot list paths"))?;

        let mut paths = Vec::new();
        for row in rows {
            let (id, raw) = row.map_err(db_err("cannot read path"))?;
            let path = RelativePath::new(raw.clone()).map_err(|_| {
                CoreError::CorruptData(format!("stored path is not library-relative: {raw}"))
            })?;
            paths.push((TrackId(id.cast_unsigned()), path));
        }
        Ok(paths)
    }

    fn delete_tracks(&mut self, ids: &[TrackId]) -> Result<(), CoreError> {
        let transaction = self
            .conn
            .transaction()
            .map_err(db_err("cannot begin transaction"))?;
        {
            let mut statement = transaction
                .prepare("DELETE FROM tracks WHERE id = ?1")
                .map_err(db_err("cannot prepare delete"))?;
            for id in ids {
                statement
                    .execute(params![id.get().cast_signed()])
                    .map_err(db_err("cannot delete track"))?;
            }
        }
        transaction
            .commit()
            .map_err(db_err("cannot commit deletion"))
    }

    fn search_tracks(&self, query: &str) -> Result<Vec<TrackId>, CoreError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }

        // The trigram tokenizer cannot match anything shorter than a trigram,
        // which matters here: two-character queries are ordinary in Japanese.
        // A scan is affordable at this size and only happens for very short
        // queries, where the index would otherwise return nothing at all.
        if query.chars().count() < TRIGRAM_MINIMUM {
            return self.search_by_scan(query);
        }

        let mut statement = self
            .conn
            .prepare("SELECT rowid FROM tracks_fts WHERE tracks_fts MATCH ?1 ORDER BY rank")
            .map_err(db_err("cannot prepare search"))?;

        // Quoted as a single FTS5 string literal so that punctuation in a title
        // is searched for rather than parsed as query syntax. A user typing
        // `AC/DC` or `-` means those characters.
        let literal = format!("\"{}\"", query.replace('"', "\"\""));
        let rows = statement
            .query_map(params![literal], |row| row.get::<_, i64>(0))
            .map_err(db_err("cannot search"))?;

        let mut ids = Vec::new();
        for row in rows {
            ids.push(TrackId(
                row.map_err(db_err("cannot read search result"))?
                    .cast_unsigned(),
            ));
        }
        Ok(ids)
    }
}

impl SqliteDatabase {
    /// Substring search for queries too short for the trigram index.
    fn search_by_scan(&self, query: &str) -> Result<Vec<TrackId>, CoreError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id FROM tracks
                 WHERE title LIKE ?1 ESCAPE '\\'
                    OR artist LIKE ?1 ESCAPE '\\'
                    OR album_artist LIKE ?1 ESCAPE '\\'
                    OR album LIKE ?1 ESCAPE '\\'
                 ORDER BY id",
            )
            .map_err(db_err("cannot prepare scan"))?;

        let escaped = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let rows = statement
            .query_map(params![format!("%{escaped}%")], |row| row.get::<_, i64>(0))
            .map_err(db_err("cannot scan"))?;

        let mut ids = Vec::new();
        for row in rows {
            ids.push(TrackId(
                row.map_err(db_err("cannot read scan result"))?
                    .cast_unsigned(),
            ));
        }
        Ok(ids)
    }
}

// -------------------------------------------------------------- AlbumRepository

impl AlbumRepository for SqliteDatabase {
    fn upsert_album(&mut self, album: &NewAlbum) -> Result<AlbumId, CoreError> {
        let (year, month, day) = date_columns(album.date);
        let id: i64 = self
            .conn
            .query_row(
                "INSERT INTO albums (
                     identity, title, album_artist, album_artist_sort,
                     date_year, date_month, date_day
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(identity) DO UPDATE SET
                     title = excluded.title,
                     album_artist = excluded.album_artist,
                     album_artist_sort = excluded.album_artist_sort,
                     date_year = excluded.date_year,
                     date_month = excluded.date_month,
                     date_day = excluded.date_day
                 RETURNING id",
                params![
                    album.identity.key(),
                    album.title,
                    album.album_artist,
                    album.album_artist_sort,
                    year,
                    month,
                    day,
                ],
                |row| row.get(0),
            )
            .map_err(db_err("cannot upsert album"))?;
        Ok(AlbumId(id.cast_unsigned()))
    }

    fn album(&self, id: AlbumId) -> Result<Option<Album>, CoreError> {
        let sql = format!("SELECT {ALBUM_COLUMNS} FROM albums WHERE id = ?1");
        self.conn
            .query_row(&sql, params![id.get().cast_signed()], |row| {
                Ok(read_album(row))
            })
            .optional()
            .map_err(db_err("cannot fetch album"))?
            .transpose()
    }

    fn album_by_identity(&self, identity: &AlbumIdentity) -> Result<Option<AlbumId>, CoreError> {
        self.conn
            .query_row(
                "SELECT id FROM albums WHERE identity = ?1",
                params![identity.key()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(db_err("cannot look up album identity"))
            .map(|id| id.map(|id| AlbumId(id.cast_unsigned())))
    }

    fn all_albums(&self) -> Result<Vec<Album>, CoreError> {
        let sql = format!("SELECT {ALBUM_COLUMNS} FROM albums");
        let mut statement = self
            .conn
            .prepare(&sql)
            .map_err(db_err("cannot prepare query"))?;
        let rows = statement
            .query_map([], |row| Ok(read_album(row)))
            .map_err(db_err("cannot list albums"))?;

        let mut albums = Vec::new();
        for row in rows {
            albums.push(row.map_err(db_err("cannot read album"))??);
        }
        Ok(albums)
    }

    fn album_count(&self) -> Result<usize, CoreError> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM albums", [], |row| row.get(0))
            .map_err(db_err("cannot count albums"))?;
        Ok(usize::try_from(n).unwrap_or(0))
    }

    fn remove_empty_albums(&mut self) -> Result<usize, CoreError> {
        let removed = self
            .conn
            .execute(
                "DELETE FROM albums WHERE id NOT IN (SELECT DISTINCT album_id FROM tracks)",
                [],
            )
            .map_err(db_err("cannot remove empty albums"))?;
        Ok(removed)
    }
}
