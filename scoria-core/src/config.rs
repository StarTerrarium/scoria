//! Settings and the paths they live at.
//!
//! `DESIGN.md` §2: configuration tunes defaults, it never constructs the
//! application. The test is whether someone who never opens settings still has
//! a fully usable player -- so everything here has a defensible default and the
//! file is optional.
//!
//! ## Why the shape is split
//!
//! [`load`] and [`save`] are generic over the settings type rather than being
//! tied to one struct. Core owns the library settings; the GUI will own its own
//! (theme, columns, browser placement), and the binary composes them into a
//! single file:
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
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct LibraryConfig {
    /// The single library root (§5.1). `None` until the first-run picker runs.
    pub root: Option<PathBuf>,
    pub play_behaviour: PlayBehaviour,
    pub sort: SortPreset,
}

impl LibraryConfig {
    /// The configured root, or an error naming what the user needs to do.
    ///
    /// # Errors
    ///
    /// [`CoreError::NoLibraryRoot`] when no root has been chosen.
    pub fn root(&self) -> Result<&Path, CoreError> {
        self.root.as_deref().ok_or(CoreError::NoLibraryRoot)
    }
}

/// Reads a settings file, falling back to defaults when it does not exist.
///
/// A file that exists but does not parse is an error rather than a silent
/// reset: quietly discarding someone's settings because of a typo is worse
/// than refusing to start and saying which line is wrong.
///
/// # Errors
///
/// [`CoreError::Config`] if the file exists but cannot be read or parsed.
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> Result<T, CoreError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(e) => {
            return Err(CoreError::Config(format!(
                "cannot read {}: {e}",
                path.display()
            )));
        }
    };
    toml::from_str(&text)
        .map_err(|e| CoreError::Config(format!("cannot parse {}: {e}", path.display())))
}

/// Writes a settings file, creating the directory if needed.
///
/// Writes to a temporary file and renames over the target, so an interrupted
/// write cannot leave a half-written config that fails to parse on next start.
///
/// # Errors
///
/// [`CoreError::Config`] if the value cannot be serialised or the file cannot
/// be written.
pub fn save<T: Serialize>(path: &Path, value: &T) -> Result<(), CoreError> {
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
