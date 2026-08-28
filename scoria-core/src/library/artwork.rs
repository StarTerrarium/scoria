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
use crate::library::identity::AlbumIdentity;
use lofty::file::TaggedFileExt;
use std::collections::HashSet;
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

/// What a cached image is filed under.
///
/// Derived from the album's *identity* (§4.2), not from its database id.
///
/// An id is a rowid, and rowids are reassigned whenever the database is rebuilt
/// -- something that happens after a corruption, a schema change, or anyone
/// deleting a file they were told is derived data. Art keyed by rowid survives
/// that rebuild attached to whatever album inherits the number, which shows the
/// wrong cover with no error anywhere. Identity is stable by construction: it
/// is what makes two files the same record in the first place.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtworkKey(String);

impl ArtworkKey {
    /// Hashes an album identity into something filename-safe.
    ///
    /// FNV-1a rather than `DefaultHasher`: the standard library makes no
    /// promise that its hashes are the same between releases, and a cache whose
    /// filenames change under a compiler upgrade is a cache that silently
    /// empties itself.
    #[must_use]
    pub fn for_album(identity: &AlbumIdentity) -> Self {
        Self(Self::hash(&identity.key()))
    }

    fn hash(text: &str) -> String {
        const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;

        let mut hash = OFFSET;
        for byte in text.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
        format!("{hash:016x}")
    }

    /// Rebuilds the key from an identity string already in storage.
    ///
    /// The `albums` table keeps the identity verbatim, so a lookup does not
    /// have to reconstruct an [`AlbumIdentity`] from its parts.
    #[must_use]
    pub fn from_stored(identity_key: &str) -> Self {
        Self(Self::hash(identity_key))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ArtworkKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Artwork stored on disk, keyed by album identity.
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
    pub fn get(&self, album: &ArtworkKey) -> Option<PathBuf> {
        SIDECAR_EXTENSIONS
            .iter()
            .map(|extension| self.directory.join(format!("{album}.{extension}")))
            .find(|path| path.exists())
    }

    #[must_use]
    pub fn has(&self, album: &ArtworkKey) -> bool {
        self.get(album).is_some()
    }

    /// Stores artwork for an album, replacing anything already cached for it.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the cache directory cannot be written.
    pub fn store(&self, album: &ArtworkKey, artwork: &Artwork) -> Result<PathBuf, CoreError> {
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

    /// Removes anything not belonging to one of `keep`.
    ///
    /// Run at the end of a scan, so the cache mirrors the library: art for
    /// albums that no longer exist goes, and so does anything left by an older
    /// naming scheme. Derived data that only ever grows is a leak.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the directory cannot be read or a file cannot
    /// be removed.
    pub fn retain(&self, keep: &HashSet<ArtworkKey>) -> Result<usize, CoreError> {
        let entries = match std::fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            // Nothing cached yet is not a failure to prune.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => {
                return Err(CoreError::Database(format!(
                    "cannot read {}: {e}",
                    self.directory.display()
                )));
            }
        };

        let mut removed = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if keep.contains(&ArtworkKey(stem.to_owned())) {
                continue;
            }
            if std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Drops any cached artwork for an album.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if a cached file exists but cannot be removed.
    pub fn forget(&self, album: &ArtworkKey) -> Result<(), CoreError> {
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
