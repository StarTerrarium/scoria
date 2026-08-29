//! Reading tags off disk.
//!
//! Uses `lofty` rather than Symphonia's metadata layer, because tag *writing*
//! is wanted later (`DESIGN.md` §9.3) and lofty covers both.
//!
//! Everything here is tolerant by design. A library is a pile of files written
//! by many different tools over many years, and a scanner that refuses a file
//! because one frame is malformed indexes less than a scanner that takes what
//! it can read. Nothing is invented, though: a field that cannot be read is
//! `None`, never a guess.

use crate::date::Date;
use crate::error::CoreError;
use crate::model::ReplayGain;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::ItemKey;
use lofty::tag::Tag;
use std::path::Path;
use std::time::Duration;

/// File extensions the scanner considers audio.
///
/// **Everything here can actually be played**, which is the rule this list is
/// kept to. `tests/formats.rs` decodes a fixture for every entry, so adding one
/// without a decoder behind it fails rather than producing a library full of
/// tracks that stop the moment they are asked to play.
///
/// That rule cost the list four entries. Symphonia 0.5 has no Opus decoder at
/// all, and none for WavPack, Monkey's Audio, Musepack or WMA (§5.2). Indexing
/// them anyway would have been a promise the player cannot keep.
///
/// `.mp4` is deliberately absent while `.m4a` and `.m4b` are here: the
/// container is the same, but a folder of `.mp4` files is a video collection,
/// and pulling those into a music library is worse than missing the rare
/// audio-only one.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "flac", "mp3", "ogg", "oga", "m4a", "m4b", "aac", "wav", "wave", "aiff", "aif",
];

/// Whether a path looks like something worth reading tags from.
#[must_use]
pub fn is_audio_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        AUDIO_EXTENSIONS
            .iter()
            .any(|known| known.eq_ignore_ascii_case(e))
    })
}

/// What one file's tags say.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub album_artist_sort: Option<String>,
    pub album: Option<String>,
    pub date: Option<Date>,
    pub disc: Option<u16>,
    pub track_number: Option<u16>,
    pub duration: Duration,
    /// `MUSICBRAINZ_ALBUMID`, the top rung of the identity ladder (§4.2).
    pub musicbrainz_album_id: Option<String>,
    pub replay_gain: ReplayGain,
    pub has_embedded_artwork: bool,
}

/// Opens a file for reading tags, by extension and then by content.
///
/// lofty decides how to parse a file from its extension alone, which is right
/// almost always and wrong in the cases that matter: `.oga` holding Vorbis
/// rather than Ogg FLAC, or anything a converter renamed without re-encoding.
/// A library that has been carried between machines for twenty years has these
/// in it, and losing a track to a wrong extension is worse than the extra read.
///
/// So the extension is tried first, because it is right almost always and costs
/// nothing, and the file's own header is consulted only when that fails.
fn open(path: &Path) -> Result<lofty::file::TaggedFile, CoreError> {
    let by_extension = lofty::probe::Probe::open(path).and_then(lofty::probe::Probe::read);
    let Err(extension_error) = by_extension else {
        return by_extension
            .map_err(|e| CoreError::CorruptData(format!("{}: {e}", path.display())));
    };

    lofty::probe::Probe::open(path)
        .map_err(|e| CoreError::CorruptData(format!("{}: {e}", path.display())))?
        .guess_file_type()
        .map_err(|e| CoreError::CorruptData(format!("{}: {e}", path.display())))?
        .read()
        // The first failure is the one worth reporting: sniffing is the
        // fallback, and its error would say only that the header made no sense
        // either.
        .map_err(|_| CoreError::CorruptData(format!("{}: {extension_error}", path.display())))
}

/// Reads tags from a file.
///
/// # Errors
///
/// [`CoreError::CorruptData`] if the file cannot be parsed as audio at all.
/// Individual unreadable fields are absent rather than fatal.
pub fn read(path: &Path) -> Result<TrackTags, CoreError> {
    let file = open(path)?;

    let duration = file.properties().duration();
    // The primary tag is the format's native one; a file with only a foreign
    // tag block still has readable metadata, so fall back rather than give up.
    let tag = file.primary_tag().or_else(|| file.first_tag());

    let Some(tag) = tag else {
        // No tags at all is normal for a stray rip, not an error.
        return Ok(TrackTags {
            duration,
            ..TrackTags::default()
        });
    };

    Ok(TrackTags {
        title: text(tag, ItemKey::TrackTitle),
        artist: text(tag, ItemKey::TrackArtist),
        album_artist: text(tag, ItemKey::AlbumArtist),
        album_artist_sort: text(tag, ItemKey::AlbumArtistSortOrder),
        album: text(tag, ItemKey::AlbumTitle),
        date: read_date(tag),
        disc: number(tag, ItemKey::DiscNumber),
        track_number: number(tag, ItemKey::TrackNumber),
        duration,
        musicbrainz_album_id: text(tag, ItemKey::MusicBrainzReleaseId),
        replay_gain: read_replay_gain(tag),
        has_embedded_artwork: !tag.pictures().is_empty(),
    })
}

/// Every tag in a file, as it is written there.
///
/// For the properties dialog (`DESIGN.md` §9.3): a complete dump, with no
/// editorial layer. That means the format's own key names rather than ours, and
/// every field including the ones nothing indexes -- the point is to show what
/// is actually in the file.
///
/// Sorted by key, because a tag block's own order is an artefact of whichever
/// tool wrote it last and carries no meaning.
///
/// # Errors
///
/// [`CoreError::CorruptData`] if the file cannot be parsed as audio.
pub fn read_all(path: &Path) -> Result<Vec<(String, String)>, CoreError> {
    let file = open(path)?;

    let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) else {
        return Ok(Vec::new());
    };

    let tag_type = tag.tag_type();
    let mut items: Vec<(String, String)> = tag
        .items()
        .map(|item| {
            let key = item
                .key()
                .map_key(tag_type)
                .map_or_else(|| format!("{:?}", item.key()), ToOwned::to_owned);
            let value = match item.value() {
                lofty::tag::ItemValue::Text(text) | lofty::tag::ItemValue::Locator(text) => {
                    text.clone()
                }
                // Binary values are things like fingerprints. Their length says
                // more than their bytes would.
                lofty::tag::ItemValue::Binary(bytes) => format!("<{} bytes>", bytes.len()),
            };
            (key, value)
        })
        .filter(|(_, value)| !value.trim().is_empty())
        .collect();

    items.sort();
    Ok(items)
}

/// A trimmed, non-empty string, or nothing.
///
/// Picard writes empty frames for fields it has no value for, so "present but
/// blank" has to mean the same as absent -- otherwise every untagged field
/// becomes an empty-string album artist that sorts as its own group.
fn text(tag: &Tag, key: ItemKey) -> Option<String> {
    tag.get_string(key)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

/// Parses `4`, `04`, or `4/12`.
///
/// The `n/total` form is ordinary in ID3 and still appears in Vorbis comments
/// written by older tools.
fn number(tag: &Tag, key: ItemKey) -> Option<u16> {
    let raw = text(tag, key)?;
    let head = raw.split('/').next().unwrap_or(&raw).trim();
    head.parse().ok()
}

fn read_date(tag: &Tag) -> Option<Date> {
    // Picard writes a full DATE; older files may only have a year. Original
    // release date is deliberately not consulted: §14 says albums sort
    // chronologically by their own date, and for a reissue those differ.
    text(tag, ItemKey::RecordingDate)
        .and_then(|d| Date::parse(&d))
        .or_else(|| text(tag, ItemKey::Year).and_then(|y| Date::parse(&y)))
}

fn read_replay_gain(tag: &Tag) -> ReplayGain {
    ReplayGain {
        track_gain_db: gain(tag, ItemKey::ReplayGainTrackGain),
        track_peak: peak(tag, ItemKey::ReplayGainTrackPeak),
        album_gain_db: gain(tag, ItemKey::ReplayGainAlbumGain),
        album_peak: peak(tag, ItemKey::ReplayGainAlbumPeak),
    }
}

/// Parses `-9.95 dB`, `-9.95dB` or `-9.95`.
///
/// The unit suffix is conventional rather than specified, so tools differ.
/// Values that are not finite are dropped: §6.7 requires 0 dB over a guess, and
/// a NaN gain would silence a track.
fn gain(tag: &Tag, key: ItemKey) -> Option<f32> {
    let raw = text(tag, key)?;
    let numeric = raw.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c.is_whitespace());
    numeric.trim().parse::<f32>().ok().filter(|v| v.is_finite())
}

/// Peaks are a bare linear amplitude. Negative or non-finite values are
/// meaningless and are dropped rather than clamped.
fn peak(tag: &Tag, key: ItemKey) -> Option<f32> {
    let raw = text(tag, key)?;
    raw.trim()
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn recognises_audio_by_extension_regardless_of_case() {
        for name in ["a.flac", "a.FLAC", "a.Mp3", "a.AIFF", "a.m4a"] {
            assert!(
                is_audio_file(&PathBuf::from(name)),
                "{name} should be audio"
            );
        }
    }

    #[test]
    fn ignores_everything_else() {
        // `.opus` and `.mp4` are here on purpose. Opus has no decoder in
        // Symphonia 0.5, and `.mp4` is a video collection far more often than
        // an audio one -- see `AUDIO_EXTENSIONS`.
        for name in [
            "cover.jpg",
            "notes.txt",
            "album",
            "a.flac.bak",
            ".hidden",
            "a.opus",
            "a.mp4",
        ] {
            assert!(
                !is_audio_file(&PathBuf::from(name)),
                "{name} should not be audio"
            );
        }
    }
}
