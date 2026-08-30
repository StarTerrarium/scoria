//! Fixtures shared by the integration tests.
//!
//! Three files had grown their own scratch-directory helper -- with three
//! different naming schemes, and only one of them cleaning up after itself --
//! and two had their own copy of `db()`. A fixture that differs between test
//! files is a fixture that can make one file pass and another fail for reasons
//! that have nothing to do with the code.
//!
//! `src/test_support.rs` cannot serve here: it is `#[cfg(test)]`, so it exists
//! only for the crate's own unit tests, and an integration test compiles
//! against the crate as an outside consumer. It also builds `Track`, the read
//! model, where everything here goes in through the repository.

// Each test binary gets its own copy of this module and uses part of it.
#![allow(dead_code, clippy::unwrap_used)]

use lofty::config::WriteOptions;
use lofty::prelude::{ItemKey, TagExt};
use lofty::tag::{Tag, TagType};
use scoria_core::db::SqliteDatabase;
use std::path::{Path, PathBuf};

/// Fifty milliseconds of silence, and the smallest real FLAC worth keeping in
/// a repository. Retagged per file, so the tag path under test is the one that
/// runs in production.
pub const SILENCE: &[u8] = include_bytes!("../fixtures/silence.flac");

pub fn db() -> SqliteDatabase {
    SqliteDatabase::open_in_memory().unwrap()
}

/// A scratch directory that removes itself.
///
/// Named for the test and the thread as well as the process, so tests running
/// in parallel cannot land in each other's tree.
pub struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "scoria-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    /// The directory itself, for tests that scan it directly.
    pub fn path(&self) -> &Path {
        &self.dir
    }

    pub fn join(&self, relative: &str) -> PathBuf {
        self.dir.join(relative)
    }

    /// A library root under the scratch directory, for tests that also want
    /// settings and a database beside it.
    pub fn root(&self) -> PathBuf {
        let root = self.dir.join("music");
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// Writes a real FLAC carrying the given Vorbis comments.
    pub fn write_track(&self, base: &Path, relative: &str, fields: &[(ItemKey, &str)]) -> PathBuf {
        let path = base.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, SILENCE).unwrap();

        let mut tag = Tag::new(TagType::VorbisComments);
        for (key, value) in fields {
            tag.insert_text(*key, (*value).to_owned());
        }
        tag.save_to_path(&path, WriteOptions::default()).unwrap();
        path
    }

    /// The common case: a track that only needs a title, on an album that
    /// exists so the identity ladder does not fall to the directory rung.
    pub fn write_titled(&self, base: &Path, relative: &str, title: &str) -> PathBuf {
        self.write_track(
            base,
            relative,
            &[
                (ItemKey::TrackTitle, title),
                (ItemKey::AlbumArtist, "Watched"),
                (ItemKey::AlbumTitle, "Album"),
            ],
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Best effort: a test that has already failed should report that
        // rather than a tidy-up problem on top of it.
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
