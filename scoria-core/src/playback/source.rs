//! The playback snapshot.
//!
//! `DESIGN.md` §6.2, and the most consequential type in the application.
//!
//! `tracks` is materialised, ordered and immutable once set. It is what plays.
//! `origin` is a label for the transport bar and the queue header and drives
//! nothing -- §6.5 makes that true by construction, since jump-to-playing
//! restores an unfiltered library view rather than anything `origin` describes.
//!
//! Shuffle is a permutation held alongside `tracks`, never a reordered query.
//! Re-running a `Selection` to decide what plays next is the one thing this
//! type exists to prevent: a re-evaluated query has no stable index, so a newly
//! imported album could silently change what "next" means mid-album.

use crate::id::{AlbumId, TrackId};
use crate::playback::rng::Rng;
use serde::{Deserialize, Serialize};

/// Where a snapshot came from. Display metadata only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    Library,
    /// Human-readable label, e.g. `Electric Callboy`.
    Filtered(String),
    Album(AlbumId),
    /// Arrives with playlists, post-1.0.
    Manual,
}

impl Origin {
    /// What the transport bar and queue header show.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Library => "Library".to_owned(),
            Self::Filtered(label) => label.clone(),
            Self::Album(id) => format!("Album {id}"),
            Self::Manual => "Queue".to_owned(),
        }
    }
}

/// §6.6. `All` wraps the same list; it never extends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

/// A materialised, ordered snapshot of what is playing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackSource {
    /// Immutable once set, always in natural (snapshot) order.
    tracks: Vec<TrackId>,
    /// Play order as indices into `tracks`. `None` means natural order.
    ///
    /// Holding the permutation separately is what lets the queue overlay show
    /// the snapshot in its own order while playback follows another, and lets
    /// shuffle be switched off without re-deriving anything.
    order: Option<Vec<usize>>,
    /// Cursor into the play order, not into `tracks`.
    cursor: usize,
    /// Play-order cursors already visited, newest last. `Previous` walks this
    /// rather than stepping back through the permutation, because in shuffle
    /// those are different things and only one matches what was just heard.
    history: Vec<usize>,
    origin: Origin,
    repeat: RepeatMode,
}

impl PlaybackSource {
    /// A snapshot positioned at the start.
    #[must_use]
    pub fn new(tracks: Vec<TrackId>, origin: Origin) -> Self {
        Self {
            tracks,
            order: None,
            cursor: 0,
            history: Vec::new(),
            origin,
            repeat: RepeatMode::Off,
        }
    }

    /// A snapshot positioned at `at`, or at the start if it is not present.
    ///
    /// This is the shape `Command::PlayFrom` needs: the user clicked a track in
    /// a view, and the view is what got snapshotted.
    #[must_use]
    pub fn starting_at(tracks: Vec<TrackId>, origin: Origin, at: TrackId) -> Self {
        let cursor = tracks.iter().position(|t| *t == at).unwrap_or(0);
        Self {
            tracks,
            order: None,
            cursor,
            history: Vec::new(),
            origin,
            repeat: RepeatMode::Off,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// The snapshot in its natural order. What the queue overlay lists.
    #[must_use]
    pub fn tracks(&self) -> &[TrackId] {
        &self.tracks
    }

    #[must_use]
    pub fn origin(&self) -> &Origin {
        &self.origin
    }

    #[must_use]
    pub const fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    pub const fn set_repeat(&mut self, repeat: RepeatMode) {
        self.repeat = repeat;
    }

    #[must_use]
    pub const fn is_shuffled(&self) -> bool {
        self.order.is_some()
    }

    /// Index into `tracks` of the current track. What the queue overlay marks
    /// and what `Event::TrackChanged` carries.
    #[must_use]
    pub fn position(&self) -> Option<usize> {
        self.track_index(self.cursor)
    }

    /// The track playing now.
    #[must_use]
    pub fn current(&self) -> Option<TrackId> {
        self.tracks.get(self.position()?).copied()
    }

    /// Maps a play-order cursor to an index into `tracks`.
    ///
    /// Every read goes through here, which is why a snapshot deserialised with
    /// a corrupt permutation degrades to `None` rather than panicking.
    fn track_index(&self, cursor: usize) -> Option<usize> {
        match &self.order {
            Some(order) => order
                .get(cursor)
                .copied()
                .filter(|i| *i < self.tracks.len()),
            None => (cursor < self.tracks.len()).then_some(cursor),
        }
    }

    /// Moves to the track at `position` in the natural order.
    ///
    /// This is what double-clicking a row in the queue overlay does (§9.2).
    /// Returns false if the position is outside the snapshot.
    pub fn set_position(&mut self, position: usize) -> bool {
        if position >= self.tracks.len() {
            return false;
        }
        let cursor = match &self.order {
            Some(order) => match order.iter().position(|i| *i == position) {
                Some(cursor) => cursor,
                None => return false,
            },
            None => position,
        };
        self.history.push(self.cursor);
        self.cursor = cursor;
        true
    }

    /// Advances, honouring repeat. `None` means the snapshot is exhausted and
    /// playback stops -- §6.4, and the reason a snapshot is a snapshot.
    ///
    /// `RepeatMode::One` is deliberately not handled here: repeating one track
    /// is the engine replaying the same source rather than the snapshot
    /// advancing, and conflating them would make `Next` a no-op under repeat-one
    /// when the user plainly means "skip".
    pub fn advance(&mut self) -> Option<TrackId> {
        if self.tracks.is_empty() {
            return None;
        }

        let next = self.cursor.checked_add(1)?;
        let next = if next < self.tracks.len() {
            next
        } else if self.repeat == RepeatMode::All {
            0
        } else {
            return None;
        };

        self.history.push(self.cursor);
        self.cursor = next;
        self.current()
    }

    /// Steps back through the history of what was actually played, falling back
    /// to the previous position in the play order once history runs out.
    pub fn previous(&mut self) -> Option<TrackId> {
        if self.tracks.is_empty() {
            return None;
        }

        if let Some(cursor) = self.history.pop() {
            self.cursor = cursor;
            return self.current();
        }

        self.cursor = match self.cursor.checked_sub(1) {
            Some(previous) => previous,
            None if self.repeat == RepeatMode::All => self.tracks.len().checked_sub(1)?,
            None => return None,
        };
        self.current()
    }

    /// Whether `advance` would yield a track.
    #[must_use]
    pub fn has_next(&self) -> bool {
        if self.tracks.is_empty() {
            return false;
        }
        self.repeat == RepeatMode::All
            || self
                .cursor
                .checked_add(1)
                .is_some_and(|n| n < self.tracks.len())
    }

    /// The track that would play next, for the gapless preload (§3.4).
    ///
    /// Does not move the cursor: `preload_next` needs to know what to decode
    /// well before the current track's tail drains.
    #[must_use]
    pub fn peek_next(&self) -> Option<TrackId> {
        let next = self.cursor.checked_add(1)?;
        let next = if next < self.tracks.len() {
            next
        } else if self.repeat == RepeatMode::All {
            0
        } else {
            return None;
        };
        self.tracks.get(self.track_index(next)?).copied()
    }

    /// Turns shuffle on, generating a fresh permutation.
    ///
    /// The current track stays current: it is moved to the front of the new
    /// order rather than being displaced, because shuffling should change what
    /// comes *next*, not interrupt what is playing.
    ///
    /// History is cleared, since the previous order's cursors no longer refer
    /// to the same tracks.
    pub fn shuffle(&mut self, rng: &mut Rng) {
        let Some(current) = self.position() else {
            self.order = Some(Vec::new());
            return;
        };

        let mut order: Vec<usize> = (0..self.tracks.len()).filter(|i| *i != current).collect();
        rng.shuffle(&mut order);
        order.insert(0, current);

        self.order = Some(order);
        self.cursor = 0;
        self.history.clear();
    }

    /// Turns shuffle off, returning to natural order with the current track
    /// still current.
    pub fn unshuffle(&mut self) {
        if let Some(position) = self.position() {
            self.cursor = position;
        }
        self.order = None;
        self.history.clear();
    }

    /// Turning shuffle off and then on again reshuffles rather than restoring
    /// the previous permutation.
    ///
    /// Resolves the question §14 left open. Restoring would mean the toggle
    /// sometimes reshuffles and sometimes does not, depending on history the
    /// user cannot see; always reshuffling is the behaviour that can be
    /// predicted from the button alone.
    pub fn set_shuffled(&mut self, shuffled: bool, rng: &mut Rng) {
        if shuffled {
            self.shuffle(rng);
        } else {
            self.unshuffle();
        }
    }

    /// The play order as indices into `tracks`, for the queue overlay to show
    /// what actually comes next.
    #[must_use]
    pub fn play_order(&self) -> Vec<usize> {
        self.order
            .clone()
            .unwrap_or_else(|| (0..self.tracks.len()).collect())
    }

    /// Skips forward past tracks the caller rejects, e.g. ones whose files no
    /// longer resolve (§6.4).
    ///
    /// Stops at the first acceptable track, or returns `None` having exhausted
    /// the snapshot. Bounded by the snapshot length, so an entirely unavailable
    /// snapshot terminates rather than looping forever under repeat.
    pub fn advance_to_playable(&mut self, playable: impl Fn(TrackId) -> bool) -> Option<TrackId> {
        for _ in 0..self.tracks.len() {
            let candidate = self.advance()?;
            if playable(candidate) {
                return Some(candidate);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;
    use std::collections::HashSet;

    fn ids(n: u64) -> Vec<TrackId> {
        (1..=n).map(TrackId).collect()
    }

    fn source(n: u64) -> PlaybackSource {
        PlaybackSource::new(ids(n), Origin::Library)
    }

    fn play_through(source: &mut PlaybackSource) -> Vec<u64> {
        let mut heard = source
            .current()
            .map(|t| t.get())
            .into_iter()
            .collect::<Vec<_>>();
        while let Some(track) = source.advance() {
            heard.push(track.get());
            if heard.len() > 100 {
                break; // A snapshot that never ends is the bug under test.
            }
        }
        heard
    }

    // ------------------------------------------------------------ the basics --

    #[test]
    fn a_new_snapshot_starts_at_the_first_track() {
        let s = source(3);
        assert_eq!(s.current(), Some(TrackId(1)));
        assert_eq!(s.position(), Some(0));
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn starting_at_positions_on_the_clicked_track() {
        let s = PlaybackSource::starting_at(ids(5), Origin::Library, TrackId(4));
        assert_eq!(s.current(), Some(TrackId(4)));
        assert_eq!(s.position(), Some(3));
    }

    #[test]
    fn starting_at_a_track_not_in_the_snapshot_falls_back_to_the_start() {
        let s = PlaybackSource::starting_at(ids(3), Origin::Library, TrackId(99));
        assert_eq!(s.current(), Some(TrackId(1)));
    }

    #[test]
    fn an_empty_snapshot_yields_nothing_and_does_not_panic() {
        let mut s = PlaybackSource::new(vec![], Origin::Library);
        assert!(s.is_empty());
        assert_eq!(s.current(), None);
        assert_eq!(s.position(), None);
        assert_eq!(s.advance(), None);
        assert_eq!(s.previous(), None);
        assert_eq!(s.peek_next(), None);
        assert!(!s.has_next());
    }

    // ------------------------------------------------------- §6.4 the rules --

    #[test]
    fn the_snapshot_stops_when_it_runs_out() {
        let mut s = source(3);
        assert_eq!(play_through(&mut s), vec![1, 2, 3]);
        assert!(!s.has_next());
        assert_eq!(s.advance(), None);
    }

    #[test]
    fn repeat_all_wraps_the_same_list_rather_than_extending_it() {
        let mut s = source(3);
        s.set_repeat(RepeatMode::All);

        let mut heard = vec![s.current().unwrap().get()];
        for _ in 0..5 {
            heard.push(s.advance().unwrap().get());
        }
        assert_eq!(heard, vec![1, 2, 3, 1, 2, 3]);

        // Wrapping, not growing: the snapshot is still three tracks.
        assert_eq!(s.len(), 3);
        assert_eq!(s.tracks(), &ids(3)[..]);
    }

    #[test]
    fn repeat_one_does_not_make_next_a_no_op() {
        // Repeat-one is the engine replaying the same source; the user pressing
        // Next still plainly means "skip".
        let mut s = source(3);
        s.set_repeat(RepeatMode::One);
        assert_eq!(s.advance(), Some(TrackId(2)));
    }

    #[test]
    fn unavailable_tracks_are_skipped_on_advance() {
        let mut s = source(5);
        let missing: HashSet<u64> = [2, 3].into_iter().collect();
        let next = s.advance_to_playable(|t| !missing.contains(&t.get()));
        assert_eq!(next, Some(TrackId(4)));
    }

    #[test]
    fn a_wholly_unavailable_snapshot_terminates_rather_than_looping() {
        let mut s = source(4);
        s.set_repeat(RepeatMode::All);
        assert_eq!(s.advance_to_playable(|_| false), None);
    }

    // ---------------------------------------------------------- §9.2 queue --

    #[test]
    fn set_position_moves_to_a_row_in_the_natural_order() {
        let mut s = source(5);
        assert!(s.set_position(3));
        assert_eq!(s.current(), Some(TrackId(4)));
    }

    #[test]
    fn set_position_rejects_a_row_outside_the_snapshot() {
        let mut s = source(3);
        assert!(!s.set_position(99));
        assert_eq!(s.current(), Some(TrackId(1)));
    }

    #[test]
    fn peek_next_does_not_move_the_cursor() {
        // The gapless preload needs this well before the tail drains (§3.4).
        let s = source(3);
        assert_eq!(s.peek_next(), Some(TrackId(2)));
        assert_eq!(s.peek_next(), Some(TrackId(2)));
        assert_eq!(s.current(), Some(TrackId(1)));
    }

    #[test]
    fn peek_next_wraps_under_repeat_all_and_stops_otherwise() {
        let mut s = source(2);
        s.set_position(1);
        assert_eq!(s.peek_next(), None);
        s.set_repeat(RepeatMode::All);
        assert_eq!(s.peek_next(), Some(TrackId(1)));
    }

    // -------------------------------------------------------- previous/§6.6 --

    #[test]
    fn previous_walks_back_through_what_was_actually_heard() {
        let mut s = source(5);
        s.advance();
        s.advance();
        assert_eq!(s.current(), Some(TrackId(3)));

        assert_eq!(s.previous(), Some(TrackId(2)));
        assert_eq!(s.previous(), Some(TrackId(1)));
    }

    #[test]
    fn previous_at_the_start_stops_unless_repeating() {
        let mut s = source(3);
        assert_eq!(s.previous(), None);

        s.set_repeat(RepeatMode::All);
        assert_eq!(s.previous(), Some(TrackId(3)));
    }

    // ------------------------------------------------------- §6.6 shuffle --

    #[test]
    fn shuffle_is_a_true_permutation_of_the_snapshot() {
        let mut s = source(50);
        s.shuffle(&mut Rng::seeded(1));

        let heard = play_through(&mut s);
        assert_eq!(heard.len(), 50, "shuffle changed how many tracks play");
        assert_eq!(
            heard.iter().copied().collect::<HashSet<_>>(),
            (1..=50).collect::<HashSet<_>>(),
            "shuffle lost, duplicated or invented a track"
        );
    }

    #[test]
    fn shuffle_leaves_the_underlying_snapshot_untouched() {
        // §6.2: `tracks` is immutable once set. Shuffle is a permutation held
        // alongside it, which is what lets the queue show either order.
        let mut s = source(20);
        let before = s.tracks().to_vec();
        s.shuffle(&mut Rng::seeded(3));
        assert_eq!(s.tracks(), &before[..]);
    }

    #[test]
    fn shuffle_does_not_interrupt_what_is_playing() {
        let mut s = source(20);
        s.set_position(7);
        let playing = s.current();

        s.shuffle(&mut Rng::seeded(11));
        assert_eq!(s.current(), playing, "shuffling changed the current track");
    }

    #[test]
    fn shuffle_actually_reorders() {
        let mut s = source(50);
        s.shuffle(&mut Rng::seeded(2));
        assert_ne!(play_through(&mut s), (1..=50).collect::<Vec<_>>());
    }

    #[test]
    fn unshuffle_returns_to_natural_order_with_the_same_track_playing() {
        let mut s = source(20);
        s.shuffle(&mut Rng::seeded(4));
        s.advance();
        s.advance();
        let playing = s.current().unwrap();

        s.unshuffle();
        assert!(!s.is_shuffled());
        assert_eq!(s.current(), Some(playing));
        assert_eq!(s.position(), Some((playing.get() - 1) as usize));
    }

    #[test]
    fn toggling_shuffle_off_and_on_reshuffles() {
        // Resolves the question §14 left open: always reshuffle, so the button
        // can be predicted from itself rather than from invisible history.
        let mut s = source(50);
        let mut rng = Rng::seeded(9);

        s.set_shuffled(true, &mut rng);
        let first = s.play_order();

        s.set_shuffled(false, &mut rng);
        s.set_shuffled(true, &mut rng);
        let second = s.play_order();

        assert_ne!(first, second);
        // Still a permutation of the same snapshot, though.
        assert_eq!(
            second.iter().copied().collect::<HashSet<_>>(),
            (0..50).collect()
        );
    }

    #[test]
    fn repeat_all_wraps_within_the_shuffled_order() {
        let mut s = source(4);
        s.shuffle(&mut Rng::seeded(6));
        s.set_repeat(RepeatMode::All);

        let order: Vec<u64> = s
            .play_order()
            .iter()
            .map(|i| s.tracks()[*i].get())
            .collect();
        let mut heard = vec![s.current().unwrap().get()];
        for _ in 0..7 {
            heard.push(s.advance().unwrap().get());
        }

        let expected: Vec<u64> = order.iter().cycle().take(8).copied().collect();
        assert_eq!(heard, expected);
    }

    #[test]
    fn shuffling_an_empty_snapshot_does_not_panic() {
        let mut s = PlaybackSource::new(vec![], Origin::Library);
        s.shuffle(&mut Rng::seeded(1));
        assert_eq!(s.current(), None);
        assert_eq!(s.advance(), None);
    }

    // ------------------------------------------------- §6.2 origin is a label --

    #[test]
    fn origin_does_not_affect_what_plays() {
        // The whole point of invariant 5. Same tracks, different labels, and
        // playback must be indistinguishable.
        let mut library = PlaybackSource::new(ids(4), Origin::Library);
        let mut filtered =
            PlaybackSource::new(ids(4), Origin::Filtered("Electric Callboy".to_owned()));
        let mut album = PlaybackSource::new(ids(4), Origin::Album(AlbumId(7)));

        // play_through exhausts the source, so capture the reference once.
        let expected = play_through(&mut library);
        assert_eq!(play_through(&mut filtered), expected);
        assert_eq!(play_through(&mut album), expected);
    }

    #[test]
    fn origin_labels_are_human_readable() {
        assert_eq!(Origin::Library.label(), "Library");
        assert_eq!(Origin::Filtered("花冷え。".to_owned()).label(), "花冷え。");
    }

    // -------------------------------------------------------------- robustness --

    #[test]
    fn a_snapshot_survives_a_serde_round_trip() {
        // Event::QueueChanged carries this across the seam.
        let mut s = source(10);
        s.shuffle(&mut Rng::seeded(8));
        s.set_repeat(RepeatMode::All);
        s.advance();

        let json = serde_json::to_string(&s).unwrap();
        let restored: PlaybackSource = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, s);
        assert_eq!(restored.current(), s.current());
    }

    #[test]
    fn a_corrupt_permutation_degrades_instead_of_panicking() {
        // Deserialising a hand-edited or truncated snapshot must not take the
        // audio thread down.
        let json = r#"{
            "tracks": [1, 2, 3],
            "order": [99, 0, 1],
            "cursor": 0,
            "history": [],
            "origin": "Library",
            "repeat": "Off"
        }"#;
        let mut s: PlaybackSource = serde_json::from_str(json).unwrap();
        assert_eq!(
            s.current(),
            None,
            "an out-of-range index should read as absent"
        );
        assert_eq!(s.advance(), Some(TrackId(1)));
    }

    #[test]
    fn a_cursor_past_the_end_degrades_instead_of_panicking() {
        let json = r#"{
            "tracks": [1, 2],
            "order": null,
            "cursor": 99,
            "history": [],
            "origin": "Library",
            "repeat": "Off"
        }"#;
        let s: PlaybackSource = serde_json::from_str(json).unwrap();
        assert_eq!(s.current(), None);
        assert_eq!(s.position(), None);
    }
}
