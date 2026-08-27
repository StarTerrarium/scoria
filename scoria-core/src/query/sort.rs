//! Sort fields, keys and chains.
//!
//! `DESIGN.md` §4.4: a sort field is not a column. Sort value and display value
//! are frequently different fields -- `Album` sorts on `date` but displays
//! `album`, `AlbumArtist` sorts on `albumartistsort` but displays
//! `albumartist`. Modelling this as "sort by the string in this column" breaks
//! on the very first key.

use crate::date::Date;
use crate::model::Track;
use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::cmp::Ordering;

/// What a field yields for comparison.
///
/// One enum rather than a trait object because the set is closed and the
/// comparison rules -- particularly where missing values land -- have to be
/// stated in exactly one place.
#[derive(Debug, Clone, Copy, PartialEq)]
enum SortValue<'a> {
    /// The tag is absent. Always sorts last, in both directions (see
    /// [`SortKey::compare`]).
    Missing,
    Text(&'a str),
    Number(i64),
    Date(Date),
}

impl SortValue<'_> {
    /// Ordering among present values. `Missing` is handled by the caller,
    /// because where it lands depends on direction.
    fn cmp_present(self, other: Self) -> Ordering {
        match (self, other) {
            (Self::Text(a), Self::Text(b)) => compare_text(a, b),
            (Self::Number(a), Self::Number(b)) => a.cmp(&b),
            (Self::Date(a), Self::Date(b)) => a.cmp(&b),
            // A field always yields the same variant, so mixed variants cannot
            // arise. Ordering by discriminant keeps the comparator total
            // anyway; a sort that panics is not an option.
            _ => self.discriminant().cmp(&other.discriminant()),
        }
    }

    const fn discriminant(self) -> u8 {
        match self {
            Self::Missing => 0,
            Self::Text(_) => 1,
            Self::Number(_) => 2,
            Self::Date(_) => 3,
        }
    }
}

/// Case-insensitive, allocation-free, with a case-sensitive tiebreak so the
/// result is deterministic rather than merely consistent.
///
/// Comparing lowercase char-by-char avoids allocating on every comparison in a
/// sort of a six-figure library. It is not full Unicode collation, and does not
/// need to be: §4.4's point is that Picard writes latin sort names, so
/// `花冷え。` arrives here already as `HANABIE.`.
#[must_use]
pub fn compare_display_text(a: &str, b: &str) -> Ordering {
    compare_text(a, b)
}

fn compare_text(a: &str, b: &str) -> Ordering {
    // Unicode case folding is the correct path and the slow one: it dominates a
    // sort of a six-figure library, because a comparison sort makes millions of
    // these calls and most of them fold every character. When both sides are
    // ASCII -- which most artist names are -- byte-wise ASCII folding gives an
    // identical answer for a fraction of the work, since ASCII lowercasing and
    // Unicode lowercasing agree over ASCII.
    let folded = if a.is_ascii() && b.is_ascii() {
        a.bytes()
            .map(|c| c.to_ascii_lowercase())
            .cmp(b.bytes().map(|c| c.to_ascii_lowercase()))
    } else {
        a.chars()
            .flat_map(char::to_lowercase)
            .cmp(b.chars().flat_map(char::to_lowercase))
    };
    folded.then_with(|| a.cmp(b))
}

/// A field a track can be ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SortField {
    /// Sorts `albumartistsort` falling back to `albumartist`; displays
    /// `albumartist`.
    AlbumArtist,
    /// Sorts chronologically on `date`; displays `album`. This is what makes an
    /// artist's albums appear in release order (§14).
    Album,
    /// Sorts `album` alphabetically. Distinct from [`Self::Album`] on purpose:
    /// same column, different question.
    AlbumTitle,
    Title,
    /// Display artist. Available as a column sort; never used for structure.
    Artist,
    Date,
    Disc,
    TrackNumber,
    Duration,
    DateAdded,
    /// The terminal tiebreaker. Every chain ends here, which is what makes
    /// equal-key rows stable across runs and queue snapshots reproducible.
    TrackId,
}

impl SortField {
    fn value_of(self, track: &Track) -> SortValue<'_> {
        fn text(s: &str) -> SortValue<'_> {
            if s.is_empty() {
                SortValue::Missing
            } else {
                SortValue::Text(s)
            }
        }
        fn number(n: Option<u16>) -> SortValue<'static> {
            n.map_or(SortValue::Missing, |n| SortValue::Number(i64::from(n)))
        }
        fn date(d: Option<Date>) -> SortValue<'static> {
            d.map_or(SortValue::Missing, SortValue::Date)
        }

        match self {
            Self::AlbumArtist => text(track.album_artist_key()),
            Self::Album | Self::Date => date(track.date),
            Self::AlbumTitle => text(&track.album),
            Self::Title => text(&track.title),
            Self::Artist => text(&track.artist),
            Self::Disc => number(track.disc),
            Self::TrackNumber => number(track.track_number),
            Self::Duration => {
                // Milliseconds keep sub-second differences without floats.
                SortValue::Number(i64::try_from(track.duration.as_millis()).unwrap_or(i64::MAX))
            }
            Self::DateAdded => SortValue::Number(track.added_at.seconds()),
            Self::TrackId => SortValue::Number(i64::try_from(track.id.get()).unwrap_or(i64::MAX)),
        }
    }

    /// What a group header shows when this field leads the chain.
    ///
    /// The display value, not the sort value: an `Album` group shows the album
    /// title even though it sorted on the date.
    #[must_use]
    pub fn group_label(self, track: &Track) -> Option<String> {
        match self {
            Self::AlbumArtist => Some(track.album_artist.clone()),
            Self::Album | Self::AlbumTitle => Some(track.album.clone()),
            Self::Artist => Some(track.artist.clone()),
            Self::Date => track.date.map(|d| d.year().to_string()),
            _ => None,
        }
    }

    /// Whether grouping is meaningful when this field leads the chain.
    ///
    /// §7.3: grouping is derived, not configured -- a header row is inserted
    /// whenever the primary key's value changes. That is only useful for fields
    /// with repeated values, which is why sorting by duration switches grouping
    /// off automatically.
    #[must_use]
    pub const fn groups(self) -> bool {
        matches!(
            self,
            Self::AlbumArtist | Self::Album | Self::AlbumTitle | Self::Artist | Self::Date
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Direction {
    #[default]
    Ascending,
    Descending,
}

impl Direction {
    const fn apply(self, ordering: Ordering) -> Ordering {
        match self {
            Self::Ascending => ordering,
            Self::Descending => ordering.reverse(),
        }
    }
}

/// One link in a sort chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SortKey {
    pub field: SortField,
    pub direction: Direction,
}

impl SortKey {
    #[must_use]
    pub const fn asc(field: SortField) -> Self {
        Self {
            field,
            direction: Direction::Ascending,
        }
    }

    #[must_use]
    pub const fn desc(field: SortField) -> Self {
        Self {
            field,
            direction: Direction::Descending,
        }
    }

    /// Compares two tracks on this key alone.
    ///
    /// **Missing values always sort last**, in both directions. Reversing them
    /// along with everything else would mean that flipping the sort on a column
    /// throws every untagged track to the top, which reads as corruption rather
    /// than as an ordering choice. §4.4 requires only that the position be
    /// defined; this is the definition.
    #[must_use]
    pub fn compare(self, a: &Track, b: &Track) -> Ordering {
        let (va, vb) = (self.field.value_of(a), self.field.value_of(b));
        match (va, vb) {
            (SortValue::Missing, SortValue::Missing) => Ordering::Equal,
            (SortValue::Missing, _) => Ordering::Greater,
            (_, SortValue::Missing) => Ordering::Less,
            _ => self.direction.apply(va.cmp_present(vb)),
        }
    }
}

/// An ordered list of sort keys, guaranteed to end in a `TrackId` tiebreaker.
///
/// The guarantee is in the constructor rather than in a convention, so "every
/// sort chain ends in a `TrackId` tiebreaker" cannot be forgotten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortChain {
    keys: Vec<SortKey>,
}

impl SortChain {
    /// Appends the `TrackId` tiebreaker unless the caller already ended with one.
    #[must_use]
    pub fn new(mut keys: Vec<SortKey>) -> Self {
        if keys.last().map(|k| k.field) != Some(SortField::TrackId) {
            keys.push(SortKey::asc(SortField::TrackId));
        }
        Self { keys }
    }

    #[must_use]
    pub fn keys(&self) -> &[SortKey] {
        &self.keys
    }

    /// The field the chain leads with, ignoring a chain that is nothing but the
    /// tiebreaker.
    #[must_use]
    pub fn primary(&self) -> Option<SortField> {
        self.keys
            .first()
            .map(|k| k.field)
            .filter(|f| *f != SortField::TrackId)
    }

    /// The field group headers are derived from, if any (§7.3).
    #[must_use]
    pub fn group_field(&self) -> Option<SortField> {
        self.primary().filter(|f| f.groups())
    }

    /// Total ordering over tracks: each key in turn, tiebroken by `TrackId`.
    #[must_use]
    pub fn compare(&self, a: &Track, b: &Track) -> Ordering {
        self.keys
            .iter()
            .map(|key| key.compare(a, b))
            .find(|o| o.is_ne())
            .unwrap_or(Ordering::Equal)
    }

    /// Sorts in place. Stable, though the `TrackId` tiebreaker means stability
    /// is belt and braces rather than load-bearing.
    pub fn sort(&self, tracks: &mut [Track]) {
        tracks.sort_by(|a, b| self.compare(a, b));
    }
}

impl SortChain {
    /// Where group headers go, derived from the primary sort key (§7.3).
    ///
    /// Grouping is derived, not configured: a header is inserted wherever the
    /// primary key's value changes. That is why sorting by duration switches
    /// grouping off -- not as a special case, but because
    /// [`SortField::groups`] says duration does not group.
    ///
    /// `tracks` must already be sorted by this chain; grouping an unsorted
    /// slice would produce a header every time the value changed, which is
    /// most rows.
    #[must_use]
    pub fn groups<T: Borrow<Track>>(&self, tracks: &[T]) -> Vec<Group> {
        let Some(field) = self.group_field() else {
            return Vec::new();
        };

        let mut groups: Vec<Group> = Vec::new();
        for (index, track) in tracks.iter().enumerate() {
            let label = field.group_label(track.borrow()).unwrap_or_default();
            match groups.last_mut() {
                Some(last) if last.label == label => last.len += 1,
                _ => groups.push(Group {
                    label,
                    start: index,
                    len: 1,
                }),
            }
        }
        groups
    }
}

/// A run of adjacent rows sharing a primary sort value, with the header the
/// table draws above them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub label: String,
    /// Index into the sorted rows where the group starts.
    pub start: usize,
    pub len: usize,
}

impl Group {
    /// One past the last row of the group.
    #[must_use]
    pub const fn end(&self) -> usize {
        self.start + self.len
    }
}

impl Default for SortChain {
    fn default() -> Self {
        SortPreset::ArtistChronological.chain()
    }
}

/// The sort chains 1.0 exposes.
///
/// §7.3: presets rather than a chain builder. A drag-and-drop chain builder is
/// a DIY layout system wearing different clothes. The `Vec<SortKey>` shape is
/// here regardless, so custom chains later are a UI addition rather than a
/// data migration -- "hardcode the value, not the shape".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SortPreset {
    /// Album artist, then albums in release order, then disc and track. The
    /// default, and the reason jump-to-playing works without choosing a filter
    /// (§6.5).
    #[default]
    ArtistChronological,
    /// Album artist, then albums by title.
    ArtistAlphabetical,
    RecentlyAdded,
    AlbumTitle,
    Year,
}

impl SortPreset {
    #[must_use]
    pub fn chain(self) -> SortChain {
        use SortField as F;
        SortChain::new(match self {
            Self::ArtistChronological => vec![
                SortKey::asc(F::AlbumArtist),
                SortKey::asc(F::Album),
                SortKey::asc(F::Disc),
                SortKey::asc(F::TrackNumber),
            ],
            Self::ArtistAlphabetical => vec![
                SortKey::asc(F::AlbumArtist),
                SortKey::asc(F::AlbumTitle),
                SortKey::asc(F::Disc),
                SortKey::asc(F::TrackNumber),
            ],
            Self::RecentlyAdded => vec![SortKey::desc(F::DateAdded)],
            Self::AlbumTitle => vec![SortKey::asc(F::AlbumTitle), SortKey::asc(F::TrackNumber)],
            Self::Year => vec![
                SortKey::asc(F::Date),
                SortKey::asc(F::AlbumArtist),
                SortKey::asc(F::TrackNumber),
            ],
        })
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ArtistChronological => "Artist chronological",
            Self::ArtistAlphabetical => "Artist alphabetical",
            Self::RecentlyAdded => "Recently added",
            Self::AlbumTitle => "Album title",
            Self::Year => "Year",
        }
    }

    /// Every preset, in the order the UI offers them.
    #[must_use]
    pub const fn all() -> [Self; 5] {
        [
            Self::ArtistChronological,
            Self::ArtistAlphabetical,
            Self::RecentlyAdded,
            Self::AlbumTitle,
            Self::Year,
        ]
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;
    use crate::test_support::{TrackBuilder, ids, track};

    // ---------------------------------------------------------------- §4.4 --

    #[test]
    fn album_artist_sorts_on_the_sort_name_not_the_display_name() {
        // The example §4.4 cites: leading-article handling comes free from the
        // tag, with no special case in our code.
        let mut tracks = vec![
            TrackBuilder::new(1)
                .album_artist("The Omnific")
                .album_artist_sort("Omnific, The")
                .build(),
            TrackBuilder::new(2)
                .album_artist("Metallica")
                .album_artist_sort("Metallica")
                .build(),
            TrackBuilder::new(3)
                .album_artist("Nine Inch Nails")
                .album_artist_sort("Nine Inch Nails")
                .build(),
        ];
        SortChain::new(vec![SortKey::asc(SortField::AlbumArtist)]).sort(&mut tracks);

        // Metallica, Nine Inch Nails, Omnific -- not "The Omnific" under T.
        assert_eq!(ids(&tracks), vec![2, 3, 1]);
    }

    #[test]
    fn japanese_names_sort_by_their_latin_sort_name() {
        // The other §4.4 example. 花冷え。 would otherwise sort by codepoint,
        // far away from every latin-titled artist.
        let mut tracks = vec![
            TrackBuilder::new(1)
                .album_artist("花冷え。")
                .album_artist_sort("HANABIE.")
                .build(),
            TrackBuilder::new(2)
                .album_artist("Ghost")
                .album_artist_sort("Ghost")
                .build(),
            TrackBuilder::new(3)
                .album_artist("Iron Maiden")
                .album_artist_sort("Iron Maiden")
                .build(),
        ];
        SortChain::new(vec![SortKey::asc(SortField::AlbumArtist)]).sort(&mut tracks);

        // Ghost, HANABIE., Iron Maiden.
        assert_eq!(ids(&tracks), vec![2, 1, 3]);
    }

    #[test]
    fn album_artist_falls_back_to_the_display_name_when_untagged() {
        let mut tracks = vec![
            TrackBuilder::new(1).album_artist("Zebra").build(),
            TrackBuilder::new(2).album_artist("Alpha").build(),
        ];
        SortChain::new(vec![SortKey::asc(SortField::AlbumArtist)]).sort(&mut tracks);
        assert_eq!(ids(&tracks), vec![2, 1]);
    }

    #[test]
    fn text_comparison_ignores_case() {
        let mut tracks = vec![
            TrackBuilder::new(1).album_artist("beta").build(),
            TrackBuilder::new(2).album_artist("Alpha").build(),
            TrackBuilder::new(3).album_artist("Gamma").build(),
        ];
        SortChain::new(vec![SortKey::asc(SortField::AlbumArtist)]).sort(&mut tracks);
        assert_eq!(ids(&tracks), vec![2, 1, 3]);
    }

    #[test]
    fn album_sorts_chronologically_but_album_title_sorts_alphabetically() {
        // §14: albums under an artist sort chronologically, not alphabetically.
        // Same column in the table, two different questions.
        let build = || {
            vec![
                TrackBuilder::new(1).album("Zenith").date("1998").build(),
                TrackBuilder::new(2).album("Apex").date("2011").build(),
            ]
        };

        let mut chronological = build();
        SortChain::new(vec![SortKey::asc(SortField::Album)]).sort(&mut chronological);
        assert_eq!(ids(&chronological), vec![1, 2], "Album should sort on date");

        let mut alphabetical = build();
        SortChain::new(vec![SortKey::asc(SortField::AlbumTitle)]).sort(&mut alphabetical);
        assert_eq!(
            ids(&alphabetical),
            vec![2, 1],
            "AlbumTitle should sort on title"
        );
    }

    // ------------------------------------------------- missing values (§4.4) --

    #[test]
    fn missing_values_sort_last_ascending() {
        let mut tracks = vec![
            TrackBuilder::new(1).no_date().build(),
            TrackBuilder::new(2).date("2011").build(),
            TrackBuilder::new(3).date("1998").build(),
        ];
        SortChain::new(vec![SortKey::asc(SortField::Date)]).sort(&mut tracks);
        assert_eq!(ids(&tracks), vec![3, 2, 1]);
    }

    #[test]
    fn missing_values_sort_last_descending_too() {
        // Reversing them with everything else would throw every untagged album
        // to the top when a column is flipped, which reads as corruption.
        let mut tracks = vec![
            TrackBuilder::new(1).no_date().build(),
            TrackBuilder::new(2).date("2011").build(),
            TrackBuilder::new(3).date("1998").build(),
        ];
        SortChain::new(vec![SortKey::desc(SortField::Date)]).sort(&mut tracks);
        assert_eq!(ids(&tracks), vec![2, 3, 1]);
    }

    #[test]
    fn an_empty_string_counts_as_missing() {
        let mut tracks = vec![
            TrackBuilder::new(1).album_artist("").build(),
            TrackBuilder::new(2).album_artist("Alpha").build(),
        ];
        SortChain::new(vec![SortKey::asc(SortField::AlbumArtist)]).sort(&mut tracks);
        assert_eq!(ids(&tracks), vec![2, 1]);
    }

    #[test]
    fn tracks_missing_a_number_sort_after_numbered_ones() {
        let mut tracks = vec![
            TrackBuilder::new(1).no_track_number().build(),
            TrackBuilder::new(2).track_number(2).build(),
            TrackBuilder::new(3).track_number(1).build(),
        ];
        SortChain::new(vec![SortKey::asc(SortField::TrackNumber)]).sort(&mut tracks);
        assert_eq!(ids(&tracks), vec![3, 2, 1]);
    }

    // ------------------------------------------------------- the tiebreaker --

    #[test]
    fn every_chain_ends_in_the_track_id_tiebreaker() {
        let chain = SortChain::new(vec![SortKey::asc(SortField::Title)]);
        assert_eq!(
            chain.keys().last().map(|k| k.field),
            Some(SortField::TrackId)
        );
    }

    #[test]
    fn an_empty_chain_is_still_a_valid_total_order() {
        let chain = SortChain::new(vec![]);
        assert_eq!(chain.keys().len(), 1);
        assert_eq!(chain.primary(), None);
    }

    #[test]
    fn the_tiebreaker_is_not_appended_twice() {
        let chain = SortChain::new(vec![SortKey::asc(SortField::TrackId)]);
        assert_eq!(chain.keys().len(), 1);
    }

    #[test]
    fn equal_key_rows_are_stable_across_runs() {
        // §4.4: reproducible order is what makes a queue snapshot reproducible.
        // Every track here has identical sort keys, so only the tiebreaker
        // decides -- and it must decide the same way from any input order.
        let chain = SortChain::new(vec![SortKey::asc(SortField::AlbumArtist)]);

        let mut forwards: Vec<_> = (1..=5).map(track).collect();
        let mut backwards: Vec<_> = (1..=5).rev().map(track).collect();
        chain.sort(&mut forwards);
        chain.sort(&mut backwards);

        assert_eq!(ids(&forwards), vec![1, 2, 3, 4, 5]);
        assert_eq!(ids(&forwards), ids(&backwards));
    }

    #[test]
    fn comparison_is_a_total_order() {
        // sort_by on an inconsistent comparator can panic. Check the three laws
        // hold across a deliberately awkward set.
        let tracks = vec![
            TrackBuilder::new(1).album_artist("a").no_date().build(),
            TrackBuilder::new(2).album_artist("A").date("2020").build(),
            TrackBuilder::new(3)
                .album_artist("")
                .date("2020-01")
                .build(),
            TrackBuilder::new(4).album_artist("b").no_date().build(),
        ];
        let chain = SortChain::new(vec![
            SortKey::asc(SortField::AlbumArtist),
            SortKey::desc(SortField::Date),
        ]);

        for a in &tracks {
            assert_eq!(chain.compare(a, a), Ordering::Equal, "not reflexive");
            for b in &tracks {
                assert_eq!(
                    chain.compare(a, b),
                    chain.compare(b, a).reverse(),
                    "not antisymmetric"
                );
                for c in &tracks {
                    let (ab, bc) = (chain.compare(a, b), chain.compare(b, c));
                    if ab == bc && ab != Ordering::Equal {
                        assert_eq!(chain.compare(a, c), ab, "not transitive");
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------ §7.3 sort --

    #[test]
    fn the_default_preset_orders_a_discography_the_way_the_design_describes() {
        // §6.5 leans on this: landing on the playing track in an unfiltered
        // library puts it inside its album, with the artist's other albums
        // immediately above and below.
        let mut tracks = vec![
            TrackBuilder::new(1)
                .album_artist_sort("Ghost")
                .album("Meliora")
                .date("2015")
                .disc(1)
                .track_number(2)
                .build(),
            TrackBuilder::new(2)
                .album_artist_sort("Ghost")
                .album("Meliora")
                .date("2015")
                .disc(1)
                .track_number(1)
                .build(),
            TrackBuilder::new(3)
                .album_artist_sort("Ghost")
                .album("Opus Eponymous")
                .date("2010")
                .disc(1)
                .track_number(1)
                .build(),
            TrackBuilder::new(4)
                .album_artist_sort("Alcest")
                .album("Écailles de Lune")
                .date("2010")
                .disc(1)
                .track_number(1)
                .build(),
            TrackBuilder::new(5)
                .album_artist_sort("Ghost")
                .album("Meliora")
                .date("2015")
                .disc(2)
                .track_number(1)
                .build(),
        ];
        SortPreset::ArtistChronological.chain().sort(&mut tracks);

        // Alcest first, then Ghost's albums oldest to newest, then disc, track.
        assert_eq!(ids(&tracks), vec![4, 3, 2, 1, 5]);
    }

    #[test]
    fn recently_added_puts_the_newest_first() {
        let mut tracks = vec![
            TrackBuilder::new(1).added_at(100).build(),
            TrackBuilder::new(2).added_at(300).build(),
            TrackBuilder::new(3).added_at(200).build(),
        ];
        SortPreset::RecentlyAdded.chain().sort(&mut tracks);
        assert_eq!(ids(&tracks), vec![2, 3, 1]);
    }

    #[test]
    fn every_preset_is_labelled_and_ends_in_the_tiebreaker() {
        for preset in SortPreset::all() {
            let chain = preset.chain();
            assert!(!preset.label().is_empty());
            assert_eq!(
                chain.keys().last().map(|k| k.field),
                Some(SortField::TrackId),
                "{:?} is missing the tiebreaker",
                preset
            );
        }
    }

    // -------------------------------------------------------- grouping §7.3 --

    #[test]
    fn grouping_follows_the_primary_key() {
        assert_eq!(
            SortPreset::ArtistChronological.chain().group_field(),
            Some(SortField::AlbumArtist)
        );
        assert_eq!(
            SortPreset::AlbumTitle.chain().group_field(),
            Some(SortField::AlbumTitle)
        );
    }

    #[test]
    fn sorting_by_duration_switches_grouping_off() {
        // §7.3 states this outcome; it falls out of the field, not a special case.
        let chain = SortChain::new(vec![SortKey::asc(SortField::Duration)]);
        assert_eq!(chain.primary(), Some(SortField::Duration));
        assert_eq!(chain.group_field(), None);
    }

    #[test]
    fn a_group_header_shows_the_display_value_not_the_sort_value() {
        // Album sorts on date; the header still has to read "Meliora".
        let t = TrackBuilder::new(1).album("Meliora").date("2015").build();
        assert_eq!(SortField::Album.group_label(&t), Some("Meliora".to_owned()));

        // Album artist sorts on "Omnific, The"; the header reads "The Omnific".
        let t = TrackBuilder::new(1)
            .album_artist("The Omnific")
            .album_artist_sort("Omnific, The")
            .build();
        assert_eq!(
            SortField::AlbumArtist.group_label(&t),
            Some("The Omnific".to_owned())
        );
    }

    #[test]
    fn duration_orders_by_actual_length() {
        let mut tracks = vec![
            TrackBuilder::new(1).duration(300).build(),
            TrackBuilder::new(2).duration(100).build(),
            TrackBuilder::new(3).duration(200).build(),
        ];
        SortChain::new(vec![SortKey::asc(SortField::Duration)]).sort(&mut tracks);
        assert_eq!(ids(&tracks), vec![2, 3, 1]);
    }

    #[test]
    fn a_chain_survives_a_serde_round_trip() {
        // The chain is persisted in settings and travels in a Selection.
        let chain = SortPreset::ArtistChronological.chain();
        let json = serde_json::to_string(&chain).unwrap();
        assert_eq!(serde_json::from_str::<SortChain>(&json).unwrap(), chain);
    }
}

#[cfg(test)]
mod fast_path_tests {
    use super::*;

    /// The Unicode path, kept here so the ASCII fast path can be checked
    /// against it rather than against expectations.
    fn reference(a: &str, b: &str) -> Ordering {
        a.chars()
            .flat_map(char::to_lowercase)
            .cmp(b.chars().flat_map(char::to_lowercase))
            .then_with(|| a.cmp(b))
    }

    #[test]
    fn the_ascii_fast_path_agrees_with_unicode_folding() {
        let names = [
            "Metallica",
            "metallica",
            "METALLICA",
            "MOS",
            "Mos",
            "",
            "Teräsbetoni",
            "TERÄSBETONI",
            "花冷え。",
            "HANABIE.",
            "Omnific, The",
            "The Omnific",
            "dEUS",
            "k.d. lang",
            "AC/DC",
            "Sigur Rós",
            "Björk",
        ];
        for a in names {
            for b in names {
                assert_eq!(
                    compare_text(a, b),
                    reference(a, b),
                    "fast path disagrees for {a:?} vs {b:?}"
                );
            }
        }
    }
}
