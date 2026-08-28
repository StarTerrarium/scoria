//! The records the library is made of.

use crate::date::Date;
use crate::id::{AlbumId, TrackId};
use crate::path::RelativePath;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Seconds since the Unix epoch.
///
/// A newtype over `i64` rather than a date-time crate: the only thing the
/// application does with it is order "recently added", so a calendar is not
/// needed and a dependency would not earn its place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Timestamp(pub i64);

impl Timestamp {
    #[must_use]
    pub const fn seconds(self) -> i64 {
        self.0
    }
}

/// Which gain a track should play at.
///
/// `DESIGN.md` §6.7: derived from how playback is ordered, never configured.
/// Album gain preserves the loudness relationship within a record, which is the
/// entire reason album gain exists; shuffle destroys that relationship anyway,
/// so track gain is correct there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GainMode {
    Album,
    Track,
}

/// ReplayGain values as read from tags. Never computed by analysing audio.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplayGain {
    pub track_gain_db: Option<f32>,
    pub track_peak: Option<f32>,
    pub album_gain_db: Option<f32>,
    pub album_peak: Option<f32>,
}

impl ReplayGain {
    /// The gain in dB to apply, or `0.0` when the tags do not say.
    ///
    /// Album mode falls back to track gain when only track gain is present,
    /// because a track-normalised album is closer to right than no
    /// normalisation at all. It never substitutes a neighbouring track's value:
    /// §6.7 is explicit that a wrong gain is more jarring than none.
    ///
    /// A positive gain is reduced where the peak says it would clip.
    #[must_use]
    pub fn gain_db(self, mode: GainMode) -> f32 {
        let (gain, peak) = match mode {
            GainMode::Album => (
                self.album_gain_db.or(self.track_gain_db),
                self.album_peak.or(self.track_peak),
            ),
            GainMode::Track => (self.track_gain_db, self.track_peak),
        };

        let Some(gain) = gain else {
            return 0.0;
        };
        if !gain.is_finite() {
            return 0.0;
        }

        match peak {
            // Sample values are already at or above full scale, so any boost
            // clips. Headroom is whatever the peak leaves us.
            Some(peak) if peak.is_finite() && peak > 0.0 && gain > 0.0 => {
                let headroom_db = -20.0 * peak.log10();
                gain.min(headroom_db.max(0.0))
            }
            _ => gain,
        }
    }

    /// Whether the tags carry anything at all. A library where this is mostly
    /// false will have audible steps between tracks; the fix is tagging it.
    #[must_use]
    pub fn is_tagged(self) -> bool {
        self.track_gain_db.is_some() || self.album_gain_db.is_some()
    }
}

/// One track.
///
/// `album_artist` is structural and `artist` is display (`DESIGN.md` §4.1).
/// Filter panes, grouping and sorting use the former; the latter is only ever
/// rendered in a column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub album_id: AlbumId,
    pub path: RelativePath,

    pub title: String,
    /// Full credits, e.g. `Electric Callboy feat. Conquer Divide`. Display only.
    pub artist: String,
    /// Single clean value, e.g. `Electric Callboy`. Structural.
    pub album_artist: String,
    /// `albumartistsort`: `Omnific, The`, `HANABIE.`. Sorting uses this in
    /// preference to `album_artist`, which is where leading-article handling
    /// and Japanese collation come from for free (§4.4).
    pub album_artist_sort: Option<String>,
    pub album: String,

    pub date: Option<Date>,
    pub disc: Option<u16>,
    pub track_number: Option<u16>,
    pub duration: Duration,

    pub replay_gain: ReplayGain,
    pub added_at: Timestamp,

    /// False once the file stops resolving. Such tracks are shown struck
    /// through and skipped on advance, never silently dropped (§7.4).
    pub available: bool,
}

impl Track {
    /// The value sorting and grouping use for the album artist: the sort name
    /// where Picard wrote one, otherwise the display name.
    #[must_use]
    pub fn album_artist_key(&self) -> &str {
        self.album_artist_sort
            .as_deref()
            .unwrap_or(&self.album_artist)
    }
}

/// One album, as resolved by the identity ladder in §4.2.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Album {
    pub id: AlbumId,
    /// The §4.2 identity this album resolved to, as stored.
    ///
    /// Carried on the record because things outside the database key on it --
    /// the artwork cache most of all, which must survive the rowid above being
    /// reassigned by a rebuild.
    pub identity: String,
    pub title: String,
    pub album_artist: String,
    pub album_artist_sort: Option<String>,
    pub date: Option<Date>,
}

impl Album {
    #[must_use]
    pub fn album_artist_key(&self) -> &str {
        self.album_artist_sort
            .as_deref()
            .unwrap_or(&self.album_artist)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)]

    use super::*;

    fn gain(track: Option<f32>, album: Option<f32>) -> ReplayGain {
        ReplayGain {
            track_gain_db: track,
            album_gain_db: album,
            ..Default::default()
        }
    }

    #[test]
    fn untagged_tracks_play_at_unity() {
        // §6.7: never estimate, never borrow a neighbour's value.
        let none = ReplayGain::default();
        assert_eq!(none.gain_db(GainMode::Album), 0.0);
        assert_eq!(none.gain_db(GainMode::Track), 0.0);
        assert!(!none.is_tagged());
    }

    #[test]
    fn each_mode_prefers_its_own_value() {
        let g = gain(Some(-9.95), Some(-9.69));
        assert_eq!(g.gain_db(GainMode::Track), -9.95);
        assert_eq!(g.gain_db(GainMode::Album), -9.69);
        assert!(g.is_tagged());
    }

    #[test]
    fn album_mode_falls_back_to_track_gain_but_not_the_reverse() {
        let track_only = gain(Some(-6.0), None);
        assert_eq!(track_only.gain_db(GainMode::Album), -6.0);

        let album_only = gain(None, Some(-6.0));
        assert_eq!(album_only.gain_db(GainMode::Track), 0.0);
    }

    #[test]
    fn a_boost_is_limited_by_the_peak_so_it_cannot_clip() {
        // Peak is already at full scale: no headroom, so no boost at all.
        let g = ReplayGain {
            track_gain_db: Some(6.0),
            track_peak: Some(1.0),
            ..Default::default()
        };
        assert_eq!(g.gain_db(GainMode::Track), 0.0);

        // Peak at half scale leaves about 6 dB of headroom.
        let g = ReplayGain {
            track_gain_db: Some(9.0),
            track_peak: Some(0.5),
            ..Default::default()
        };
        let applied = g.gain_db(GainMode::Track);
        assert!(
            (applied - 6.02).abs() < 0.01,
            "expected ~6.02 dB, got {applied}"
        );
    }

    #[test]
    fn attenuation_is_never_limited_by_the_peak() {
        let g = ReplayGain {
            track_gain_db: Some(-9.0),
            track_peak: Some(1.0),
            ..Default::default()
        };
        assert_eq!(g.gain_db(GainMode::Track), -9.0);
    }

    #[test]
    fn nonsense_values_fall_back_to_unity_rather_than_producing_silence() {
        let g = ReplayGain {
            track_gain_db: Some(f32::NAN),
            ..Default::default()
        };
        assert_eq!(g.gain_db(GainMode::Track), 0.0);

        let g = ReplayGain {
            track_gain_db: Some(f32::NEG_INFINITY),
            ..Default::default()
        };
        assert_eq!(g.gain_db(GainMode::Track), 0.0);
    }
}
