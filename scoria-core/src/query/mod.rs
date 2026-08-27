//! Query specification.
//!
//! `Selection` describes what to show; turning it into SQL is the repository
//! layer's job (M4). It is defined here because `Command::Query` carries it
//! across the seam, so its shape is part of the contract.

pub mod engine;
pub mod index;
pub mod sort;

pub use engine::{AlbumPane, AlbumRow, FilterPane, FilterRow, QueryEngine, QueryOutcome};
pub use index::LibraryIndex;

use crate::id::AlbumId;
use serde::{Deserialize, Serialize};
use sort::SortChain;

/// The filter cascade plus an ordering.
///
/// `DESIGN.md` §7.2: album artist, then album. Empty means "All" at that level,
/// which is why `All (n)` is pinned to the top of each pane rather than being a
/// separate mode.
///
/// **There is no genre field, and there will not be one.** §7.2 explains why: a
/// filter pane implies its rows partition the library, and genre is many-valued
/// per track, so either the counts stop adding up to `All` or we invent a
/// primary-genre rule that is not in the tags.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    /// Album artists to include. Empty means all.
    ///
    /// Keyed on `albumartist`, never on `artist` -- §4.1, and invariant 7.
    pub album_artists: Vec<String>,
    /// Albums to include. Empty means all.
    pub albums: Vec<AlbumId>,
    /// Free-text search, run through FTS5.
    pub search: Option<String>,
    pub sort: SortChain,
    /// Whether tracks whose files no longer resolve are included. They are by
    /// default: §7.4 shows them struck through rather than dropping them,
    /// because silently vanishing rows make users doubt the software.
    pub include_unavailable: bool,
}

impl Selection {
    /// Everything, in the default order.
    #[must_use]
    pub fn all() -> Self {
        Self {
            include_unavailable: true,
            ..Self::default()
        }
    }

    /// Whether any filter is applied. Drives the "showing a subset" indicator
    /// and the queue overlay's divergence footer (§9.2).
    #[must_use]
    pub fn is_filtered(&self) -> bool {
        !self.album_artists.is_empty()
            || !self.albums.is_empty()
            || self.search.as_ref().is_some_and(|s| !s.trim().is_empty())
    }

    /// A human-readable label for the transport bar and queue header.
    ///
    /// This becomes an `Origin::Filtered`, which is display metadata and drives
    /// nothing (invariant 5).
    #[must_use]
    pub fn label(&self) -> Option<String> {
        match (self.album_artists.as_slice(), self.albums.is_empty()) {
            ([one], true) => Some(one.clone()),
            ([], _) => None,
            (many, _) => Some(format!("{} artists", many.len())),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn an_unfiltered_selection_is_not_filtered() {
        assert!(!Selection::all().is_filtered());
        assert_eq!(Selection::all().label(), None);
    }

    #[test]
    fn whitespace_search_does_not_count_as_a_filter() {
        let sel = Selection {
            search: Some("   ".to_owned()),
            ..Selection::all()
        };
        assert!(!sel.is_filtered());
    }

    #[test]
    fn a_single_artist_labels_itself() {
        let sel = Selection {
            album_artists: vec!["Electric Callboy".to_owned()],
            ..Selection::all()
        };
        assert!(sel.is_filtered());
        assert_eq!(sel.label().as_deref(), Some("Electric Callboy"));
    }

    #[test]
    fn several_artists_are_counted_rather_than_listed() {
        let sel = Selection {
            album_artists: vec!["Ghost".to_owned(), "Alcest".to_owned()],
            ..Selection::all()
        };
        assert_eq!(sel.label().as_deref(), Some("2 artists"));
    }

    #[test]
    fn defaults_to_the_default_sort_chain() {
        assert_eq!(Selection::all().sort, SortChain::default());
    }

    #[test]
    fn survives_a_serde_round_trip() {
        let sel = Selection {
            album_artists: vec!["花冷え。".to_owned()],
            ..Selection::all()
        };
        let json = serde_json::to_string(&sel).unwrap();
        assert_eq!(serde_json::from_str::<Selection>(&json).unwrap(), sel);
    }
}
