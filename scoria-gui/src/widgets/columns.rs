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

    // Off by default. §7.3's default set is what most people want to look at;
    // these are here because a library-centric player is also a place where
    // people go to answer questions about their library, and the answers are
    // in the tags.
    Disc,
    /// The full release date, where `Year` shows only the year.
    Released,
    /// When the scan first saw the file.
    Added,
    /// §7.2 rules out a genre *pane* -- pane rows have to partition the
    /// library, and genre is many-valued. A column renders the tag as written
    /// and promises nothing about counts, so the argument does not reach it.
    Genre,
    Composer,
    /// The track's own ReplayGain (§6.7). Blank where the file is untagged,
    /// which is what makes the coverage visible track by track.
    Gain,
    /// What the file is, as the decoder sees it.
    Format,
    Bitrate,
    SampleRate,
    BitDepth,
    /// The path relative to the library root, for finding one's way around.
    File,
}

impl Column {
    /// Every column, for the show/hide menu.
    pub const ALL: [Self; 18] = [
        Self::TrackNumber,
        Self::Title,
        Self::AlbumArtist,
        Self::Artist,
        Self::Album,
        Self::Duration,
        Self::Year,
        Self::Disc,
        Self::Released,
        Self::Added,
        Self::Genre,
        Self::Composer,
        Self::Gain,
        Self::Format,
        Self::Bitrate,
        Self::SampleRate,
        Self::BitDepth,
        Self::File,
    ];

    /// Whether it is shown before anyone chooses.
    ///
    /// §7.3's default set, and nothing else. This is what stops a new column
    /// appearing unbidden in a table someone already arranged: `repaired`
    /// appends columns a settings file has never heard of, and appending them
    /// visible would rearrange the table on upgrade.
    #[must_use]
    pub const fn visible_by_default(self) -> bool {
        matches!(
            self,
            Self::TrackNumber
                | Self::Title
                | Self::AlbumArtist
                | Self::Artist
                | Self::Album
                | Self::Duration
                | Self::Year
        )
    }

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
            Self::Disc => "DISC",
            Self::Released => "RELEASED",
            Self::Added => "ADDED",
            Self::Genre => "GENRE",
            Self::Composer => "COMPOSER",
            Self::Gain => "GAIN",
            Self::Format => "FORMAT",
            Self::Bitrate => "BITRATE",
            Self::SampleRate => "RATE",
            Self::BitDepth => "DEPTH",
            Self::File => "FILE",
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
            Self::Disc => "Disc number",
            Self::Released => "Release date",
            Self::Added => "Date added",
            Self::Genre => "Genre",
            Self::Composer => "Composer",
            Self::Gain => "ReplayGain",
            Self::Format => "File format",
            Self::Bitrate => "Bitrate",
            Self::SampleRate => "Sample rate",
            Self::BitDepth => "Bit depth",
            Self::File => "File path",
        }
    }

    /// The marker shown instead of a track number on the playing row.
    ///
    /// A glyph rather than a colour, because colour alone is not enough here:
    /// §11 makes the now-playing tint quiet and ambient on purpose, and a row
    /// that is *also* selected takes the selection's background -- leaving the
    /// two indistinguishable exactly when it matters, since the queue opens
    /// with its cursor on the playing row.
    pub const PLAYING_MARKER: &'static str = "▶";

    /// What this column shows, given whether the row is playing.
    ///
    /// Only the track-number column changes, which is the least missed: in a
    /// play order, where you are matters more than a track's number within its
    /// own album.
    #[must_use]
    pub fn value_for(self, track: &Track, playing: bool) -> String {
        if playing && matches!(self, Self::TrackNumber) {
            return Self::PLAYING_MARKER.to_owned();
        }
        self.value(track)
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
            Self::Disc => track.disc.map_or_else(String::new, |d| d.to_string()),
            Self::Released => track.date.map_or_else(String::new, |d| d.to_string()),
            Self::Added => track.added_at.to_date_string(),
            Self::Genre => track.genre.clone().unwrap_or_default(),
            Self::Composer => track.composer.clone().unwrap_or_default(),
            // Blank where untagged, rather than "0.0 dB". Zero is what an
            // untagged track is *played* at (§6.7), and printing it here would
            // make an absent tag look like a measured one.
            Self::Gain => track
                .replay_gain
                .track_gain_db
                .map_or_else(String::new, |db| format!("{db:+.1} dB")),
            Self::Format => track
                .path
                .as_path()
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_uppercase)
                .unwrap_or_default(),
            Self::Bitrate => track
                .audio
                .bitrate_kbps
                .map_or_else(String::new, |k| format!("{k} kbps")),
            // kHz with one decimal: 44.1 and 48 are the two everyone reads, and
            // 44100 makes the column wide for no gain.
            Self::SampleRate => track.audio.sample_rate_hz.map_or_else(String::new, |hz| {
                format!("{:.1} kHz", f64::from(hz) / 1000.0)
            }),
            Self::BitDepth => track
                .audio
                .bit_depth
                .map_or_else(String::new, |d| format!("{d}-bit")),
            Self::File => track.path.as_path().to_string_lossy().into_owned(),
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
            Self::Disc => 40,
            Self::Released => 90,
            Self::Added => 90,
            Self::Genre => 160,
            Self::Composer => 160,
            Self::Gain => 70,
            Self::Format => 60,
            Self::Bitrate => 80,
            Self::SampleRate => 70,
            Self::BitDepth => 60,
            Self::File => 300,
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
            Self::Year | Self::Released => F::Date,
            Self::Disc => F::Disc,
            Self::Added => F::DateAdded,
            Self::Genre => F::Genre,
            Self::Composer => F::Composer,
            Self::Gain => F::TrackGain,
            // The extension is the front of the path, near enough: sorting by
            // it groups the formats together, which is the question being
            // asked, and there is no stored format field to sort on.
            Self::Format | Self::File => F::Path,
            Self::Bitrate => F::Bitrate,
            Self::SampleRate => F::SampleRate,
            Self::BitDepth => F::BitDepth,
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
    /// Every column is present; the optional ones are simply off (§7.3). That
    /// is what makes the panel a list of what exists rather than a list of what
    /// happens to be on.
    ///
    /// The order is `Column::ALL`, which is the one the application ships with
    /// rather than the order §7.3 lists them in: duration reads better beside
    /// the other short numeric columns than wedged between the title and the
    /// artist.
    fn default() -> Self {
        Self {
            columns: Column::ALL
                .into_iter()
                .map(|column| ColumnSetting {
                    column,
                    visible: column.visible_by_default(),
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
                    visible: column.visible_by_default(),
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

    /// Moves the column at visible position `from` to visible position `to`.
    ///
    /// Positions are among the *visible* columns, because that is what the
    /// header shows and what a drag can point at. Hidden columns keep their
    /// place in the stored order relative to their neighbours, so turning one
    /// back on puts it back where it was rather than at the end.
    pub fn move_visible(&mut self, from: usize, to: usize) {
        let visible: Vec<usize> = self
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.visible)
            .map(|(i, _)| i)
            .collect();

        let (Some(&source), Some(&target)) = (visible.get(from), visible.get(to)) else {
            return;
        };
        if source == target {
            return;
        }

        let setting = self.columns.remove(source);
        // Removing shifts everything after it down one, so a rightward move
        // lands after the target and a leftward move lands on it.
        let insert_at = if source < target {
            target
        } else {
            target.min(self.columns.len())
        };
        self.columns.insert(insert_at, setting);
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

    /// Which visible column sits at `fraction` of the table's width.
    ///
    /// The same geometry [`Self::drag_edge`] works in, and for the same reason:
    /// the header is laid out by weight, so a position in the header is a
    /// position in the weights. Divider widths are ignored -- a couple of
    /// pixels either side of a boundary cannot change which column a pointer is
    /// obviously over.
    #[must_use]
    pub fn visible_at(&self, fraction: f32) -> usize {
        let visible = self.visible();
        let last = visible.len().saturating_sub(1);
        let total = self.total_weight();
        if total == 0 {
            return 0;
        }

        let x = fraction.clamp(0.0, 1.0) * total as f32;
        let mut edge = 0.0_f32;
        for (index, setting) in visible.iter().enumerate() {
            edge += f32::from(setting.weight);
            if x < edge {
                return index;
            }
        }
        last
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
    fn the_playing_row_is_marked_where_its_number_would_be() {
        let track = playing_track();
        assert_eq!(
            Column::TrackNumber.value_for(&track, true),
            Column::PLAYING_MARKER,
            "the playing row needs a mark that does not depend on its background"
        );
        assert_eq!(Column::TrackNumber.value_for(&track, false), "7");
    }

    #[test]
    fn the_marker_replaces_nothing_else() {
        // Overwriting a title or an album with a glyph would lose information
        // the row exists to show.
        let track = playing_track();
        for column in Column::ALL {
            if column == Column::TrackNumber {
                continue;
            }
            assert_eq!(
                column.value_for(&track, true),
                column.value(&track),
                "{} changed when the row started playing",
                column.name()
            );
        }
    }

    fn playing_track() -> Track {
        Track {
            id: scoria_core::TrackId(1),
            album_id: scoria_core::AlbumId(1),
            path: scoria_core::RelativePath::new("a/b.flac").unwrap(),
            title: "Title".to_owned(),
            artist: "Artist".to_owned(),
            album_artist: "Album Artist".to_owned(),
            album_artist_sort: None,
            album: "Album".to_owned(),
            date: scoria_core::Date::parse("2020"),
            disc: Some(1),
            track_number: Some(7),
            duration: std::time::Duration::from_secs(100),
            replay_gain: scoria_core::ReplayGain::default(),
            added_at: scoria_core::Timestamp(0),
            genre: None,
            composer: None,
            audio: scoria_core::model::AudioProperties::default(),
            available: true,
        }
    }

    #[test]
    fn the_optional_columns_render_what_they_claim_to() {
        let mut track = playing_track();
        track.genre = Some("Grunge; Alternative Metal".to_owned());
        track.composer = Some("Jerry Cantrell".to_owned());
        track.date = scoria_core::Date::parse("1992-09-29");
        track.added_at = scoria_core::Timestamp(1_700_000_000);
        track.replay_gain.track_gain_db = Some(-6.42);
        track.audio = scoria_core::model::AudioProperties {
            bitrate_kbps: Some(1034),
            sample_rate_hz: Some(44_100),
            bit_depth: Some(16),
        };

        assert_eq!(Column::Disc.value(&track), "1");
        assert_eq!(Column::Released.value(&track), "1992-09-29");
        assert_eq!(Column::Year.value(&track), "1992");
        assert_eq!(Column::Added.value(&track), "2023-11-14");
        assert_eq!(Column::Genre.value(&track), "Grunge; Alternative Metal");
        assert_eq!(Column::Composer.value(&track), "Jerry Cantrell");
        // Signed, because the sign is the whole information: this track plays
        // quieter than reference, and a bare "6.4" would read as louder.
        assert_eq!(Column::Gain.value(&track), "-6.4 dB");
        assert_eq!(Column::Format.value(&track), "FLAC");
        assert_eq!(Column::Bitrate.value(&track), "1034 kbps");
        assert_eq!(Column::SampleRate.value(&track), "44.1 kHz");
        assert_eq!(Column::BitDepth.value(&track), "16-bit");
        assert_eq!(Column::File.value(&track), "a/b.flac");
    }

    #[test]
    fn a_boost_reads_as_a_boost() {
        let mut track = playing_track();
        track.replay_gain.track_gain_db = Some(2.5);
        assert_eq!(Column::Gain.value(&track), "+2.5 dB");
    }

    #[test]
    fn absent_metadata_is_blank_rather_than_invented() {
        // An untagged track plays at 0 dB (§6.7), but printing "0.0 dB" here
        // would make an absent tag look like a measured one -- and the whole
        // point of the column is seeing which files lack it. The same applies
        // to every other optional column.
        let track = playing_track();
        for column in [
            Column::Genre,
            Column::Composer,
            Column::Gain,
            Column::Bitrate,
            Column::SampleRate,
            Column::BitDepth,
        ] {
            assert_eq!(
                column.value(&track),
                "",
                "{} invented a value",
                column.name()
            );
        }
    }

    #[test]
    fn the_default_shows_the_default_set_and_offers_the_rest() {
        // §7.3's default set is what opens; the optional columns are present in
        // the panel and off. A new column arriving visible would rearrange a
        // table someone had already arranged.
        let layout = ColumnLayout::default();
        assert_eq!(layout.all().len(), Column::ALL.len());
        assert!(
            layout.visible().len() < Column::ALL.len(),
            "every column is on by default, so the optional ones are not optional"
        );
        for setting in layout.all() {
            assert_eq!(
                setting.visible,
                setting.column.visible_by_default(),
                "{:?} does not open as it says it does",
                setting.column
            );
        }
    }

    #[test]
    fn a_settings_file_that_predates_a_column_does_not_gain_it_visibly() {
        // The upgrade path: `repaired` appends columns the file has never heard
        // of, and appending them visible would change the table under someone
        // who had arranged it.
        let full = ColumnLayout::default();
        let trimmed: Vec<_> = full
            .all()
            .iter()
            .copied()
            .filter(|c| c.column != Column::Genre)
            .collect();

        // Through serde, because that is how a settings file actually arrives.
        let text = serde_json::to_string(&trimmed).unwrap();
        let stale: ColumnLayout = serde_json::from_str(&text).unwrap();
        let repaired = stale.repaired();
        let genre = repaired
            .all()
            .iter()
            .find(|c| c.column == Column::Genre)
            .unwrap();
        assert!(!genre.visible, "the restored column arrived switched on");
    }

    #[test]
    fn a_drag_lands_the_column_where_it_was_dropped() {
        let mut layout = ColumnLayout::default();
        let before: Vec<_> = layout.visible().iter().map(|c| c.column).collect();

        // Carry the title (position 1) to where the album is (position 4).
        layout.move_visible(1, 4);
        let after: Vec<_> = layout.visible().iter().map(|c| c.column).collect();

        assert_eq!(after.get(4), Some(&Column::Title), "{after:?}");
        assert_eq!(after.len(), before.len(), "a column was lost or gained");
        for column in &before {
            assert!(after.contains(column), "{column:?} disappeared");
        }
    }

    #[test]
    fn a_drag_leftwards_lands_on_the_target_rather_than_past_it() {
        // The two directions need different insertion points, because removing
        // the column first shifts everything after it down one. Getting this
        // wrong is off-by-one in one direction only, which is exactly the kind
        // that survives a casual test.
        let mut layout = ColumnLayout::default();
        layout.move_visible(4, 1);
        let after: Vec<_> = layout.visible().iter().map(|c| c.column).collect();
        assert_eq!(after.get(1), Some(&Column::Album), "{after:?}");
    }

    #[test]
    fn dragging_a_column_onto_itself_changes_nothing() {
        // This is the click-to-sort case: pressed and released without moving.
        let mut layout = ColumnLayout::default();
        let before = layout.visible();
        layout.move_visible(2, 2);
        assert_eq!(layout.visible(), before);
    }

    #[test]
    fn a_drag_past_the_ends_is_ignored_rather_than_wrapping() {
        let mut layout = ColumnLayout::default();
        let before = layout.visible();
        layout.move_visible(0, 99);
        layout.move_visible(99, 0);
        assert_eq!(layout.visible(), before);
    }

    #[test]
    fn hidden_columns_keep_their_place_when_a_visible_one_moves() {
        // Positions in a drag are positions among the *visible* columns, but
        // the stored order holds every column. A hidden one must not be
        // dragged along with its neighbour, or turning it back on would put it
        // somewhere nobody moved it to.
        let mut layout = ColumnLayout::default();
        layout.set_visible(Column::Artist, false);
        let hidden_before = layout.all().iter().position(|c| c.column == Column::Artist);

        layout.move_visible(0, 3);

        let hidden_after = layout.all().iter().position(|c| c.column == Column::Artist);
        assert!(hidden_before.is_some() && hidden_after.is_some());
        assert!(
            !layout.visible().iter().any(|c| c.column == Column::Artist),
            "a hidden column became visible"
        );
    }

    #[test]
    fn the_pointer_picks_the_column_it_is_over() {
        // The header is laid out by weight, so a fraction of the width is a
        // position in the weights -- the same geometry `drag_edge` works in.
        let layout = ColumnLayout::default();
        let visible = layout.visible();
        let total = f32::from(u16::try_from(layout.total_weight()).unwrap_or(u16::MAX));

        let mut edge = 0.0_f32;
        for (index, setting) in visible.iter().enumerate() {
            let middle = (edge + f32::from(setting.weight) / 2.0) / total;
            assert_eq!(
                layout.visible_at(middle),
                index,
                "the middle of {} picked the wrong column",
                setting.column.name()
            );
            edge += f32::from(setting.weight);
        }
    }

    #[test]
    fn a_pointer_beyond_the_table_picks_an_end_rather_than_nothing() {
        // The catcher sheet follows the pointer off the table, so these arrive.
        let layout = ColumnLayout::default();
        let last = layout.visible().len() - 1;
        assert_eq!(layout.visible_at(-5.0), 0);
        assert_eq!(layout.visible_at(0.0), 0);
        assert_eq!(layout.visible_at(1.0), last);
        assert_eq!(layout.visible_at(5.0), last);
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
