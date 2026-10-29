//! Writing ReplayGain tags into files.
//!
//! **The only code in Scoria that writes to the library.** Everything else
//! reads, and `scripts/check-boundaries.sh` fails the build if lofty's save
//! calls appear anywhere but here. A carefully Picard-tagged library is the
//! source of truth (§13), and the way to keep it safe is for there to be one
//! place that can change it, doing one thing.
//!
//! ## How it avoids damaging a file
//!
//! - **Concrete format types, never lofty's generic `Tag`.** Converting a
//!   file's tag to the generic form and back can lose what the generic form
//!   has no room for. Editing the Vorbis comments in place keeps every other
//!   comment exactly as it was, in its order.
//! - **Strict parsing.** lofty's default is to make a best attempt at a
//!   malformed tag, which means a field it could not parse is quietly absent
//!   from what it read -- and so from what it would write back. A file lofty
//!   cannot read completely is refused instead.
//! - **Cover art is read in.** A FLAC's pictures live beside its comments, and
//!   a file read without them would be written back without them.
//! - **Four keys, nothing else.** The `REPLAYGAIN_*` keys are replaced; any
//!   other key, including `R128_*` and any lower-case variant someone else
//!   wrote, is left alone.

use super::compute::Gain;
use crate::error::CoreError;
use lofty::config::{ParseOptions, ParsingMode, WriteOptions};
use lofty::file::{AudioFile, FileType};
use lofty::flac::FlacFile;
use lofty::ogg::VorbisFile;
use lofty::ogg::tag::VorbisComments;
use std::fs::File;
use std::path::Path;

/// What is written for one track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GainTags {
    pub track: Gain,
    pub album: Gain,
}

/// Whether a file can be written, judged from its extension.
///
/// For planning only: the writer checks the file's real type, because an
/// extension can lie (§5.2).
#[must_use]
pub fn supported_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "flac" | "ogg" | "oga"))
}

/// Writes `tags` into the file at `path`.
///
/// # Errors
///
/// [`CoreError::Io`] naming the file, if it cannot be opened, read strictly,
/// is not FLAC or Ogg Vorbis, or cannot be written. Nothing is written in any
/// of those cases.
pub fn write(path: &Path, tags: &GainTags) -> Result<(), CoreError> {
    let fail = |what: &str| CoreError::Io(format!("{}: {what}", path.display()));

    let file_type = lofty::probe::Probe::open(path)
        .map_err(|e| fail(&e.to_string()))?
        .guess_file_type()
        .map_err(|e| fail(&e.to_string()))?
        .file_type();

    let options = ParseOptions::new()
        .parsing_mode(ParsingMode::Strict)
        .read_cover_art(true)
        .read_properties(false);
    let mut file = File::open(path).map_err(|e| fail(&e.to_string()))?;

    match file_type {
        Some(FileType::Flac) => {
            let mut flac =
                FlacFile::read_from(&mut file, options).map_err(|e| fail(&e.to_string()))?;
            drop(file);
            match flac.vorbis_comments_mut() {
                Some(comments) => apply(comments, tags),
                None => {
                    let mut comments = VorbisComments::default();
                    apply(&mut comments, tags);
                    flac.set_vorbis_comments(comments);
                }
            }
            flac.save_to_path(path, WriteOptions::default())
                .map_err(|e| fail(&e.to_string()))
        }
        Some(FileType::Vorbis) => {
            let mut vorbis =
                VorbisFile::read_from(&mut file, options).map_err(|e| fail(&e.to_string()))?;
            drop(file);
            apply(vorbis.vorbis_comments_mut(), tags);
            vorbis
                .save_to_path(path, WriteOptions::default())
                .map_err(|e| fail(&e.to_string()))
        }
        Some(other) => Err(fail(&format!(
            "{other:?} files cannot be given loudness tags yet"
        ))),
        None => Err(fail("not a recognised audio file")),
    }
}

/// Replaces the four ReplayGain comments, upper case as rsgain writes them.
fn apply(comments: &mut VorbisComments, tags: &GainTags) {
    for (key, value) in [
        ("REPLAYGAIN_TRACK_GAIN", tags.track.gain_tag()),
        ("REPLAYGAIN_TRACK_PEAK", tags.track.peak_tag()),
        ("REPLAYGAIN_ALBUM_GAIN", tags.album.gain_tag()),
        ("REPLAYGAIN_ALBUM_PEAK", tags.album.peak_tag()),
    ] {
        comments.insert(key.to_owned(), value);
    }
}
