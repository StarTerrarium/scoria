//! Album identity resolution.
//!
//! `DESIGN.md` §4.2. The library is assumed to be tagged with MusicBrainz
//! Picard, which makes album identity a lookup rather than a heuristic:
//!
//! 1. `MUSICBRAINZ_ALBUMID` where present.
//! 2. Fall back to `(albumartist, album, date)`.
//! 3. Fall back to the containing directory.
//!
//! Steps 2 and 3 exist for stray untagged files, not as the primary mechanism.
//! Reading the tags that feed this is the scanner's job (M3); resolving them to
//! a stable key is here, because the key is what storage is keyed on.

use crate::date::Date;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// How an album was identified, in descending order of confidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlbumIdentity {
    /// Rung 1: the release MBID Picard wrote.
    MusicBrainz(String),
    /// Rung 2: album artist, title and date together.
    Tagged {
        album_artist: String,
        album: String,
        date: Option<Date>,
    },
    /// Rung 3: the directory the files sit in. Last resort, and the only rung
    /// that breaks if the library is reorganised.
    Directory(String),
}

impl AlbumIdentity {
    /// Resolves the ladder from what the tags provided.
    ///
    /// `directory` is the track's parent directory relative to the library
    /// root, which is why it survives the library being moved between drives
    /// (§4.3) even though it does not survive reorganisation.
    #[must_use]
    pub fn resolve(
        musicbrainz_album_id: Option<&str>,
        album_artist: Option<&str>,
        album: Option<&str>,
        date: Option<Date>,
        directory: Option<&Path>,
    ) -> Self {
        if let Some(mbid) = musicbrainz_album_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Self::MusicBrainz(mbid.to_owned());
        }

        let album_artist = album_artist.map(str::trim).filter(|s| !s.is_empty());
        let album = album.map(str::trim).filter(|s| !s.is_empty());
        if let (Some(album_artist), Some(album)) = (album_artist, album) {
            return Self::Tagged {
                album_artist: album_artist.to_owned(),
                album: album.to_owned(),
                date,
            };
        }

        Self::Directory(directory.map_or_else(
            || "<unknown>".to_owned(),
            |d| d.to_string_lossy().into_owned(),
        ))
    }

    /// The stable string storage keys albums on.
    ///
    /// Prefixed by rung, so a directory literally named like an MBID cannot
    /// collide with a real one. Each field is then length-prefixed rather than
    /// merely delimited: a separator alone is forgeable, because a tag value
    /// containing the separator could produce the key of a different album.
    /// Tags are arbitrary user data and this key decides which tracks are the
    /// same record, so it is worth making that impossible rather than unlikely.
    #[must_use]
    pub fn key(&self) -> String {
        fn field(value: &str, out: &mut String) {
            out.push_str(&value.len().to_string());
            out.push(':');
            out.push_str(value);
        }

        let mut key = String::new();
        match self {
            Self::MusicBrainz(mbid) => {
                key.push_str("mb:");
                field(mbid, &mut key);
            }
            Self::Tagged {
                album_artist,
                album,
                date,
            } => {
                key.push_str("tag:");
                field(album_artist, &mut key);
                field(album, &mut key);
                field(&date.map_or_else(String::new, |d| d.to_string()), &mut key);
            }
            Self::Directory(dir) => {
                key.push_str("dir:");
                field(dir, &mut key);
            }
        }
        key
    }

    /// Which rung resolved this, for diagnostics. A library resolving mostly by
    /// directory is a library that is not tagged the way §4.2 assumes.
    #[must_use]
    pub const fn rung(&self) -> u8 {
        match self {
            Self::MusicBrainz(_) => 1,
            Self::Tagged { .. } => 2,
            Self::Directory(_) => 3,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn date(s: &str) -> Option<Date> {
        Date::parse(s)
    }

    #[test]
    fn prefers_the_musicbrainz_id() {
        let id = AlbumIdentity::resolve(
            Some("1adca70c-fdd0-4ee3-9020-3556c04fba58"),
            Some("花冷え。"),
            Some("乙女改革"),
            date("2021-01-14"),
            Some(Path::new("花冷え。/乙女改革")),
        );
        assert_eq!(id.rung(), 1);
        assert!(matches!(id, AlbumIdentity::MusicBrainz(_)));
    }

    #[test]
    fn falls_back_to_the_tag_tuple_without_an_mbid() {
        let id = AlbumIdentity::resolve(
            None,
            Some("Ghost"),
            Some("Meliora"),
            date("2015"),
            Some(Path::new("Ghost/Meliora")),
        );
        assert_eq!(id.rung(), 2);
    }

    #[test]
    fn falls_back_to_the_directory_when_barely_tagged() {
        let id = AlbumIdentity::resolve(None, None, None, None, Some(Path::new("misc/rips")));
        assert_eq!(id, AlbumIdentity::Directory("misc/rips".to_owned()));
        assert_eq!(id.rung(), 3);
    }

    #[test]
    fn treats_blank_tags_as_absent() {
        // Picard writes empty frames for fields it has no value for; an empty
        // MBID must not be mistaken for a real one.
        let id = AlbumIdentity::resolve(
            Some("   "),
            Some(""),
            Some("Meliora"),
            None,
            Some(Path::new("Ghost/Meliora")),
        );
        assert_eq!(id.rung(), 3);
    }

    #[test]
    fn trims_surrounding_whitespace() {
        let a = AlbumIdentity::resolve(None, Some(" Ghost "), Some(" Meliora "), None, None);
        let b = AlbumIdentity::resolve(None, Some("Ghost"), Some("Meliora"), None, None);
        assert_eq!(a.key(), b.key());
    }

    #[test]
    fn two_albums_with_the_same_title_by_different_artists_are_distinct() {
        let a = AlbumIdentity::resolve(None, Some("Alpha"), Some("Greatest Hits"), None, None);
        let b = AlbumIdentity::resolve(None, Some("Beta"), Some("Greatest Hits"), None, None);
        assert_ne!(a.key(), b.key());
    }

    #[test]
    fn reissues_of_the_same_album_in_different_years_are_distinct() {
        let a = AlbumIdentity::resolve(None, Some("Ghost"), Some("Meliora"), date("2015"), None);
        let b = AlbumIdentity::resolve(None, Some("Ghost"), Some("Meliora"), date("2020"), None);
        assert_ne!(a.key(), b.key());
    }

    #[test]
    fn a_directory_cannot_forge_a_musicbrainz_identity() {
        let mb = AlbumIdentity::MusicBrainz("abc".to_owned());
        let dir = AlbumIdentity::Directory("abc".to_owned());
        assert_ne!(mb.key(), dir.key());
    }

    #[test]
    fn a_title_cannot_forge_a_different_tagged_identity() {
        // A plain delimiter would make these two collide, whatever character it
        // is: the tag can always contain it. Length prefixes cannot be forged.
        for separator in ["\u{1f}", ":", "|", "\0"] {
            let a = AlbumIdentity::resolve(
                None,
                Some("A"),
                Some(&format!("B{separator}C")),
                None,
                None,
            );
            let b = AlbumIdentity::resolve(
                None,
                Some(&format!("A{separator}B")),
                Some("C"),
                None,
                None,
            );
            assert_ne!(a.key(), b.key(), "forgeable with {separator:?}");
        }
    }

    #[test]
    fn a_field_boundary_cannot_be_shifted_by_a_numeric_prefix() {
        // The length prefix is itself delimited, so a value starting with
        // digits and a colon cannot be read as a different length.
        let a = AlbumIdentity::resolve(None, Some("1:X"), Some("Y"), None, None);
        let b = AlbumIdentity::resolve(None, Some("1"), Some(":XY"), None, None);
        assert_ne!(a.key(), b.key());
    }

    #[test]
    fn the_same_inputs_always_produce_the_same_key() {
        let build =
            || AlbumIdentity::resolve(None, Some("Ghost"), Some("Meliora"), date("2015"), None);
        assert_eq!(build().key(), build().key());
    }
}
