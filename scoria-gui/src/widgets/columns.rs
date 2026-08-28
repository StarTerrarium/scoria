//! The track table's columns.
//!
//! `DESIGN.md` §7.3: show/hide, reorder and resize, from a right-click on the
//! header. This is configuration that tunes a good default -- bounded,
//! discoverable and ignorable -- and it is what `CLAUDE.md` invariant 4 permits
//! while banning anything that lets the user *construct* a layout.
//!
//! **Widths are weights, not pixels.** A column sized on one monitor is wrong
//! on the next one, the same way a fixed drawer height was. Weights are handed
//! to the layout, which divides whatever width there is.

use scoria_core::model::Track;
use serde::{Deserialize, Serialize};

/// A column of the track table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Column {
    TrackNumber,
    Title,
    /// §4.1: structural. Shown by default alongside `Artist`, deliberately.
    AlbumArtist,
    /// §4.1: display, carrying the full credits.
    Artist,
    Album,
    Duration,
    Year,
}

impl Column {
    /// Every column, for the show/hide menu.
    pub const ALL: [Self; 7] = [
        Self::TrackNumber,
        Self::Title,
        Self::AlbumArtist,
        Self::Artist,
        Self::Album,
        Self::Duration,
        Self::Year,
    ];

    #[must_use]
    pub const fn heading(self) -> &'static str {
        match self {
            Self::TrackNumber => "#",
            Self::Title => "TITLE",
            Self::AlbumArtist => "ALBUM ARTIST",
            Self::Artist => "ARTIST",
            Self::Album => "ALBUM",
            Self::Duration => "TIME",
            Self::Year => "YEAR",
        }
    }

    /// The longer name, for a menu where "#" would be a poor label.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::TrackNumber => "Track number",
            Self::Title => "Title",
            Self::AlbumArtist => "Album artist",
            Self::Artist => "Artist",
            Self::Album => "Album",
            Self::Duration => "Duration",
            Self::Year => "Year",
        }
    }

    /// What this column shows for a track.
    #[must_use]
    pub fn value(self, track: &Track) -> String {
        match self {
            Self::TrackNumber => track
                .track_number
                .map_or_else(String::new, |n| n.to_string()),
            Self::Title => track.title.clone(),
            Self::AlbumArtist => track.album_artist.clone(),
            Self::Artist => track.artist.clone(),
            Self::Album => track.album.clone(),
            Self::Duration => crate::widgets::format_duration(track.duration),
            Self::Year => track
                .date
                .map_or_else(String::new, |d| d.year().to_string()),
        }
    }

    /// How wide it opens, relative to the others.
    const fn default_weight(self) -> u16 {
        match self {
            Self::TrackNumber => 34,
            Self::Title => 300,
            Self::AlbumArtist => 200,
            Self::Artist => 200,
            Self::Album => 200,
            Self::Duration => 60,
            Self::Year => 50,
        }
    }

    /// Which sort field a click on this heading asks for (§7.3).
    ///
    /// **Sorts by what the column shows.** That is the whole contract of a
    /// clickable heading, and it is not the same as picking the sort field with
    /// the matching name -- §4.4 is explicit that a sort field is not a column.
    ///
    /// `Album` is where the two diverge. `SortField::Album` sorts on the
    /// *date* and displays the title, which is what puts an artist's albums in
    /// release order in the default preset. Wiring the heading to it sorted the
    /// library chronologically while showing titles, so the result looked
    /// shuffled: Dirt at the top and a 2026 record at the bottom, with
    /// everything in between apparently unordered. `AlbumTitle` is the field
    /// that means what the column shows.
    ///
    /// `AlbumArtist` is a deliberate exception: it orders on
    /// `albumartistsort`, so `The Omnific` files under O. That is §4.4's
    /// central case and the reason the Picard assumption matters, not an
    /// oversight.
    #[must_use]
    pub const fn sort_field(self) -> scoria_core::SortField {
        use scoria_core::SortField as F;
        match self {
            Self::TrackNumber => F::TrackNumber,
            Self::Title => F::Title,
            Self::AlbumArtist => F::AlbumArtist,
            Self::Artist => F::Artist,
            Self::Album => F::AlbumTitle,
            Self::Duration => F::Duration,
            Self::Year => F::Date,
        }
    }
}

/// One column's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnSetting {
    pub column: Column,
    pub visible: bool,
    /// Share of the table's width, relative to the other visible columns.
    pub weight: u16,
}

/// Never let a column be dragged away to nothing: a zero-width column cannot be
/// grabbed again, which would make the drag a one-way door.
pub const MIN_WEIGHT: u16 = 16;

/// The columns, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ColumnLayout {
    columns: Vec<ColumnSetting>,
}

impl Default for ColumnLayout {
    /// §7.3's default set.
    ///
    /// The order here is the one the application ships with rather than the
    /// order §7.3 lists them in: duration reads better beside the other short
    /// numeric columns than wedged between the title and the artist.
    fn default() -> Self {
        Self {
            columns: [
                Column::TrackNumber,
                Column::Title,
                Column::AlbumArtist,
                Column::Artist,
                Column::Album,
                Column::Duration,
                Column::Year,
            ]
            .into_iter()
            .map(|column| ColumnSetting {
                column,
                visible: true,
                weight: column.default_weight(),
            })
            .collect(),
        }
    }
}

impl ColumnLayout {
    /// The columns actually drawn, in order.
    #[must_use]
    pub fn visible(&self) -> Vec<ColumnSetting> {
        self.columns.iter().copied().filter(|c| c.visible).collect()
    }

    /// Every column, including hidden ones, for the menu.
    #[must_use]
    pub fn all(&self) -> &[ColumnSetting] {
        &self.columns
    }

    /// Repairs a layout read from a file.
    ///
    /// A settings file can be older than the application, hand-edited, or both.
    /// Missing columns are appended and duplicates dropped, so a stale file
    /// loses a column rather than the application losing a column it needs.
    #[must_use]
    pub fn repaired(mut self) -> Self {
        let mut seen = std::collections::HashSet::new();
        self.columns
            .retain(|c| seen.insert(c.column) && c.weight >= MIN_WEIGHT);

        for column in Column::ALL {
            if !seen.contains(&column) {
                self.columns.push(ColumnSetting {
                    column,
                    visible: true,
                    weight: column.default_weight(),
                });
            }
        }
        self
    }

    /// Shows or hides a column.
    ///
    /// Refuses to hide the last visible one: a table with no columns shows
    /// nothing and offers no way back, since the header it is configured from
    /// would be empty too.
    pub fn set_visible(&mut self, column: Column, visible: bool) {
        if !visible && self.visible().len() <= 1 {
            return;
        }
        if let Some(setting) = self.columns.iter_mut().find(|c| c.column == column) {
            setting.visible = visible;
        }
    }

    pub fn toggle(&mut self, column: Column) {
        let visible = self
            .columns
            .iter()
            .find(|c| c.column == column)
            .is_some_and(|c| c.visible);
        self.set_visible(column, !visible);
    }

    /// Moves a column one place left or right among the visible ones.
    pub fn reorder(&mut self, column: Column, delta: isize) {
        let Some(from) = self.columns.iter().position(|c| c.column == column) else {
            return;
        };
        let Some(to) = from.checked_add_signed(delta) else {
            return;
        };
        if to >= self.columns.len() {
            return;
        }
        self.columns.swap(from, to);
    }

    /// Total weight of the visible columns, for turning one into a fraction.
    #[must_use]
    pub fn total_weight(&self) -> u32 {
        self.visible().iter().map(|c| u32::from(c.weight)).sum()
    }

    /// Drags the right edge of the `index`th visible column to `fraction` of
    /// the table's width.
    ///
    /// Width is taken from the column to the right, so the columns beyond the
    /// one being dragged stay where they are. Dragging the last column's edge
    /// does nothing, because there is nothing to take from -- the table has no
    /// slack of its own.
    pub fn drag_edge(&mut self, index: usize, fraction: f32) {
        let visible = self.visible();
        let Some(dragged) = visible.get(index) else {
            return;
        };
        let Some(neighbour) = visible.get(index + 1) else {
            return;
        };

        let total = self.total_weight();
        if total == 0 {
            return;
        }

        let before: u32 = visible
            .iter()
            .take(index)
            .map(|c| u32::from(c.weight))
            .sum();
        let wanted = (fraction.clamp(0.0, 1.0) * total as f32).round() as i64 - i64::from(before);

        let pair = i64::from(dragged.weight) + i64::from(neighbour.weight);
        let low = i64::from(MIN_WEIGHT);
        let high = pair - low;
        if high < low {
            return;
        }

        let new_dragged = wanted.clamp(low, high);
        let new_neighbour = pair - new_dragged;

        self.set_weight(dragged.column, new_dragged);
        self.set_weight(neighbour.column, new_neighbour);
    }

    fn set_weight(&mut self, column: Column, weight: i64) {
        if let Some(setting) = self.columns.iter_mut().find(|c| c.column == column) {
            setting.weight = u16::try_from(weight.max(i64::from(MIN_WEIGHT))).unwrap_or(u16::MAX);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::indexing_slicing, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn clicking_a_heading_sorts_by_what_that_column_shows() {
        // The bug this guards: ALBUM was wired to SortField::Album, which sorts
        // on the date and displays the title. Clicking it ordered the library
        // chronologically while showing album names, which reads as no order at
        // all.
        use scoria_core::SortField as F;

        for (column, expected) in [
            (Column::TrackNumber, F::TrackNumber),
            (Column::Title, F::Title),
            (Column::Artist, F::Artist),
            (Column::Album, F::AlbumTitle),
            (Column::Duration, F::Duration),
            (Column::Year, F::Date),
        ] {
            assert_eq!(
                column.sort_field(),
                expected,
                "{} should sort by what it displays",
                column.name()
            );
        }
    }

    #[test]
    fn the_album_artist_heading_orders_by_sort_name_on_purpose() {
        // §4.4's central case: `The Omnific` files under O. The one heading
        // whose ordering is deliberately not its displayed text.
        assert_eq!(
            Column::AlbumArtist.sort_field(),
            scoria_core::SortField::AlbumArtist
        );
    }

    #[test]
    fn no_heading_sorts_by_a_field_it_does_not_show() {
        // A column ordering on something invisible looks broken, whatever the
        // ordering is. AlbumArtist is the documented exception above.
        for column in Column::ALL {
            if column == Column::AlbumArtist {
                continue;
            }
            assert_ne!(
                column.sort_field(),
                scoria_core::SortField::Album,
                "{} would sort on a date while showing something else",
                column.name()
            );
        }
    }

    #[test]
    fn the_default_shows_every_column() {
        let layout = ColumnLayout::default();
        assert_eq!(layout.visible().len(), Column::ALL.len());
    }

    #[test]
    fn hiding_the_last_column_is_refused() {
        // A table with no columns shows nothing and offers no way back: the
        // header it is configured from would be empty too.
        let mut layout = ColumnLayout::default();
        for column in Column::ALL {
            layout.set_visible(column, false);
        }
        assert_eq!(layout.visible().len(), 1, "something must remain");
    }

    #[test]
    fn a_hidden_column_can_be_brought_back() {
        let mut layout = ColumnLayout::default();
        layout.toggle(Column::Artist);
        assert!(!layout.visible().iter().any(|c| c.column == Column::Artist));
        layout.toggle(Column::Artist);
        assert!(layout.visible().iter().any(|c| c.column == Column::Artist));
    }

    #[test]
    fn reordering_moves_one_place_and_stops_at_the_ends() {
        let mut layout = ColumnLayout::default();
        let first = layout.all()[0].column;
        layout.reorder(first, -1);
        assert_eq!(
            layout.all()[0].column,
            first,
            "should not fall off the start"
        );

        layout.reorder(first, 1);
        assert_eq!(layout.all()[1].column, first);
    }

    #[test]
    fn dragging_an_edge_takes_width_from_the_neighbour() {
        // The columns past the one being dragged must not move: a table where
        // one drag reflows everything is one nobody can arrange.
        let mut layout = ColumnLayout::default();
        let before = layout.total_weight();
        let tail: Vec<u16> = layout.visible().iter().skip(2).map(|c| c.weight).collect();

        layout.drag_edge(0, 0.25);

        assert_eq!(
            layout.total_weight(),
            before,
            "total width should not change"
        );
        let after: Vec<u16> = layout.visible().iter().skip(2).map(|c| c.weight).collect();
        assert_eq!(after, tail, "columns beyond the drag moved");
    }

    #[test]
    fn a_column_cannot_be_dragged_away_to_nothing() {
        // A zero-width column cannot be grabbed again, which would make the
        // drag a one-way door.
        let mut layout = ColumnLayout::default();
        layout.drag_edge(0, 0.0);
        assert!(layout.visible()[0].weight >= MIN_WEIGHT);

        layout.drag_edge(0, 1.0);
        assert!(layout.visible()[1].weight >= MIN_WEIGHT);
    }

    #[test]
    fn dragging_the_last_edge_does_nothing() {
        // There is nothing to the right to take width from.
        let mut layout = ColumnLayout::default();
        let before = layout.visible();
        layout.drag_edge(before.len() - 1, 0.5);
        assert_eq!(layout.visible(), before);
    }

    #[test]
    fn a_stale_settings_file_gains_the_columns_it_is_missing() {
        // A file written by an older build should cost the reader a column, not
        // the application one.
        let layout = ColumnLayout {
            columns: vec![ColumnSetting {
                column: Column::Title,
                visible: true,
                weight: 100,
            }],
        }
        .repaired();

        assert_eq!(layout.all().len(), Column::ALL.len());
        assert_eq!(
            layout.all()[0].column,
            Column::Title,
            "the stored order comes first"
        );
    }

    #[test]
    fn a_hand_edited_file_cannot_produce_duplicates_or_slivers() {
        let layout = ColumnLayout {
            columns: vec![
                ColumnSetting {
                    column: Column::Title,
                    visible: true,
                    weight: 100,
                },
                ColumnSetting {
                    column: Column::Title,
                    visible: true,
                    weight: 100,
                },
                ColumnSetting {
                    column: Column::Album,
                    visible: true,
                    weight: 0,
                },
            ],
        }
        .repaired();

        assert_eq!(
            layout
                .all()
                .iter()
                .filter(|c| c.column == Column::Title)
                .count(),
            1
        );
        assert!(layout.all().iter().all(|c| c.weight >= MIN_WEIGHT));
    }

    #[test]
    fn a_layout_survives_a_serde_round_trip() {
        let layout = ColumnLayout::default();
        let text = toml::to_string_pretty(&Wrapper {
            columns: layout.clone(),
        })
        .unwrap();
        let back: Wrapper = toml::from_str(&text).unwrap();
        assert_eq!(back.columns, layout);
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Wrapper {
        columns: ColumnLayout,
    }
}
