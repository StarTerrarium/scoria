//! Fixture builders for unit tests.
//!
//! Kept in one place so tests describe only the field under test and the
//! defaults stay consistent across modules.

#![allow(clippy::unwrap_used)]
// A fixture builder covers every field, including ones no test needs yet.
#![allow(dead_code)]

use crate::date::Date;
use crate::id::{AlbumId, TrackId};
use crate::model::{ReplayGain, Timestamp, Track};
use crate::path::RelativePath;
use std::time::Duration;

/// Builds a `Track` with plausible defaults, overriding only what a test cares
/// about.
#[derive(Debug, Clone)]
pub struct TrackBuilder {
    track: Track,
}

impl TrackBuilder {
    pub fn new(id: u64) -> Self {
        Self {
            track: Track {
                id: TrackId(id),
                album_id: AlbumId(1),
                path: RelativePath::new(format!("artist/album/{id:02}.flac")).unwrap(),
                title: format!("Track {id}"),
                artist: "Artist".to_owned(),
                album_artist: "Artist".to_owned(),
                album_artist_sort: None,
                album: "Album".to_owned(),
                date: Some(Date::from_year(2020)),
                disc: Some(1),
                track_number: Some(u16::try_from(id).unwrap_or(u16::MAX)),
                duration: Duration::from_secs(200),
                replay_gain: ReplayGain::default(),
                added_at: Timestamp(1_700_000_000),
                genre: None,
                composer: None,
                audio: crate::model::AudioProperties::default(),
                available: true,
            },
        }
    }

    pub fn album_artist(mut self, name: &str) -> Self {
        self.track.album_artist = name.to_owned();
        self
    }

    pub fn album_artist_sort(mut self, name: &str) -> Self {
        self.track.album_artist_sort = Some(name.to_owned());
        self
    }

    pub fn artist(mut self, name: &str) -> Self {
        self.track.artist = name.to_owned();
        self
    }

    pub fn album(mut self, title: &str) -> Self {
        self.track.album = title.to_owned();
        self
    }

    pub fn album_id(mut self, id: u64) -> Self {
        self.track.album_id = AlbumId(id);
        self
    }

    pub fn title(mut self, title: &str) -> Self {
        self.track.title = title.to_owned();
        self
    }

    /// Takes the tag form, so tests read like the files they stand in for.
    pub fn date(mut self, date: &str) -> Self {
        self.track.date = Date::parse(date);
        self
    }

    pub fn no_date(mut self) -> Self {
        self.track.date = None;
        self
    }

    pub fn disc(mut self, disc: u16) -> Self {
        self.track.disc = Some(disc);
        self
    }

    pub fn track_number(mut self, n: u16) -> Self {
        self.track.track_number = Some(n);
        self
    }

    pub fn no_track_number(mut self) -> Self {
        self.track.track_number = None;
        self
    }

    pub fn duration(mut self, secs: u64) -> Self {
        self.track.duration = Duration::from_secs(secs);
        self
    }

    pub fn added_at(mut self, secs: i64) -> Self {
        self.track.added_at = Timestamp(secs);
        self
    }

    pub fn unavailable(mut self) -> Self {
        self.track.available = false;
        self
    }

    pub fn replay_gain(mut self, gain: ReplayGain) -> Self {
        self.track.replay_gain = gain;
        self
    }

    pub fn build(self) -> Track {
        self.track
    }
}

/// Shorthand for a track that only needs an id.
pub fn track(id: u64) -> Track {
    TrackBuilder::new(id).build()
}

/// The ids of a slice of tracks, for asserting on order.
pub fn ids(tracks: &[Track]) -> Vec<u64> {
    tracks.iter().map(|t| t.id.get()).collect()
}
