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
use crate::id::{QueryId, TrackId};
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
use std::sync::mpsc::{Receiver, Sender, channel};

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
    /// # Errors
    ///
    /// [`CoreError::NoLibraryRoot`] if no root is configured, or
    /// [`CoreError::Database`] if the platform watcher cannot be created.
    pub fn watch(&self) -> Result<LibraryWatcher, CoreError> {
        LibraryWatcher::watch(self.config.root()?)
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
