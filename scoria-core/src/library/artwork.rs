//! Album artwork: finding it, and caching what was found.
//!
//! `DESIGN.md` §5.3. Extracted at scan time rather than on demand, because the
//! transport bar wants a thumbnail for whatever starts playing and a
//! decode-on-first-paint stutter is exactly the kind of thing a dense UI shows
//! off.
//!
//! ## Why nothing is resized
//!
//! The cache stores the artwork exactly as it was found. Rescaling would need
//! an image codec dependency, and it would buy nothing here: artwork always
//! follows what is playing (§9.1), so at most one image is on screen at a time.
//! There is no album grid in this design -- that is what would make a thumbnail
//! pipeline worth its weight. The renderer scales one image when it draws it.
//!
//! What the cache does buy is not having to open and parse an audio file to
//! find the picture again, which is the expensive half.

use crate::error::CoreError;
use crate::id::AlbumId;
use lofty::file::TaggedFileExt;
use std::path::{Path, PathBuf};

/// Sidecar filenames, in the order they are preferred.
///
/// Embedded artwork wins over all of these: it travels with the file and is
/// what Picard writes.
const SIDECAR_STEMS: &[&str] = &["cover", "folder", "front", "album", "artwork"];
const SIDECAR_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "gif"];

/// Artwork found for an album, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Artwork {
    /// Read out of the audio file itself.
    Embedded {
        data: Vec<u8>,
        extension: &'static str,
    },
    /// A separate image file sitting in the album directory.
    Sidecar(PathBuf),
}

/// Pulls the front cover out of an audio file, if it has one.
///
/// Prefers a picture explicitly typed as the front cover; falls back to the
/// first picture present, because plenty of files carry exactly one picture
/// with no type set.
#[must_use]
pub fn embedded(path: &Path) -> Option<Artwork> {
    let file = lofty::read_from_path(path).ok()?;
    let tag = file.primary_tag().or_else(|| file.first_tag())?;
    let pictures = tag.pictures();

    let picture = pictures
        .iter()
        .find(|p| p.pic_type() == lofty::picture::PictureType::CoverFront)
        .or_else(|| pictures.first())?;

    Some(Artwork::Embedded {
        data: picture.data().to_vec(),
        extension: extension_for(picture.mime_type()),
    })
}

/// Looks for a cover image alongside the audio files.
#[must_use]
pub fn sidecar(directory: &Path) -> Option<Artwork> {
    // Read the directory once and match case-insensitively, rather than
    // probing a few dozen exact filenames. Libraries contain `Cover.jpg` and
    // `folder.JPG` in roughly equal measure.
    let entries: Vec<_> = std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .collect();

    for stem in SIDECAR_STEMS {
        for extension in SIDECAR_EXTENSIONS {
            let wanted = format!("{stem}.{extension}");
            if let Some(entry) = entries.iter().find(|e| {
                e.file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&wanted)
            }) {
                return Some(Artwork::Sidecar(entry.path()));
            }
        }
    }
    None
}

/// Embedded first, then a sidecar in the containing directory (§5.3).
#[must_use]
pub fn find(track_path: &Path) -> Option<Artwork> {
    embedded(track_path).or_else(|| sidecar(track_path.parent()?))
}

fn extension_for(mime: Option<&lofty::picture::MimeType>) -> &'static str {
    use lofty::picture::MimeType;
    match mime {
        Some(MimeType::Png) => "png",
        Some(MimeType::Gif) => "gif",
        Some(MimeType::Bmp) => "bmp",
        // Unknown or absent means we did not recognise it, not that it is
        // broken; JPEG is overwhelmingly what album art actually is.
        _ => "jpg",
    }
}

/// Artwork stored on disk, keyed by album.
///
/// Everything here is derived data. Deleting the directory costs a rescan of
/// the artwork and nothing else, which is why it lives under the cache
/// directory rather than the data directory.
#[derive(Debug, Clone)]
pub struct ArtworkCache {
    directory: PathBuf,
}

impl ArtworkCache {
    #[must_use]
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The cached image for an album, if one has been stored.
    #[must_use]
    pub fn get(&self, album: AlbumId) -> Option<PathBuf> {
        SIDECAR_EXTENSIONS
            .iter()
            .map(|extension| self.directory.join(format!("{album}.{extension}")))
            .find(|path| path.exists())
    }

    #[must_use]
    pub fn has(&self, album: AlbumId) -> bool {
        self.get(album).is_some()
    }

    /// Stores artwork for an album, replacing anything already cached for it.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the cache directory cannot be written.
    pub fn store(&self, album: AlbumId, artwork: &Artwork) -> Result<PathBuf, CoreError> {
        std::fs::create_dir_all(&self.directory).map_err(|e| {
            CoreError::Database(format!("cannot create {}: {e}", self.directory.display()))
        })?;

        // An album whose art changed from PNG to JPEG would otherwise keep both,
        // and `get` would return whichever extension it happened to try first.
        self.forget(album)?;

        let (extension, data) = match artwork {
            Artwork::Embedded { data, extension } => (*extension, data.clone()),
            Artwork::Sidecar(path) => {
                let extension = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .and_then(|e| {
                        SIDECAR_EXTENSIONS
                            .iter()
                            .find(|k| k.eq_ignore_ascii_case(e))
                            .copied()
                    })
                    .unwrap_or("jpg");
                let data = std::fs::read(path).map_err(|e| {
                    CoreError::Database(format!("cannot read {}: {e}", path.display()))
                })?;
                (extension, data)
            }
        };

        let destination = self.directory.join(format!("{album}.{extension}"));
        std::fs::write(&destination, data).map_err(|e| {
            CoreError::Database(format!("cannot write {}: {e}", destination.display()))
        })?;
        Ok(destination)
    }

    /// Drops any cached artwork for an album.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if a cached file exists but cannot be removed.
    pub fn forget(&self, album: AlbumId) -> Result<(), CoreError> {
        for extension in SIDECAR_EXTENSIONS {
            let path = self.directory.join(format!("{album}.{extension}"));
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(CoreError::Database(format!(
                        "cannot remove {}: {e}",
                        path.display()
                    )));
                }
            }
        }
        Ok(())
    }
}
