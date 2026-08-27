//! Running a [`Selection`] against the library.
//!
//! `DESIGN.md` §7.2 for the filter cascade, §7.3 for sorting and grouping.
//!
//! ## How the work is divided
//!
//! Everything except full-text search runs against [`LibraryIndex`], in memory.
//! Storage is consulted only for FTS, which is the one thing SQLite does that
//! a slice cannot.
//!
//! Two reasons, and neither is a preference for Rust over SQL.
//!
//! Sorting cannot go to SQL without duplicating the comparator, and SQLite
//! would reach a different answer: its `NOCASE` collation is ASCII-only, so it
//! orders `TERÄSBETONI` before `Teräsbetoni` where the Rust comparator folds
//! them. Two orders for one library would be a correctness bug, because §6.3
//! requires a "play the whole library" snapshot to respect the table's current
//! sort exactly.
//!
//! And the cascade needs counts against three filter contexts at once --
//! artists counted before the artist filter applies, albums counted after it,
//! tracks counted after both. Over a slice that is one pass. In SQL it is three
//! round trips that have to agree with each other.
//!
//! [`Selection`]: crate::query::Selection
//! [`LibraryIndex`]: crate::query::index::LibraryIndex

use crate::db::repository::TrackRepository;
use crate::error::CoreError;
use crate::id::{AlbumId, TrackId};
use crate::model::Track;
use crate::query::Selection;
use crate::query::index::LibraryIndex;
use crate::query::sort::{Group, SortChain, SortField, SortKey};
use std::collections::{HashMap, HashSet};

/// One row of a filter pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterRow {
    /// What the pane shows. The display value, never the sort value: the album
    /// artist pane reads `The Omnific` even though it sorted under `O`.
    pub label: String,
    /// How many tracks this row accounts for, in this pane's context.
    pub count: usize,
    pub selected: bool,
}

/// A filter pane: `All (n)` pinned at the top, then rows (§7.2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterPane {
    /// The pinned `All (n)` count. Every row's count sums to this, which is the
    /// contract a pane makes by existing.
    pub total: usize,
    pub rows: Vec<FilterRow>,
}

impl FilterPane {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The rows the user has selected.
    #[must_use]
    pub fn selected(&self) -> Vec<&FilterRow> {
        self.rows.iter().filter(|r| r.selected).collect()
    }
}

/// A pane row keyed by album, since albums are filtered by id rather than name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumRow {
    pub id: AlbumId,
    pub label: String,
    pub count: usize,
    pub selected: bool,
}

/// The album pane. Kept separate from [`FilterPane`] because its rows carry an
/// id: two albums can legitimately share a title.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlbumPane {
    pub total: usize,
    pub rows: Vec<AlbumRow>,
}

/// Everything one query produced.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueryOutcome {
    /// The tracks to show, in order.
    ///
    /// Ids rather than rows: the caller already holds the
    /// [`LibraryIndex`](crate::query::index::LibraryIndex) these came from, so
    /// copying rows out would be duplicating what it can look up in constant
    /// time. This is also exactly what `Event::QueryResult` carries.
    pub tracks: Vec<TrackId>,
    pub album_artists: FilterPane,
    pub albums: AlbumPane,
    /// Group header positions, derived from the sort chain (§7.3).
    pub groups: Vec<Group>,
    /// Tracks in the library that the availability constraint admits, before
    /// any filtering. For "showing 42 of 307".
    pub total: usize,
}

impl QueryOutcome {
    #[must_use]
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }
}

/// Runs selections against a repository.
#[derive(Debug, Clone, Copy, Default)]
pub struct QueryEngine;

impl QueryEngine {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Runs a selection.
    ///
    /// `repository` is used only for full-text search, and only when the
    /// selection actually has a search term.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if full-text search fails.
    pub fn run<R: TrackRepository>(
        &self,
        index: &LibraryIndex,
        repository: &R,
        selection: &Selection,
    ) -> Result<QueryOutcome, CoreError> {
        // References throughout. Nothing is cloned until the very end, and then
        // only the ids.
        let mut candidates: Vec<&Track> = if selection.include_unavailable {
            index.tracks().iter().collect()
        } else {
            index.tracks().iter().filter(|t| t.available).collect()
        };
        let total = candidates.len();

        // Search sits above the cascade: it narrows what every pane counts
        // against, which is why it is applied before any of them.
        if let Some(query) = selection
            .search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let matches: HashSet<TrackId> = repository.search_tracks(query)?.into_iter().collect();
            candidates.retain(|t| matches.contains(&t.id));
        }

        // Album artist counts are computed *before* the album artist filter
        // applies. That is what makes a pane a cascade rather than a set of
        // independent filters: selecting an artist must not empty the pane the
        // selection was made in, or there would be no way to change your mind.
        let album_artists = artist_pane(&candidates, &selection.album_artists);

        if !selection.album_artists.is_empty() {
            let wanted: HashSet<&str> =
                selection.album_artists.iter().map(String::as_str).collect();
            candidates.retain(|t| wanted.contains(t.album_artist.as_str()));
        }

        // Album counts see the artist filter but not their own.
        let albums = album_pane(&candidates, &selection.albums);

        if !selection.albums.is_empty() {
            let wanted: HashSet<AlbumId> = selection.albums.iter().copied().collect();
            candidates.retain(|t| wanted.contains(&t.album_id));
        }

        candidates.sort_by(|a, b| selection.sort.compare(a, b));
        let groups = selection.sort.groups(&candidates);
        let tracks = candidates.iter().map(|t| t.id).collect();

        Ok(QueryOutcome {
            tracks,
            album_artists,
            albums,
            groups,
            total,
        })
    }
}

/// Counts tracks per album artist, ordered the way the pane shows them.
fn artist_pane(tracks: &[&Track], selected: &[String]) -> FilterPane {
    // Keyed on the display value, because that is what a Selection filters on.
    // The sort key is carried alongside so ordering can use it: §4.1 again, the
    // structural value and the displayed value are different fields.
    let mut counts: HashMap<&str, (usize, &str)> = HashMap::new();
    for track in tracks {
        let entry = counts
            .entry(track.album_artist.as_str())
            .or_insert((0, track.album_artist_key()));
        entry.0 += 1;
    }

    let selected: HashSet<&str> = selected.iter().map(String::as_str).collect();
    let mut rows: Vec<(&str, FilterRow)> = counts
        .into_iter()
        .map(|(display, (count, key))| {
            (
                key,
                FilterRow {
                    label: display.to_owned(),
                    count,
                    selected: selected.contains(display),
                },
            )
        })
        .collect();

    // Ordered by albumartistsort, using the same comparison the table uses so
    // the pane and the table agree about where an artist sits.
    rows.sort_by(|(a, arow), (b, brow)| {
        crate::query::sort::compare_display_text(a, b).then_with(|| arow.label.cmp(&brow.label))
    });

    FilterPane {
        total: tracks.len(),
        rows: rows.into_iter().map(|(_, row)| row).collect(),
    }
}

/// Counts tracks per album, ordered chronologically (§14).
fn album_pane(tracks: &[&Track], selected: &[AlbumId]) -> AlbumPane {
    let mut counts: HashMap<AlbumId, (usize, &Track)> = HashMap::new();
    for track in tracks {
        let entry = counts.entry(track.album_id).or_insert((0, *track));
        entry.0 += 1;
    }

    let selected: HashSet<AlbumId> = selected.iter().copied().collect();
    let mut rows: Vec<(&Track, AlbumRow)> = counts
        .into_iter()
        .map(|(id, (count, exemplar))| {
            (
                exemplar,
                AlbumRow {
                    id,
                    label: exemplar.album.clone(),
                    count,
                    selected: selected.contains(&id),
                },
            )
        })
        .collect();

    // Albums under an artist sort chronologically, not alphabetically -- the
    // same chain the table uses for its default preset.
    let chain = SortChain::new(vec![
        SortKey::asc(SortField::AlbumArtist),
        SortKey::asc(SortField::Album),
    ]);
    rows.sort_by(|(a, _), (b, _)| chain.compare(a, b));

    AlbumPane {
        total: tracks.len(),
        rows: rows.into_iter().map(|(_, row)| row).collect(),
    }
}
