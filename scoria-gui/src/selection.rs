//! Which rows are chosen, and the transient sort on top of the configured one.
//!
//! Both are rules rather than drawing, and both were methods on `App` reaching
//! into half a dozen of its fields -- which is why neither had a test. `App`
//! holds a session and a window's worth of state, so anything living on it can
//! only be exercised by running the application.
//!
//! They are values here instead. The rules are the same ones; what changed is
//! that they can be asserted.

use crate::keymap::Mods;
use scoria_core::query::sort::{Direction, SortChain, SortField, SortKey};
use std::collections::BTreeSet;

/// The rows chosen in the table, the cursor, and where a range started.
///
/// Deliberately independent of playback -- §6.1: browsing never disturbs what
/// is playing, and the two are allowed to disagree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowSelection {
    cursor: Option<usize>,
    rows: BTreeSet<usize>,
    /// Where a range selection started.
    ///
    /// Not "wherever the cursor has reached": extending twice has to grow one
    /// range rather than walk one, and that is only true if the far end stays
    /// put.
    anchor: Option<usize>,
}

impl RowSelection {
    #[must_use]
    pub const fn cursor(&self) -> Option<usize> {
        self.cursor
    }

    #[must_use]
    pub const fn rows(&self) -> &BTreeSet<usize> {
        &self.rows
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Applies a click.
    ///
    /// Plain replaces, shift extends from the anchor, control toggles. Out of
    /// range does nothing at all rather than clamping: a click past the end of
    /// a list that just got shorter is not a click on its last row.
    pub fn click(&mut self, index: usize, modifiers: Mods, len: usize) {
        if index >= len {
            return;
        }
        self.cursor = Some(index);

        if modifiers.shift {
            let anchor = self.anchor.unwrap_or(index);
            let (from, to) = if anchor <= index {
                (anchor, index)
            } else {
                (index, anchor)
            };
            self.rows = (from..=to).collect();
            return;
        }

        if modifiers.ctrl {
            if !self.rows.remove(&index) {
                self.rows.insert(index);
            }
            self.anchor = Some(index);
            return;
        }

        self.rows = std::iter::once(index).collect();
        self.anchor = Some(index);
    }

    /// Moves the cursor by `delta`, replacing or extending the selection.
    ///
    /// With no cursor yet, moving down starts at the top and moving up starts
    /// at the bottom, so the first press goes somewhere predictable from the
    /// key that was pressed.
    pub fn move_by(&mut self, delta: isize, extend: bool, len: usize) {
        let Some(last) = len.checked_sub(1) else {
            return;
        };
        let next = match self.cursor {
            None if delta < 0 => last,
            None => 0,
            Some(current) => current.saturating_add_signed(delta).min(last),
        };
        self.click(
            next,
            Mods {
                shift: extend,
                ctrl: false,
                alt: false,
            },
            len,
        );
    }

    /// Puts the cursor on one row and selects only it.
    pub fn focus(&mut self, index: usize, len: usize) {
        self.click(index, Mods::NONE, len);
    }

    /// The row the cursor is on, if it is on one within `len`.
    #[must_use]
    pub fn cursor_within(&self, len: usize) -> Option<usize> {
        self.cursor.filter(|index| *index < len)
    }

    /// Drops anything referring to a row that no longer exists.
    ///
    /// Row indices refer to the result they came from, so a new query makes
    /// anything past its end meaningless rather than merely stale.
    pub fn retain_within(&mut self, len: usize) {
        self.rows.retain(|index| *index < len);
        self.anchor = self.anchor.filter(|index| *index < len);
        self.cursor = self.cursor.filter(|index| *index < len);
    }

    /// Forgets the cursor and the selection, for a view that has been replaced
    /// rather than merely re-run.
    pub fn clear(&mut self) {
        self.cursor = None;
        self.rows.clear();
        self.anchor = None;
    }
}

/// A transient sort applied on top of the configured chain (§7.3).
///
/// A header click never replaces the configured sort. It sits above it, with
/// an indicator and one action to drop back -- and a third click on the same
/// heading drops back on its own, so the override is always one click from
/// being gone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SortOverride(Option<(SortField, Direction)>);

impl SortOverride {
    #[must_use]
    pub const fn get(self) -> Option<(SortField, Direction)> {
        self.0
    }

    #[must_use]
    pub const fn is_set(self) -> bool {
        self.0.is_some()
    }

    pub const fn clear(&mut self) {
        self.0 = None;
    }

    /// A click on the heading for `field`: ascending, then descending, then
    /// back to the configured sort.
    pub fn click(&mut self, field: SortField) {
        self.0 = match self.0 {
            Some((current, Direction::Ascending)) if current == field => {
                Some((field, Direction::Descending))
            }
            Some((current, Direction::Descending)) if current == field => None,
            _ => Some((field, Direction::Ascending)),
        };
    }

    /// The chain to query with: the override first, then the configured chain
    /// entire.
    ///
    /// On top of, not instead of. The configured chain still breaks ties, so
    /// an override on a coarse field -- a year, a genre -- keeps albums in
    /// their own order within it rather than scattering them.
    #[must_use]
    pub fn over(self, configured: &SortChain) -> SortChain {
        let Some((field, direction)) = self.0 else {
            return configured.clone();
        };
        let mut keys = vec![SortKey { field, direction }];
        keys.extend(configured.keys().iter().copied());
        SortChain::new(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scoria_core::query::sort::SortPreset;

    const SHIFT: Mods = Mods::SHIFT;
    const CTRL: Mods = Mods::CTRL;
    const PLAIN: Mods = Mods::NONE;

    fn chosen(selection: &RowSelection) -> Vec<usize> {
        selection.rows().iter().copied().collect()
    }

    // ------------------------------------------------------- clicking rows --

    #[test]
    fn a_plain_click_replaces_the_selection() {
        let mut selection = RowSelection::default();
        selection.click(3, PLAIN, 10);
        selection.click(7, PLAIN, 10);
        assert_eq!(chosen(&selection), vec![7]);
        assert_eq!(selection.cursor(), Some(7));
    }

    #[test]
    fn shift_extends_from_the_anchor_rather_than_from_the_cursor() {
        // The distinction the anchor exists for: extending twice grows one
        // range instead of walking one. Without it, shift-clicking 6 and then
        // 8 would leave only 6..=8 rather than 3..=8.
        let mut selection = RowSelection::default();
        selection.click(3, PLAIN, 10);
        selection.click(6, SHIFT, 10);
        assert_eq!(chosen(&selection), vec![3, 4, 5, 6]);

        selection.click(8, SHIFT, 10);
        assert_eq!(chosen(&selection), vec![3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn shift_extends_backwards_too() {
        let mut selection = RowSelection::default();
        selection.click(6, PLAIN, 10);
        selection.click(2, SHIFT, 10);
        assert_eq!(chosen(&selection), vec![2, 3, 4, 5, 6]);
        assert_eq!(selection.cursor(), Some(2), "the cursor follows the click");
    }

    #[test]
    fn shrinking_a_range_is_the_same_gesture() {
        let mut selection = RowSelection::default();
        selection.click(2, PLAIN, 10);
        selection.click(8, SHIFT, 10);
        selection.click(4, SHIFT, 10);
        assert_eq!(chosen(&selection), vec![2, 3, 4]);
    }

    #[test]
    fn control_toggles_one_row_and_leaves_the_rest() {
        let mut selection = RowSelection::default();
        selection.click(1, PLAIN, 10);
        selection.click(4, CTRL, 10);
        selection.click(6, CTRL, 10);
        assert_eq!(chosen(&selection), vec![1, 4, 6]);

        selection.click(4, CTRL, 10);
        assert_eq!(chosen(&selection), vec![1, 6], "control did not deselect");
    }

    #[test]
    fn control_moves_the_anchor_so_a_later_shift_starts_from_there() {
        let mut selection = RowSelection::default();
        selection.click(1, PLAIN, 10);
        selection.click(5, CTRL, 10);
        selection.click(7, SHIFT, 10);
        assert_eq!(chosen(&selection), vec![5, 6, 7]);
    }

    #[test]
    fn shift_with_no_anchor_selects_just_the_row() {
        let mut selection = RowSelection::default();
        selection.click(4, SHIFT, 10);
        assert_eq!(chosen(&selection), vec![4]);
    }

    #[test]
    fn a_click_past_the_end_does_nothing_rather_than_clamping() {
        // A click on a row that is no longer there is not a click on the last
        // one. The result can get shorter underneath a stale index.
        let mut selection = RowSelection::default();
        selection.click(2, PLAIN, 10);
        selection.click(99, PLAIN, 10);
        assert_eq!(chosen(&selection), vec![2]);
        assert_eq!(selection.cursor(), Some(2));
    }

    #[test]
    fn clicking_an_empty_list_does_nothing() {
        let mut selection = RowSelection::default();
        selection.click(0, PLAIN, 0);
        assert!(selection.is_empty());
        assert_eq!(selection.cursor(), None);
    }

    // ------------------------------------------------------ moving by key --

    #[test]
    fn the_first_press_starts_at_the_end_the_key_points_at() {
        let mut down = RowSelection::default();
        down.move_by(1, false, 10);
        assert_eq!(down.cursor(), Some(0));

        let mut up = RowSelection::default();
        up.move_by(-1, false, 10);
        assert_eq!(up.cursor(), Some(9));
    }

    #[test]
    fn moving_stops_at_the_ends_rather_than_wrapping() {
        let mut selection = RowSelection::default();
        selection.click(8, PLAIN, 10);
        selection.move_by(5, false, 10);
        assert_eq!(selection.cursor(), Some(9));

        selection.move_by(-50, false, 10);
        assert_eq!(selection.cursor(), Some(0), "moving up ran past the top");
    }

    #[test]
    fn extending_by_key_grows_from_the_anchor() {
        let mut selection = RowSelection::default();
        selection.click(4, PLAIN, 10);
        selection.move_by(1, true, 10);
        selection.move_by(1, true, 10);
        assert_eq!(chosen(&selection), vec![4, 5, 6]);
    }

    #[test]
    fn moving_within_an_empty_list_does_nothing() {
        let mut selection = RowSelection::default();
        selection.move_by(1, false, 0);
        assert_eq!(selection.cursor(), None);
    }

    // ------------------------------------------------- surviving a requery --

    #[test]
    fn a_shorter_result_drops_the_rows_that_are_gone() {
        let mut selection = RowSelection::default();
        selection.click(2, PLAIN, 10);
        selection.click(8, SHIFT, 10);
        selection.retain_within(5);

        assert_eq!(chosen(&selection), vec![2, 3, 4]);
        assert_eq!(selection.cursor(), None, "the cursor was past the end");
    }

    #[test]
    fn a_result_that_still_covers_the_cursor_keeps_it() {
        let mut selection = RowSelection::default();
        selection.click(2, PLAIN, 10);
        selection.retain_within(5);
        assert_eq!(selection.cursor(), Some(2));
    }

    // ------------------------------------------------------- §7.3 sorting --

    #[test]
    fn a_heading_click_cycles_ascending_descending_and_off() {
        // Always one click from being gone, which is what makes a transient
        // override safe to apply by accident.
        let mut sort = SortOverride::default();
        assert_eq!(sort.get(), None);

        sort.click(SortField::Title);
        assert_eq!(sort.get(), Some((SortField::Title, Direction::Ascending)));

        sort.click(SortField::Title);
        assert_eq!(sort.get(), Some((SortField::Title, Direction::Descending)));

        sort.click(SortField::Title);
        assert_eq!(sort.get(), None, "a third click should drop the override");
    }

    #[test]
    fn clicking_a_different_heading_starts_that_one_ascending() {
        let mut sort = SortOverride::default();
        sort.click(SortField::Title);
        sort.click(SortField::Title);
        sort.click(SortField::Duration);
        assert_eq!(
            sort.get(),
            Some((SortField::Duration, Direction::Ascending)),
            "a new heading should not inherit the old one's direction"
        );
    }

    #[test]
    fn no_override_queries_with_the_configured_chain() {
        let configured = SortPreset::ArtistChronological.chain();
        assert_eq!(SortOverride::default().over(&configured), configured);
    }

    #[test]
    fn an_override_leads_the_chain_and_keeps_the_rest_of_it() {
        // On top of, not instead of. An override on a coarse field has to
        // leave albums in their own order within it rather than scattering
        // them, and that only happens if the configured chain still breaks
        // the ties.
        let configured = SortPreset::ArtistChronological.chain();
        let mut sort = SortOverride::default();
        sort.click(SortField::Genre);

        let chain = sort.over(&configured);
        assert_eq!(
            chain.keys().first().map(|k| (k.field, k.direction)),
            Some((SortField::Genre, Direction::Ascending))
        );
        for key in configured.keys() {
            assert!(
                chain.keys().contains(key),
                "{key:?} was dropped from the configured chain"
            );
        }
    }

    #[test]
    fn an_overridden_chain_still_ends_in_the_tiebreaker() {
        // Every chain does (§4.4), and this one is built by hand.
        let mut sort = SortOverride::default();
        sort.click(SortField::Duration);
        let chain = sort.over(&SortPreset::RecentlyAdded.chain());
        assert_eq!(
            chain.keys().last().map(|k| k.field),
            Some(SortField::TrackId)
        );
    }
}
