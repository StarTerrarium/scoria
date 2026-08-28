//! The seam, assembled.
//!
//! A `Session` owns the library, the query engine and the audio thread, and is
//! the only thing that knows about all three. Frontends send [`Command`] and
//! consume [`Event`] (`DESIGN.md` §3.2); they never reach past this.
//!
//! That is what makes invariant 2 checkable rather than aspirational. A view
//! function calling `self.library.something()` is a violation, and the only way
//! to avoid needing to is for something below the UI to already do the
//! translating. This is that something -- and it is also why the TUI is cheap
//! and why putting a socket in the middle later is mechanical.

use crate::command::{Command, SourceSpec};
use crate::config::{LibraryConfig, Paths, PlayBehaviour};
use crate::db::repository::TrackRepository;
use crate::db::sqlite::SqliteDatabase;
use crate::error::CoreError;
use crate::event::{Event, LibraryDelta};
use crate::id::{AlbumId, QueryId, TrackId};
use crate::library::artwork::ArtworkKey;
use crate::library::scanner::{ScanOutcome, ScanProgress, Scanner};
use crate::library::{ArtworkCache, LibraryWatcher};
use crate::playback::backend::AudioBackend;
use crate::playback::engine::{EngineCommand, PlaybackHandle, spawn};
use crate::playback::resolver::IndexResolver;
use crate::playback::source::{Origin, PlaybackSource};
use crate::query::index::LibraryIndex;
use crate::query::sort::SortChain;
use crate::query::{QueryEngine, QueryOutcome, Selection};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;

/// Everything the core needs to answer a frontend.
pub struct Session {
    db: SqliteDatabase,
    index: LibraryIndex,
    config: LibraryConfig,
    paths: Paths,
    artwork: ArtworkCache,
    queries: QueryEngine,
    events: Sender<Event>,
    inbox: Receiver<Event>,
    /// Started on first use, so listing a library does not require a sound card.
    playback: Option<PlaybackHandle>,
    /// Watches the root and rescans in the background. See [`Session::start_watching`].
    watcher: Option<WatchHandle>,
}

/// How long the watch thread waits before checking whether it should stop.
///
/// Also the worst-case delay on an explicit rescan request, which is why it is
/// short rather than convenient.
const WATCH_POLL: Duration = Duration::from_millis(250);

/// A running watch thread.
///
/// Dropping it stops the thread and waits for it, so a session cannot leave one
/// behind scanning a library nothing is looking at any more.
#[derive(Debug)]
struct WatchHandle {
    stop: Arc<AtomicBool>,
    rescan: Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Session {
    /// Opens the database and settings at `paths`.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] or [`CoreError::Config`] if storage or settings
    /// cannot be read.
    pub fn open(paths: Paths) -> Result<Self, CoreError> {
        let config: LibraryConfig = crate::config::load(&paths.config_file())?;
        let db = SqliteDatabase::open(&paths.database())?;
        let index = LibraryIndex::load(&db)?;
        let (events, inbox) = channel();

        Ok(Self {
            artwork: ArtworkCache::new(paths.artwork_cache()),
            db,
            index,
            config,
            paths,
            queries: QueryEngine::new(),
            events,
            inbox,
            playback: None,
            watcher: None,
        })
    }

    #[must_use]
    pub fn config(&self) -> &LibraryConfig {
        &self.config
    }

    #[must_use]
    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    #[must_use]
    pub fn artwork(&self) -> &ArtworkCache {
        &self.artwork
    }

    /// The cached artwork for an album, if there is any.
    ///
    /// Goes through the album's identity rather than its id, which is the whole
    /// point: an id is a rowid and is reassigned when the database is rebuilt,
    /// so art filed under one would resurface attached to whichever album
    /// inherited the number. Resolved from the in-memory index, because a
    /// frontend asks for this while drawing.
    #[must_use]
    pub fn artwork_for(&self, album: AlbumId) -> Option<PathBuf> {
        let album = self.index.album(album)?;
        self.artwork.get(&ArtworkKey::from_stored(&album.identity))
    }

    #[must_use]
    pub fn index(&self) -> &LibraryIndex {
        &self.index
    }

    /// Runs a selection directly, for callers that want the panes and groups
    /// rather than just the ids an event carries.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the query fails.
    pub fn query(&self, selection: &Selection) -> Result<QueryOutcome, CoreError> {
        self.queries.run(&self.index, &self.db, selection)
    }

    /// Events produced since the last call, in order.
    ///
    /// One stream: playback, scanning and queries all arrive here.
    pub fn drain_events(&self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(event) = self.inbox.try_recv() {
            events.push(event);
        }
        events
    }

    /// Blocks for the next event, up to `timeout`.
    #[must_use]
    pub fn next_event(&self, timeout: std::time::Duration) -> Option<Event> {
        self.inbox.recv_timeout(timeout).ok()
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    /// Watches the library root for changes (§5.2).
    ///
    /// Returns the watcher for a caller that wants to drive it itself. Most
    /// callers want [`Session::start_watching`] instead.
    ///
    /// # Errors
    ///
    /// [`CoreError::NoLibraryRoot`] if no root is configured, or
    /// [`CoreError::Database`] if the platform watcher cannot be created.
    pub fn watch(&self) -> Result<LibraryWatcher, CoreError> {
        LibraryWatcher::watch(self.config.root()?)
    }

    /// Watches the root and rescans in the background when it changes (§5.2).
    ///
    /// The scan runs on its own thread with its own database handle rather than
    /// on the caller's. Scanning a large library takes seconds, and a frontend
    /// that did it inline would freeze for that long every time a file landed.
    /// This is what the WAL journal enabled in `db::sqlite` is for: the scan
    /// writes while the frontend keeps reading.
    ///
    /// Results arrive as ordinary events -- `ScanProgress` while it runs, then
    /// `LibraryChanged`. A frontend that already handles those needs no new
    /// code path; it does need to call [`Session::reload_index`] when one
    /// arrives, because the copy this session queries is its own.
    ///
    /// # Errors
    ///
    /// [`CoreError::NoLibraryRoot`] if no root is configured, or
    /// [`CoreError::Database`] if the watcher or the second database handle
    /// cannot be opened.
    pub fn start_watching(&mut self) -> Result<(), CoreError> {
        if self.watcher.is_some() {
            return Ok(());
        }

        let root = self.library_root()?;
        let watcher = LibraryWatcher::watch(&root)?;
        // A separate handle, not a shared one: the scan thread writes while the
        // frontend reads, which is the whole point of doing this elsewhere.
        let mut db = SqliteDatabase::open(&self.paths.database())?;
        let artwork = ArtworkCache::new(self.paths.artwork_cache());

        let stop = Arc::new(AtomicBool::new(false));
        let (rescan_tx, rescan_rx) = channel::<()>();
        let events = self.events.clone();
        let thread_stop = Arc::clone(&stop);

        let thread = std::thread::Builder::new()
            .name("scoria-scan".to_owned())
            .spawn(move || {
                // Scan once before watching, to catch up on whatever changed
                // while nothing was running. Watching only reports events it
                // was present for, so without this a library edited between
                // sessions would stay stale until it was edited again.
                //
                // Silent when it finds nothing: an unchanged library produces
                // no deltas and therefore no events.
                let mut scan_now = true;

                while !thread_stop.load(Ordering::Acquire) {
                    if !scan_now {
                        // Either the filesystem settled after a change, or
                        // someone asked for a scan outright (§5.2: an explicit
                        // rescan is always available).
                        let changed = watcher.next_change(WATCH_POLL).is_some();
                        let requested = rescan_rx.try_recv().is_ok();
                        if !changed && !requested {
                            continue;
                        }
                    }
                    scan_now = false;
                    // Drain any further requests that piled up, so a burst
                    // becomes one scan rather than a queue of them.
                    while rescan_rx.try_recv().is_ok() {}

                    let scanner = Scanner::new(&root).with_artwork(&artwork);
                    let mut report = |progress: ScanProgress| {
                        let _ = events.send(Event::ScanProgress {
                            seen: progress.seen,
                            total: progress.total,
                        });
                    };

                    match scanner.scan(&mut db, &mut report) {
                        Ok(outcome) => {
                            let deltas = outcome.deltas();
                            if deltas.is_empty() {
                                // Nothing changed after all -- a temporary file,
                                // or our own artwork cache being written. Saying
                                // so anyway would make the frontend reload the
                                // library for nothing.
                                continue;
                            }
                            for delta in deltas {
                                let _ = events.send(Event::LibraryChanged(delta));
                            }
                        }
                        Err(e) => {
                            let _ = events.send(Event::Error(e));
                        }
                    }
                }
            })
            .map_err(|e| CoreError::Database(format!("cannot start the scan thread: {e}")))?;

        self.watcher = Some(WatchHandle {
            stop,
            rescan: rescan_tx,
            thread: Some(thread),
        });
        Ok(())
    }

    /// Asks the watch thread to scan now.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if watching was never started or the thread has
    /// stopped.
    pub fn request_rescan(&self) -> Result<(), CoreError> {
        match &self.watcher {
            Some(handle) => handle
                .rescan
                .send(())
                .map_err(|_| CoreError::Database("the scan thread has stopped".to_owned())),
            None => Err(CoreError::Database(
                "library watching is not running".to_owned(),
            )),
        }
    }

    /// Re-reads the library index from storage.
    ///
    /// Call this on `Event::LibraryChanged`. The index is a projection of the
    /// database, and a background scan changes the database without touching
    /// this copy of it.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if storage cannot be read.
    pub fn reload_index(&mut self) -> Result<(), CoreError> {
        self.index = LibraryIndex::load(&self.db)?;

        // The audio thread resolves against its own copy, so it needs the new
        // one -- but only after the scan, so a rescan mid-album cannot disturb
        // what is playing (§6.1).
        if self.playback.is_some() {
            let root = self.library_root()?;
            let resolver = Arc::new(IndexResolver::new(self.index.clone(), root));
            self.send_to_engine(EngineCommand::SetResolver(resolver))?;
        }
        Ok(())
    }

    // ------------------------------------------------------------ commands --

    /// Applies a command.
    ///
    /// Failures are reported as [`Event::Error`] as well as returned, so a
    /// frontend that only watches the event stream still learns about them.
    ///
    /// # Errors
    ///
    /// Whatever the underlying operation failed with.
    pub fn handle(&mut self, command: Command) -> Result<(), CoreError> {
        let result = self.dispatch(command);
        if let Err(e) = &result {
            self.emit(Event::Error(e.clone()));
        }
        result
    }

    fn dispatch(&mut self, command: Command) -> Result<(), CoreError> {
        match command {
            Command::Play(id) => {
                let source = PlaybackSource::new(vec![id], Origin::Library);
                self.start(source)
            }
            Command::PlayFrom { source, at } => {
                let source = self.materialise(&source, at)?;
                self.start(source)
            }
            Command::Pause => self.send_to_engine(EngineCommand::Pause),
            Command::Resume => self.send_to_engine(EngineCommand::Resume),
            Command::Stop => self.send_to_engine(EngineCommand::Stop),
            Command::Next => self.send_to_engine(EngineCommand::Next),
            Command::Previous => self.send_to_engine(EngineCommand::Previous),
            Command::Seek(position) => self.send_to_engine(EngineCommand::Seek(position)),
            Command::SetVolume(v) => self.send_to_engine(EngineCommand::SetVolume(v)),
            Command::SetPosition(p) => self.send_to_engine(EngineCommand::SetPosition(p)),
            Command::SetShuffle(on) => self.send_to_engine(EngineCommand::SetShuffle(on)),
            Command::SetRepeat(mode) => self.send_to_engine(EngineCommand::SetRepeat(mode)),
            Command::Query(id, selection) => {
                let outcome = self.query(&selection)?;
                self.emit(Event::QueryResult(id, outcome.tracks));
                Ok(())
            }
            Command::Rescan => self.rescan(&mut |_| {}).map(|_| ()),
            Command::SetRoot(root) => self.set_root(root),
        }
    }

    /// Convenience for frontends: run a selection and get an id back for
    /// correlating the result.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the query fails.
    pub fn submit_query(&mut self, id: QueryId, selection: Selection) -> Result<(), CoreError> {
        self.handle(Command::Query(id, selection))
    }

    // ------------------------------------------------------------ playback --

    /// Which snapshot a click should produce, per the §6.3 setting.
    ///
    /// The one place that setting is consulted. Everything downstream is
    /// identical between the two modes, which is the point of it being one
    /// enum.
    #[must_use]
    pub fn source_spec(&self, shown: Vec<TrackId>, label: Option<String>) -> SourceSpec {
        match self.config.play_behaviour {
            PlayBehaviour::WholeLibrary => SourceSpec::Library {
                sort: self.config.sort.chain(),
            },
            PlayBehaviour::WhatsShown => SourceSpec::Shown {
                tracks: shown,
                label,
            },
        }
    }

    /// Turns a [`SourceSpec`] into the materialised snapshot that will play.
    ///
    /// §6.2: `tracks` is what plays and `origin` is a label. This is the only
    /// place a query becomes a play order, and once it has, nothing re-derives
    /// it -- which is invariant 5.
    fn materialise(&self, spec: &SourceSpec, at: TrackId) -> Result<PlaybackSource, CoreError> {
        let (tracks, origin) = match spec {
            SourceSpec::Library { sort } => (self.sorted_library(sort), Origin::Library),
            SourceSpec::Shown { tracks, label } => (
                tracks.clone(),
                label.clone().map_or(Origin::Library, Origin::Filtered),
            ),
            SourceSpec::Album(album) => {
                let selection = Selection {
                    albums: vec![*album],
                    ..Selection::all()
                };
                (self.query(&selection)?.tracks, Origin::Album(*album))
            }
        };

        if tracks.is_empty() {
            return Err(CoreError::EmptySource);
        }
        Ok(PlaybackSource::starting_at(tracks, origin, at))
    }

    /// The whole library in the given order. Ignores filters but respects the
    /// sort, since sort is something the user just deliberately set (§6.3).
    fn sorted_library(&self, sort: &SortChain) -> Vec<TrackId> {
        let mut tracks: Vec<&crate::model::Track> = self.index.tracks().iter().collect();
        tracks.sort_by(|a, b| sort.compare(a, b));
        tracks.iter().map(|t| t.id).collect()
    }

    fn start(&mut self, source: PlaybackSource) -> Result<(), CoreError> {
        self.ensure_playback()?;
        self.send_to_engine(EngineCommand::PlaySource(Box::new(source)))
    }

    fn send_to_engine(&mut self, command: EngineCommand) -> Result<(), CoreError> {
        self.ensure_playback()?;
        match &self.playback {
            Some(handle) => handle.send(command),
            None => Err(CoreError::Database("playback is not running".to_owned())),
        }
    }

    /// Starts the audio thread on first use.
    ///
    /// Deferred so that scanning or listing a library works on a machine with
    /// no sound card, which is an ordinary thing for a headless run or a CI
    /// container to be.
    fn ensure_playback(&mut self) -> Result<(), CoreError> {
        if self.playback.is_some() {
            return Ok(());
        }
        let backend = crate::playback::backend::RodioBackend::open()?;
        self.start_playback(Box::new(backend))
    }

    /// Starts the audio thread with a specific backend.
    ///
    /// Exists so a caller can supply a mock, which is how playback behaviour is
    /// tested without a device.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the thread cannot be started.
    pub fn start_playback(&mut self, backend: Box<dyn AudioBackend>) -> Result<(), CoreError> {
        let resolver = Arc::new(IndexResolver::new(self.index.clone(), self.library_root()?));
        self.playback = Some(spawn(backend, resolver, self.events.clone())?);
        Ok(())
    }

    fn library_root(&self) -> Result<PathBuf, CoreError> {
        self.config.root().map(Path::to_path_buf)
    }

    // ------------------------------------------------------------ library --

    /// Scans the library root and brings the index up to date.
    ///
    /// Blocking: §3.3 puts scanning on tokio, and the way to honour that is to
    /// call this from `spawn_blocking` rather than to dress a filesystem walk
    /// up as async.
    ///
    /// # Errors
    ///
    /// [`CoreError::NoLibraryRoot`] if no root is set, or
    /// [`CoreError::Database`] if storage fails.
    pub fn rescan(
        &mut self,
        progress: &mut dyn FnMut(ScanProgress),
    ) -> Result<ScanOutcome, CoreError> {
        let root = self.library_root()?;
        let mut report = |p: ScanProgress| {
            progress(p);
        };

        let outcome = Scanner::new(&root)
            .with_artwork(&self.artwork)
            .scan(&mut self.db, &mut report)?;

        self.index = LibraryIndex::load(&self.db)?;
        for delta in outcome.deltas() {
            self.emit(Event::LibraryChanged(delta));
        }
        if outcome.deltas().is_empty() {
            self.emit(Event::LibraryChanged(LibraryDelta::Invalidated));
        }

        // The audio thread resolves against its own copy, so it needs the new
        // one -- but only after the scan, so a rescan mid-album cannot disturb
        // what is playing (§6.1).
        if self.playback.is_some() {
            let resolver = Arc::new(IndexResolver::new(self.index.clone(), &root));
            self.send_to_engine(EngineCommand::SetResolver(resolver))?;
        }
        Ok(outcome)
    }

    /// Points the library at a new root and rescans.
    ///
    /// # Errors
    ///
    /// [`CoreError::Config`] if settings cannot be written, or
    /// [`CoreError::Database`] if the scan fails.
    pub fn set_root(&mut self, root: PathBuf) -> Result<(), CoreError> {
        self.config.root = Some(root);
        crate::config::save(&self.paths.config_file(), &self.config)?;
        self.rescan(&mut |_| {}).map(|_| ())
    }

    /// Persists the current settings.
    ///
    /// # Errors
    ///
    /// [`CoreError::Config`] if the file cannot be written.
    pub fn save_config(&self) -> Result<(), CoreError> {
        crate::config::save(&self.paths.config_file(), &self.config)
    }

    /// Settings, for a frontend to adjust before saving.
    pub fn config_mut(&mut self) -> &mut LibraryConfig {
        &mut self.config
    }

    /// The database, for a frontend that needs repository access directly.
    ///
    /// Full-text search goes through here, since the index cannot answer it.
    #[must_use]
    pub fn database(&self) -> &impl TrackRepository {
        &self.db
    }
}
