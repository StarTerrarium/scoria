//! Choosing what to analyse.

use super::write::supported_extension;
use crate::id::AlbumId;
use crate::model::Track;
use crate::query::index::LibraryIndex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One album to analyse, as a whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumJob {
    pub album_artist: String,
    pub title: String,
    /// Absolute paths, resolved against the root when planned.
    pub tracks: Vec<PathBuf>,
}

/// What an analysis would do, and what it would leave alone and why.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GainPlan {
    pub albums: Vec<AlbumJob>,
    pub skipped: Skipped,
    /// Tracks carrying both track and album gain already.
    pub tracks_levelled: usize,
    pub tracks_total: usize,
}

/// Albums left out, counted by reason.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    /// Some of the album's files are missing (§7.4). Album gain over part of
    /// an album would be wrong for the whole of it.
    pub missing_files: usize,
    /// Some of the album's files are in a format with no writer yet.
    pub unsupported: usize,
}

impl GainPlan {
    /// How many files the plan would write.
    #[must_use]
    pub fn files(&self) -> usize {
        self.albums.iter().map(|a| a.tracks.len()).sum()
    }
}

/// Whether a track already carries what an analysis writes.
fn levelled(track: &Track) -> bool {
    track.replay_gain.track_gain_db.is_some() && track.replay_gain.album_gain_db.is_some()
}

/// Plans an analysis of the library at `root`.
///
/// An album is analysed when any of its tracks lacks track or album gain --
/// all of it, because album gain depends on every track. With `all`, every
/// album is, which is how a library tagged by something else is brought into
/// line. Albums with missing files or unsupported formats are counted, not
/// analysed.
///
/// "Album" is Scoria's album identity (§4.2), not a folder: a set split into
/// one folder per disc is levelled as the one album it is.
#[must_use]
pub fn plan(index: &LibraryIndex, root: &Path, all: bool) -> GainPlan {
    let mut albums: BTreeMap<AlbumId, Vec<&Track>> = BTreeMap::new();
    for track in index.tracks() {
        albums.entry(track.album_id).or_default().push(track);
    }

    let mut plan = GainPlan {
        tracks_total: index.len(),
        tracks_levelled: index.tracks().iter().filter(|t| levelled(t)).count(),
        ..GainPlan::default()
    };

    for tracks in albums.values() {
        let Some(first) = tracks.first() else {
            continue;
        };
        if !all && tracks.iter().all(|t| levelled(t)) {
            continue;
        }
        if tracks.iter().any(|t| !t.available) {
            plan.skipped.missing_files += 1;
            continue;
        }
        let paths: Vec<PathBuf> = tracks.iter().map(|t| t.path.to_absolute(root)).collect();
        if !paths.iter().all(|p| supported_extension(p)) {
            plan.skipped.unsupported += 1;
            continue;
        }
        plan.albums.push(AlbumJob {
            album_artist: first.album_artist.clone(),
            title: first.album.clone(),
            tracks: paths,
        });
    }

    // In the order the library lists them, so progress reads as a walk
    // through it rather than a scatter.
    plan.albums.sort_by(|a, b| {
        a.album_artist
            .to_lowercase()
            .cmp(&b.album_artist.to_lowercase())
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    plan
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;
    use crate::model::ReplayGain;
    use crate::test_support::TrackBuilder;

    fn tagged() -> ReplayGain {
        ReplayGain {
            track_gain_db: Some(-8.0),
            track_peak: Some(0.9),
            album_gain_db: Some(-7.5),
            album_peak: Some(0.95),
        }
    }

    fn track(id: u64, album: u64, path: &str) -> TrackBuilder {
        TrackBuilder::new(id).album_id(album).path(path)
    }

    fn index(tracks: Vec<Track>) -> LibraryIndex {
        LibraryIndex::from_parts(tracks, Vec::new())
    }

    #[test]
    fn an_album_with_one_gap_is_analysed_whole() {
        let lib = index(vec![
            track(1, 1, "a/1.flac").replay_gain(tagged()).build(),
            track(2, 1, "a/2.flac").build(),
            track(3, 2, "b/1.flac").replay_gain(tagged()).build(),
        ]);
        let plan = plan(&lib, Path::new("/home/someone/Music"), false);
        assert_eq!(plan.albums.len(), 1);
        assert_eq!(plan.albums[0].tracks.len(), 2, "the tagged track too");
        assert_eq!(plan.tracks_levelled, 2);
        assert_eq!(plan.tracks_total, 3);
        assert_eq!(plan.files(), 2);
    }

    #[test]
    fn all_takes_albums_that_are_already_levelled() {
        let lib = index(vec![track(1, 1, "a/1.flac").replay_gain(tagged()).build()]);
        assert!(plan(&lib, Path::new("/r"), false).albums.is_empty());
        assert_eq!(plan(&lib, Path::new("/r"), true).albums.len(), 1);
    }

    #[test]
    fn an_album_with_a_missing_file_is_counted_not_analysed() {
        let lib = index(vec![
            track(1, 1, "a/1.flac").build(),
            track(2, 1, "a/2.flac").unavailable().build(),
        ]);
        let plan = plan(&lib, Path::new("/r"), false);
        assert!(plan.albums.is_empty());
        assert_eq!(plan.skipped.missing_files, 1);
    }

    #[test]
    fn an_album_in_a_format_without_a_writer_is_counted_not_analysed() {
        let lib = index(vec![track(1, 1, "a/1.mp3").build()]);
        let plan = plan(&lib, Path::new("/r"), false);
        assert!(plan.albums.is_empty());
        assert_eq!(plan.skipped.unsupported, 1);
    }

    #[test]
    fn paths_are_resolved_against_the_root() {
        let lib = index(vec![track(1, 1, "a/1.flac").build()]);
        let plan = plan(&lib, Path::new("/home/someone/Music"), false);
        assert_eq!(
            plan.albums[0].tracks,
            [PathBuf::from("/home/someone/Music/a/1.flac")]
        );
    }
}
