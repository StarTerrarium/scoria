//! Watching the library root for changes.
//!
//! `DESIGN.md` §5.2: `notify` watches the root continuously, and an explicit
//! rescan is always available. This is the continuous half.
//!
//! ## Why it coalesces rather than reacting
//!
//! Filesystem events arrive in storms. Copying an album emits a create and
//! several writes per file; a tagger rewriting a directory can emit hundreds of
//! events in a second, and many are for temporary files that no longer exist by
//! the time anything reads them. Reacting to each one would mean re-reading
//! tags for a file still being written.
//!
//! So this reports *that* something changed under a set of directories, after
//! things have gone quiet, and lets the caller decide what to rescan. The
//! coalescing window is the difference between a scan per file and a scan per
//! album drop.

use crate::error::CoreError;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

/// How long the root must be quiet before a change is reported.
pub const QUIET_PERIOD: Duration = Duration::from_millis(750);

/// A settled batch of filesystem changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    /// Directories containing something that changed. Deduplicated, so copying
    /// a 20-track album yields one entry rather than twenty.
    pub directories: Vec<PathBuf>,
}

/// Watches a library root.
///
/// Dropping this stops the watch.
pub struct LibraryWatcher {
    // Held to keep the watch alive; notify stops watching when it is dropped.
    _watcher: RecommendedWatcher,
    events: Receiver<Event>,
}

impl LibraryWatcher {
    /// Starts watching `root` and everything under it.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the platform watcher cannot be created or the
    /// root cannot be watched.
    pub fn watch(root: &Path) -> Result<Self, CoreError> {
        let (sender, events) = channel();

        let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
            // A send failure means the receiver is gone, which means the
            // watcher is being dropped. Nothing to do and nothing to report.
            if let Ok(event) = result {
                let _ = sender.send(event);
            }
        })
        .map_err(|e| CoreError::Database(format!("cannot create watcher: {e}")))?;

        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(|e| CoreError::Database(format!("cannot watch {}: {e}", root.display())))?;

        Ok(Self {
            _watcher: watcher,
            events,
        })
    }

    /// Blocks until something changes and then settles, or until `timeout`
    /// passes with nothing happening.
    ///
    /// Returns `None` on timeout, so a caller can poll this in a loop and still
    /// notice a shutdown request between calls.
    #[must_use]
    pub fn next_change(&self, timeout: Duration) -> Option<Changed> {
        let first = self.events.recv_timeout(timeout).ok()?;
        let mut directories = HashSet::new();
        collect(&first, &mut directories);

        // Keep draining until the storm dies down. The deadline stops a
        // continuously busy directory from deferring the report forever.
        let deadline = Instant::now() + QUIET_PERIOD * 8;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match self.events.recv_timeout(QUIET_PERIOD.min(remaining)) {
                Ok(event) => collect(&event, &mut directories),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        if directories.is_empty() {
            return None;
        }
        let mut directories: Vec<PathBuf> = directories.into_iter().collect();
        directories.sort();
        Some(Changed { directories })
    }
}

/// Records the directories an event touched.
fn collect(event: &Event, directories: &mut HashSet<PathBuf>) {
    // Access events fire on ordinary reads -- including our own, when the
    // scanner reads tags. Treating them as changes would make a scan trigger
    // another scan.
    if matches!(event.kind, EventKind::Access(_) | EventKind::Other) {
        return;
    }

    for path in &event.paths {
        // The path may already be gone (a temporary file, a delete), so this
        // cannot ask the filesystem whether it is a directory.
        let directory = if path.extension().is_some() {
            path.parent().map(Path::to_path_buf)
        } else {
            Some(path.clone())
        };
        if let Some(directory) = directory {
            directories.insert(directory);
        }
    }
}
