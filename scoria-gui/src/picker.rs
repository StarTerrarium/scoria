//! Choosing the library folder.
//!
//! `DESIGN.md` §5.1: one root, chosen once. Both ways in -- the first-run
//! panel and the settings drawer -- come through here, so there is a single
//! folder-picking path rather than two that drift.

use std::path::{Path, PathBuf};

/// Opens the desktop's folder chooser.
///
/// The picker is a separate process talking to the XDG portal, so this is
/// awaited off the UI thread rather than blocking the window for as long as
/// someone browses.
pub async fn choose_folder(start: Option<PathBuf>) -> Option<PathBuf> {
    let mut dialog = rfd::AsyncFileDialog::new().set_title("Music library");
    if let Some(start) = start {
        dialog = dialog.set_directory(start);
    }
    dialog
        .pick_folder()
        .await
        .map(|handle| handle.path().to_path_buf())
}

/// Where to open the chooser.
///
/// The configured root when there is one, so "choose a different folder" starts
/// beside the current one. Otherwise the desktop's own music directory, because
/// opening at the filesystem root on first run asks someone to navigate to a
/// place the system already knows.
#[must_use]
pub fn suggested_start(current: Option<PathBuf>) -> Option<PathBuf> {
    match current {
        Some(root) if root.is_dir() => Some(root),
        _ => music_dir(),
    }
}

/// The desktop's music directory, if it has one that exists.
fn music_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;

    // Exported by some shells and set by the test suite. Cheaper than reading a
    // file, and it is what the user-dirs machinery is for.
    if let Some(dir) = std::env::var_os("XDG_MUSIC_DIR").map(PathBuf::from)
        && dir.is_dir()
    {
        return Some(dir);
    }

    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    if let Ok(text) = std::fs::read_to_string(config.join("user-dirs.dirs"))
        && let Some(dir) = parse_user_dirs(&text, &home)
        && dir.is_dir()
    {
        return Some(dir);
    }

    // The conventional name, and the fallback xdg-user-dirs itself uses.
    let conventional = home.join("Music");
    if conventional.is_dir() {
        return Some(conventional);
    }
    home.is_dir().then_some(home)
}

/// Reads `XDG_MUSIC_DIR` out of a `user-dirs.dirs` file.
///
/// The format is shell-assignment-shaped -- `XDG_MUSIC_DIR="$HOME/Music"` --
/// but it is not evaluated by a shell, and the only expansion the specification
/// allows is a leading `$HOME`.
#[must_use]
pub fn parse_user_dirs(text: &str, home: &Path) -> Option<PathBuf> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != "XDG_MUSIC_DIR" {
            continue;
        }
        let value = value.trim().trim_matches('"');
        // A bare `$HOME` is the whole value; `$HOME/Music` is the usual one.
        let path = match value.strip_prefix("$HOME") {
            Some(rest) => home.join(rest.trim_start_matches('/')),
            None => PathBuf::from(value),
        };
        // An empty value means "this user has no music directory", which is not
        // the same as the file being silent about one.
        if path.as_os_str().is_empty() || path == home {
            return None;
        }
        return Some(path);
    }
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/home/someone")
    }

    #[test]
    fn the_usual_line_expands_home() {
        let text = "XDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_MUSIC_DIR=\"$HOME/Music\"\n";
        assert_eq!(
            parse_user_dirs(text, &home()),
            Some(PathBuf::from("/home/someone/Music"))
        );
    }

    #[test]
    fn an_absolute_value_is_taken_as_written() {
        let text = "XDG_MUSIC_DIR=\"/mnt/media/music\"\n";
        assert_eq!(
            parse_user_dirs(text, &home()),
            Some(PathBuf::from("/mnt/media/music"))
        );
    }

    #[test]
    fn comments_and_other_directories_are_ignored() {
        let text = "# generated\n#XDG_MUSIC_DIR=\"$HOME/Wrong\"\nXDG_VIDEOS_DIR=\"$HOME/Videos\"\n";
        assert_eq!(parse_user_dirs(text, &home()), None);
    }

    #[test]
    fn a_disabled_directory_is_not_a_suggestion() {
        // xdg-user-dirs writes `$HOME` or an empty value to mean the user has
        // no such directory. Opening the chooser at their home folder would be
        // a worse guess than not guessing.
        for text in ["XDG_MUSIC_DIR=\"$HOME\"\n", "XDG_MUSIC_DIR=\"\"\n"] {
            assert_eq!(parse_user_dirs(text, &home()), None, "{text:?}");
        }
    }

    #[test]
    fn a_non_utf8_or_malformed_file_yields_nothing_rather_than_a_guess() {
        for text in ["", "nonsense", "XDG_MUSIC_DIR\n", "=\n"] {
            assert_eq!(parse_user_dirs(text, &home()), None, "{text:?}");
        }
    }

    #[test]
    fn an_existing_root_is_preferred_to_the_music_directory() {
        // "Choose a different folder" should start beside the current one.
        let existing = std::env::temp_dir();
        assert_eq!(
            suggested_start(Some(existing.clone())),
            Some(existing),
            "the configured root was not offered"
        );
    }

    #[test]
    fn a_root_that_no_longer_exists_falls_back() {
        // The folder was renamed or the drive is not mounted. Starting the
        // chooser somewhere that is not there shows an error dialog instead of
        // a file list.
        let missing = std::env::temp_dir().join("scoria-not-here-cbe1");
        assert_ne!(suggested_start(Some(missing.clone())), Some(missing));
    }
}
