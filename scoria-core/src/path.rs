//! Library-relative paths.
//!
//! `DESIGN.md` §4.3: track paths are stored relative to the library root, so
//! moving a library between drives is a settings change rather than a rescan.
//!
//! That rule is enforced by this type rather than by remembering to follow it.
//! A `RelativePath` cannot be constructed from an absolute path or from one
//! containing `..`, so "no absolute path reaches storage" is true by
//! construction and the repository layer inherits it for free.

use crate::error::CoreError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// A path below the library root. Never absolute, never containing `..`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RelativePath(PathBuf);

impl RelativePath {
    /// Rejects absolute paths and any path that could escape the root.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotRelative`] if the path is absolute or rooted;
    /// [`CoreError::EscapesRoot`] if it contains a `..` component.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, CoreError> {
        let path = path.into();

        if path.is_absolute() {
            return Err(CoreError::NotRelative {
                path: path.display().to_string(),
            });
        }

        for component in path.components() {
            match component {
                Component::ParentDir => {
                    return Err(CoreError::EscapesRoot {
                        path: path.display().to_string(),
                    });
                }
                // A prefix or root on a relative-looking path (`\windows\path`)
                // is still an escape from our root.
                Component::Prefix(_) | Component::RootDir => {
                    return Err(CoreError::NotRelative {
                        path: path.display().to_string(),
                    });
                }
                Component::CurDir | Component::Normal(_) => {}
            }
        }

        Ok(Self(path))
    }

    /// Strips `root` from an absolute path to produce a library-relative one.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotRelative`] if `path` is not below `root`.
    pub fn from_absolute(path: &Path, root: &Path) -> Result<Self, CoreError> {
        let stripped = path
            .strip_prefix(root)
            .map_err(|_| CoreError::NotRelative {
                path: path.display().to_string(),
            })?;
        Self::new(stripped)
    }

    /// Resolves against the library root. The one place a full path is formed.
    #[must_use]
    pub fn to_absolute(&self, root: &Path) -> PathBuf {
        root.join(&self.0)
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// The album directory this file sits in, used by the artwork sidecar
    /// lookup and by the last rung of the album identity ladder (§4.2).
    #[must_use]
    pub fn parent(&self) -> Option<&Path> {
        self.0.parent()
    }
}

impl fmt::Display for RelativePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn accepts_a_path_below_the_root() {
        let p = RelativePath::new("Electric Callboy/TEKKNO/01 Hurrikan.flac").unwrap();
        assert_eq!(p.to_string(), "Electric Callboy/TEKKNO/01 Hurrikan.flac");
    }

    #[test]
    fn accepts_non_ascii_paths() {
        // The library is full of these; they must survive the round trip.
        let p = RelativePath::new("花冷え。/乙女改革/01 SUNRISE 味噌SOUP.flac").unwrap();
        assert_eq!(p.parent().unwrap().to_string_lossy(), "花冷え。/乙女改革");
    }

    #[test]
    fn rejects_absolute_paths() {
        let err = RelativePath::new("/home/someone/Music/a.flac").unwrap_err();
        assert!(matches!(err, CoreError::NotRelative { .. }));
    }

    #[test]
    fn rejects_paths_that_escape_the_root() {
        let err = RelativePath::new("../../etc/passwd").unwrap_err();
        assert!(matches!(err, CoreError::EscapesRoot { .. }));

        let err = RelativePath::new("Metallica/../../secrets").unwrap_err();
        assert!(matches!(err, CoreError::EscapesRoot { .. }));
    }

    #[test]
    fn strips_the_root_from_an_absolute_path() {
        let p = RelativePath::from_absolute(
            Path::new("/home/someone/Music/Ghost/Meliora/01 Spirit.flac"),
            Path::new("/home/someone/Music"),
        )
        .unwrap();
        assert_eq!(p.to_string(), "Ghost/Meliora/01 Spirit.flac");
    }

    #[test]
    fn refuses_to_strip_a_root_the_path_is_not_under() {
        let err = RelativePath::from_absolute(
            Path::new("/var/other/a.flac"),
            Path::new("/home/someone/Music"),
        )
        .unwrap_err();
        assert!(matches!(err, CoreError::NotRelative { .. }));
    }

    #[test]
    fn resolves_against_a_root_and_survives_relocation() {
        // The whole point of §4.3: the same stored path resolves against a new
        // root without a rescan.
        let p = RelativePath::new("Ghost/Meliora/01 Spirit.flac").unwrap();
        assert_eq!(
            p.to_absolute(Path::new("/home/someone/Music")),
            Path::new("/home/someone/Music/Ghost/Meliora/01 Spirit.flac")
        );
        assert_eq!(
            p.to_absolute(Path::new("/mnt/archive")),
            Path::new("/mnt/archive/Ghost/Meliora/01 Spirit.flac")
        );
    }

    #[test]
    fn survives_a_serde_round_trip() {
        let p = RelativePath::new("花冷え。/乙女改革/01.flac").unwrap();
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<RelativePath>(&json).unwrap(), p);
    }
}
