//! Running an analysis: albums in parallel, each analysed whole and then
//! written.
//!
//! Its own threads, never the audio thread (invariant 9), and never tokio:
//! decoding is CPU work, and a pool sized to leave a core free keeps playback
//! untouched while it runs.

use super::compute;
use super::measure::{Measurement, measure};
use super::plan::{AlbumJob, Skipped};
use super::write::{GainTags, write};
use crate::error::CoreError;
use crate::event::Event;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};

/// How an analysis went. Carried by [`Event::GainFinished`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GainReport {
    /// Albums analysed and written, whether or not every file took.
    pub albums_done: usize,
    pub albums_total: usize,
    /// Stopped before the end, at someone's request.
    pub stopped: bool,
    /// Left out when planned, by reason.
    pub skipped: Skipped,
    /// Files that could not be measured or written, with why: the path as a
    /// string, because this crosses the seam.
    pub failed: Vec<(String, String)>,
}

/// What a running job shares with the session that started it.
#[derive(Clone)]
pub struct Shared {
    pub cancel: Arc<AtomicBool>,
    /// Files that must not be rewritten right now: the one playing and the one
    /// preloaded after it. A file rewritten while it is being decoded can
    /// shift under the decoder if its tag block grows.
    pub protected: Arc<Mutex<HashSet<PathBuf>>>,
    /// Writes put off because their file was protected, for the session to
    /// make once playback has moved on.
    pub held: Arc<Mutex<Vec<(PathBuf, GainTags)>>>,
    /// Set while the watch thread is scanning; the job waits for it before
    /// writing anything, so a scan never reads a file mid-write.
    pub scan_running: Arc<AtomicBool>,
}

/// An album measured and computed, ready to write.
pub struct Analysed {
    pub writes: Vec<(PathBuf, GainTags)>,
}

/// Measures every track on an album and computes its gains.
///
/// All or nothing: a track that will not decode leaves the album unwritten,
/// because album gain without it would be wrong for the rest. `None` if
/// `cancel` was set partway -- checked between tracks, so stopping waits for
/// one track rather than a whole album, and an album abandoned this way has
/// had nothing written.
///
/// # Errors
///
/// The first file that failed, and why.
pub fn analyse(
    album: &AlbumJob,
    cancel: &AtomicBool,
) -> Result<Option<Analysed>, (PathBuf, CoreError)> {
    let mut measurements: Vec<Measurement> = Vec::with_capacity(album.tracks.len());
    for path in &album.tracks {
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        measurements.push(measure(path).map_err(|e| (path.clone(), e))?);
    }
    let gains = compute::album(&measurements);

    let unmeasurable = |path: &Path| {
        (
            path.to_path_buf(),
            CoreError::Audio(format!("{}: could not be measured", path.display())),
        )
    };
    let first = album.tracks.first().map_or(Path::new(""), PathBuf::as_path);
    let album_gain = gains.album.ok_or_else(|| unmeasurable(first))?;

    let writes = album
        .tracks
        .iter()
        .zip(gains.tracks)
        .map(|(path, track)| {
            let track = track.ok_or_else(|| unmeasurable(path))?;
            Ok((
                path.clone(),
                GainTags {
                    track,
                    album: album_gain,
                },
            ))
        })
        .collect::<Result<_, _>>()?;
    Ok(Some(Analysed { writes }))
}

/// How many albums to work on at once: every core but one, and at least one.
fn workers() -> usize {
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZero::get)
        .saturating_sub(1)
        .max(1)
}

/// Runs the whole job, reporting progress and then a [`GainReport`].
///
/// Blocks until done or stopped; the session runs it on a thread of its own.
pub fn run(albums: &[AlbumJob], skipped: Skipped, shared: &Shared, events: &Sender<Event>) {
    // A scan already in flight finishes first. The session has already told
    // the watch thread to hold any new ones.
    while shared.scan_running.load(Ordering::SeqCst) && !shared.cancel.load(Ordering::Acquire) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let failed: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
    let total = albums.len();

    std::thread::scope(|scope| {
        for _ in 0..workers().min(total.max(1)) {
            scope.spawn(|| {
                loop {
                    // Also checked between tracks, inside `analyse`. Never
                    // during writing: an album is only written once all of it
                    // is measured, and then all of it is written.
                    if shared.cancel.load(Ordering::Acquire) {
                        break;
                    }
                    let Some(album) = albums.get(next.fetch_add(1, Ordering::AcqRel)) else {
                        break;
                    };

                    let failures = match analyse(album, &shared.cancel) {
                        Ok(Some(analysed)) => write_album(analysed, shared),
                        // Stopped partway; nothing was written, and it does
                        // not count as done.
                        Ok(None) => break,
                        Err((path, e)) => vec![(path.display().to_string(), e.to_string())],
                    };
                    if !failures.is_empty() {
                        failed
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .extend(failures);
                    }

                    let finished = done.fetch_add(1, Ordering::AcqRel).saturating_add(1);
                    let _ = events.send(Event::GainProgress {
                        done: finished,
                        total,
                        album: album.title.clone(),
                    });
                }
            });
        }
    });

    let albums_done = done.load(Ordering::Acquire);
    let _ = events.send(Event::GainFinished(GainReport {
        albums_done,
        albums_total: total,
        stopped: albums_done < total,
        skipped,
        failed: failed.into_inner().unwrap_or_else(PoisonError::into_inner),
    }));
}

/// Writes an analysed album, holding back any file that is protected.
fn write_album(analysed: Analysed, shared: &Shared) -> Vec<(String, String)> {
    let mut failures = Vec::new();
    for (path, tags) in analysed.writes {
        let protected = shared
            .protected
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&path);
        if protected {
            shared
                .held
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((path, tags));
            continue;
        }
        if let Err(e) = write(&path, &tags) {
            failures.push((path.display().to_string(), e.to_string()));
        }
    }
    failures
}

/// Makes any held write whose file is no longer protected.
///
/// Called by the session whenever what is playing changes. Failures are
/// returned to be reported; the write is not retried.
pub fn flush_held(
    protected: &Mutex<HashSet<PathBuf>>,
    held: &Mutex<Vec<(PathBuf, GainTags)>>,
) -> Vec<CoreError> {
    let protected = protected
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let ready: Vec<(PathBuf, GainTags)> = {
        let mut held = held.lock().unwrap_or_else(PoisonError::into_inner);
        let (ready, still): (Vec<_>, Vec<_>) = held
            .drain(..)
            .partition(|(path, _)| !protected.contains(path));
        *held = still;
        ready
    };
    ready
        .iter()
        .filter_map(|(path, tags)| write(path, tags).err())
        .collect()
}
