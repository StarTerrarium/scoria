//! Settings and the paths they live at.
//!
//! `DESIGN.md` §2: configuration tunes defaults, it never constructs the
//! application. The test is whether someone who never opens settings still has
//! a fully usable player -- so everything here has a defensible default and the
//! file is optional.
//!
//! ## Why the shape is split
//!
//! One file, one section per owner. Core owns the library settings; the GUI
//! owns its own (theme, columns, browser placement):
//!
//! ```toml
//! [library]
//! root = "/home/user/Music"
//!
//! [ui]
//! theme = "scoria-dark"
//! ```
//!
//! That keeps one file for the user while leaving each crate owning its own
//! section -- core never has to name a UI concept to store one.
//!
//! **There is deliberately no whole-document read or write.** Neither side can
//! name the other's types, so serialising a whole document from either one
//! means writing a file with the other's settings missing. There used to be a
//! `save` that did exactly that, and core used it: every volume change wiped
//! `[ui]`. [`load_section`] and [`save_section`] are the only way in, and
//! [`load_library`] moves files written by that version into `[library]`.

use crate::error::CoreError;
use crate::query::sort::SortPreset;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fs;
use std::path::{Path, PathBuf};

/// Where Scoria keeps things, per the XDG base directory specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    config_dir: PathBuf,
    data_dir: PathBuf,
    cache_dir: PathBuf,
}

impl Paths {
    /// Resolves from the environment.
    ///
    /// # Errors
    ///
    /// [`CoreError::Config`] if neither the relevant `XDG_*` variable nor
    /// `HOME` gives an absolute path to work from.
    pub fn from_env() -> Result<Self, CoreError> {
        Ok(Self {
            config_dir: Self::base("XDG_CONFIG_HOME", ".config")?.join("scoria"),
            data_dir: Self::base("XDG_DATA_HOME", ".local/share")?.join("scoria"),
            cache_dir: Self::base("XDG_CACHE_HOME", ".cache")?.join("scoria"),
        })
    }

    /// Roots everything at one directory. For tests, and for anyone who wants
    /// a portable install.
    #[must_use]
    pub fn rooted_at(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        Self {
            config_dir: dir.join("config"),
            data_dir: dir.join("data"),
            cache_dir: dir.join("cache"),
        }
    }

    fn base(var: &str, fallback: &str) -> Result<PathBuf, CoreError> {
        // The spec says a relative XDG path is invalid and must be ignored,
        // rather than resolved against the working directory.
        if let Some(path) = std::env::var_os(var).map(PathBuf::from)
            && path.is_absolute()
        {
            return Ok(path);
        }
        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            CoreError::Config(format!("neither {var} nor HOME is set to an absolute path"))
        })?;
        Ok(home.join(fallback))
    }

    #[must_use]
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    #[must_use]
    pub fn database(&self) -> PathBuf {
        self.data_dir.join("library.db")
    }

    /// Thumbnail cache (§5.3). Derived data, safe to delete at any time.
    #[must_use]
    pub fn artwork_cache(&self) -> PathBuf {
        self.cache_dir.join("artwork")
    }

    #[must_use]
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    #[must_use]
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }
}

/// What gets snapshotted when a track is clicked (`DESIGN.md` §6.3).
///
/// The one user-facing option in the playback model. Both modes are snapshots;
/// they differ only in what gets snapshotted, and everything downstream is
/// identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlayBehaviour {
    /// Snapshot the current view's contents. The default.
    #[default]
    WhatsShown,
    /// Snapshot the entire library, ignoring filters but respecting the
    /// table's current sort -- sort is something the user just deliberately
    /// set.
    WholeLibrary,
}

/// The settings `scoria-core` owns.
///
/// Not `Eq`, because volume is a float. Everything else here is.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct LibraryConfig {
    /// The single library root (§5.1). `None` until the first-run picker runs.
    pub root: Option<PathBuf>,
    pub play_behaviour: PlayBehaviour,
    pub sort: SortPreset,
    /// Playback volume, 0 to 1.
    ///
    /// Core rather than frontend: a volume that reset when the window closed
    /// would be a surprise, and it is the same volume whichever frontend set
    /// it. Read through [`Self::volume`], which is what enforces the range on
    /// a file anyone can edit.
    volume: f32,
}

impl Default for LibraryConfig {
    fn default() -> Self {
        Self {
            root: None,
            play_behaviour: PlayBehaviour::default(),
            sort: SortPreset::default(),
            // Full, because a player that starts quiet looks broken.
            volume: 1.0,
        }
    }
}

impl LibraryConfig {
    /// The stored volume, always within range.
    ///
    /// Clamped on the way out rather than trusted: this file is meant to be
    /// hand-editable (§7.3), and `volume = 40` should be loud rather than
    /// whatever the backend does with a gain of forty.
    #[must_use]
    pub fn volume(&self) -> f32 {
        if self.volume.is_finite() {
            self.volume.clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    /// Records the volume, clamped.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            1.0
        };
    }

    /// The configured root, or an error naming what the user needs to do.
    ///
    /// # Errors
    ///
    /// [`CoreError::NoLibraryRoot`] when no root has been chosen.
    pub fn root(&self) -> Result<&Path, CoreError> {
        self.root.as_deref().ok_or(CoreError::NoLibraryRoot)
    }
}

/// The section `scoria-core`'s own settings live under.
///
/// They have not always: before this existed, core wrote its settings at the
/// top level and the frontend wrote `[ui]` beside them. See [`load_library`].
pub const LIBRARY_SECTION: &str = "library";

/// Reads a settings file as a document, or `None` if there is no file.
///
/// Shared by everything that has to preserve what it did not write, which is
/// every reader and writer of a sectioned file.
fn read_document(path: &Path) -> Result<Option<toml::Table>, CoreError> {
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text)
            .map(Some)
            .map_err(|e| CoreError::Config(format!("cannot parse {}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CoreError::Config(format!(
            "cannot read {}: {e}",
            path.display()
        ))),
    }
}

/// Reads one section of a settings file.
///
/// Missing file, or missing section, yields defaults. See [`save_section`] for
/// why sections exist.
///
/// # Errors
///
/// [`CoreError::Config`] if the file exists but cannot be read or parsed.
pub fn load_section<T: DeserializeOwned + Default>(
    path: &Path,
    section: &str,
) -> Result<T, CoreError> {
    let Some(document) = read_document(path)? else {
        return Ok(T::default());
    };

    let Some(value) = document.get(section) else {
        return Ok(T::default());
    };
    value.clone().try_into().map_err(|e| {
        CoreError::Config(format!(
            "cannot read [{section}] from {}: {e}",
            path.display()
        ))
    })
}

/// Writes one section of a settings file, leaving everything else untouched.
///
/// This is what makes one file work for several owners. Core owns the library
/// settings and a frontend owns its own; neither can name the other's types, so
/// neither can serialise the whole document. Writing the whole file from either
/// side would silently delete the other's settings the first time it saved.
///
/// # Errors
///
/// [`CoreError::Config`] if the existing file cannot be read or parsed, the
/// value cannot be serialised, or the file cannot be written.
pub fn save_section<T: Serialize>(path: &Path, section: &str, value: &T) -> Result<(), CoreError> {
    let mut document = read_document(path)?.unwrap_or_default();

    let value = toml::Value::try_from(value)
        .map_err(|e| CoreError::Config(format!("cannot serialise [{section}]: {e}")))?;
    document.insert(section.to_owned(), value);
    write_atomically(path, &document)
}

/// Reads the library settings, moving a file written before [`LIBRARY_SECTION`]
/// existed into it.
///
/// Core used to write its settings at the top level of the file while the
/// frontend wrote `[ui]` beside them -- and since neither side can name the
/// other's types, core's save serialised a bare [`LibraryConfig`] over the
/// whole document and took the frontend's settings with it every time the
/// volume settled. [`save_section`] exists precisely to stop that; core simply
/// was not using it.
///
/// Anything at the top level that is not a table belonged to core, so it moves
/// into `[library]` and the strays are dropped in the same write. Left in
/// place they would be a second, dead `root` sitting above the live one, which
/// matters because §7.3 sends people to this file to edit it by hand.
///
/// # Errors
///
/// [`CoreError::Config`] if the file cannot be read, parsed or rewritten.
pub fn load_library(path: &Path) -> Result<LibraryConfig, CoreError> {
    let Some(mut document) = read_document(path)? else {
        return Ok(LibraryConfig::default());
    };
    if document.contains_key(LIBRARY_SECTION) {
        return load_section(path, LIBRARY_SECTION);
    }

    let legacy: toml::Table = document
        .iter()
        .filter(|(_, value)| !value.is_table())
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if legacy.is_empty() {
        // A sectioned file that simply has no [library] yet, which is the
        // ordinary state of a fresh install whose frontend saved first.
        return Ok(LibraryConfig::default());
    }

    let config: LibraryConfig = toml::Value::Table(legacy).try_into().map_err(|e| {
        CoreError::Config(format!(
            "cannot read the library settings from {}: {e}",
            path.display()
        ))
    })?;

    document.retain(|_, value| value.is_table());
    document.insert(
        LIBRARY_SECTION.to_owned(),
        toml::Value::try_from(&config)
            .map_err(|e| CoreError::Config(format!("cannot serialise [{LIBRARY_SECTION}]: {e}")))?,
    );
    write_atomically(path, &document)?;
    Ok(config)
}

/// Writes the library settings, leaving every other section alone.
///
/// # Errors
///
/// [`CoreError::Config`] if the file cannot be read, parsed or written.
pub fn save_library(path: &Path, config: &LibraryConfig) -> Result<(), CoreError> {
    save_section(path, LIBRARY_SECTION, config)
}

/// Writes to a temporary file and renames over the target.
///
/// An interrupted write cannot leave a half-written config that fails to parse
/// on next start.
fn write_atomically<T: Serialize>(path: &Path, value: &T) -> Result<(), CoreError> {
    let text = toml::to_string_pretty(value)
        .map_err(|e| CoreError::Config(format!("cannot serialise settings: {e}")))?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| CoreError::Config(format!("cannot create {}: {e}", parent.display())))?;
    }

    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, text)
        .map_err(|e| CoreError::Config(format!("cannot write {}: {e}", temporary.display())))?;
    fs::rename(&temporary, path)
        .map_err(|e| CoreError::Config(format!("cannot replace {}: {e}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn the_volume_starts_full() {
        // A player that opens quiet looks broken, and the first thing anyone
        // would do is reach for the volume they cannot hear.
        assert_eq!(LibraryConfig::default().volume(), 1.0);
    }

    #[test]
    fn a_hand_edited_volume_is_brought_into_range() {
        // This file is meant to be edited by hand (§7.3), so `volume = 40`
        // should be loud rather than whatever the backend does with a gain of
        // forty.
        let mut config = LibraryConfig::default();
        for (written, expected) in [(40.0, 1.0), (-3.0, 0.0), (0.5, 0.5)] {
            config.set_volume(written);
            assert_eq!(config.volume(), expected, "{written} was stored as written");
        }
    }

    #[test]
    fn a_volume_that_is_not_a_number_falls_back_to_full() {
        // NaN compares false against everything, so a clamp alone would let it
        // through and every later comparison would quietly misbehave.
        let mut config = LibraryConfig::default();
        config.set_volume(f32::NAN);
        assert_eq!(config.volume(), 1.0);
        config.set_volume(f32::INFINITY);
        assert_eq!(config.volume(), 1.0);
    }

    #[test]
    fn a_volume_read_from_a_file_is_clamped_too() {
        // The setter is not the only way in: serde writes the field directly.
        let text = "volume = 12.0\n";
        let config: LibraryConfig = toml::from_str(text).unwrap();
        assert_eq!(config.volume(), 1.0);
    }
}
