//! The application: state, messages, update, view.
//!
//! Everything here talks to the core by sending `Command` and reading `Event`
//! (`DESIGN.md` §3.2). **No function in this file calls a method on a library,
//! database or playback object** -- that is `CLAUDE.md` invariant 2, and the
//! way to tell it has been broken is a view or handler reaching for
//! `self.library.something()`.
//!
//! `Session` is the one thing held, and it only accepts commands.

use crate::keymap::{Action, Keymap};
use crate::layouts::{self, Browser, BrowserPlacement};
use crate::overlays::{self, Overlay};
use crate::selection::{RowSelection, SortOverride};
use crate::settings::Settings;
use crate::theme::Theme;
use crate::widgets::columns::Column;
use crate::widgets::{menu, panes, table, transport, welcome};
use iced::widget::{Space, button, container, row, text, text_input};
use iced::{Element, Fill, Length, Subscription, Task};
use scoria_core::command::Command;
use scoria_core::config::Paths;
use scoria_core::event::{Event as CoreEvent, PlaybackState};
use scoria_core::id::TrackId;
use scoria_core::model::Track;
use scoria_core::playback::RepeatMode;
use scoria_core::query::Selection;
use scoria_core::session::Session;
use std::time::Duration;

/// How often the core's event stream is drained.
///
/// A poll rather than a subscription over the channel: `Session` owns the
/// receiver, and handing it to a subscription would mean the state no longer
/// owns its own event stream.
///
/// This is also the seek bar's frame rate, since the bar redraws when a
/// position event is handled. Twenty a second is smooth to the eye and costs a
/// redraw of a virtualised table, which is a few dozen rows.
const POLL: Duration = Duration::from_millis(25);

const SEARCH_ID: &str = "search";
/// The table's scrollable, so the cursor can drag the view with it.
const TABLE_ID: &str = "tracks";
const ARTIST_PANE_ID: &str = "artists";
const ALBUM_PANE_ID: &str = "albums";
const QUEUE_ID: &str = "queue";

#[derive(Debug, Clone)]
pub enum Message {
    /// Drain whatever the core has produced.
    Tick,
    Table(table::Event),
    Transport(transport::Event),
    ArtistPane(panes::Event),
    AlbumPane(panes::Event),
    ClearFilters,
    DrawerDragStart,
    DrawerDragTo(f32),
    DrawerDragEnd,
    Queue(table::Event),
    Columns(overlays::columns::Event),
    Settings(overlays::settings::Event),
    Welcome(welcome::Event),
    /// Put the browser here (§7.1).
    SetPlacement(BrowserPlacement),
    /// The folder picker finished. `None` means it was cancelled.
    RootChosen(Option<std::path::PathBuf>),
    CloseOverlay,
    SearchChanged(String),
    /// Something chosen from a track's right-click menu.
    TrackMenu(menu::TrackAction),
    /// Something chosen from the header's overflow menu.
    HeaderMenu(menu::HeaderAction),
    /// Open the header's overflow menu.
    OpenHeaderMenu,
    /// Dismiss whichever menu is open.
    CloseMenu,
    /// A key press, already resolved through the keymap.
    Action(Action),
    /// Which modifiers are held. Pointer events do not carry them, so a click
    /// cannot tell plain from shift from control without this.
    Modifiers(crate::keymap::Mods),
}

/// Everything the window shows.
pub struct App {
    /// `None` when the library could not be opened. The window still opens and
    /// says why: §5.1 makes first run a "point me at your library" moment, and
    /// a failure to read the database is the same shape of problem.
    session: Option<Session>,
    /// Why there is no session, when there is not one. What the explorer says
    /// in place of the track list (§5.1).
    problem: Option<String>,
    theme: Theme,
    keymap: Keymap,
    placement: BrowserPlacement,
    /// What persists between runs: theme, placement, columns, sort, drawer
    /// heights.
    settings: Settings,
    paths: Paths,
    /// §7.3: a header click overrides the configured sort transiently, with an
    /// indicator and one action to drop back. It never replaces the chain.
    sort_override: SortOverride,
    /// The column edge being dragged, if any.
    resizing_column: Option<usize>,
    /// The heading being dragged to a new position, if any (§7.3).
    reordering: Option<table::Reorder>,

    /// The current query and its result. `rows` is the ordered id list; the
    /// tracks themselves are looked up through the session's index.
    selection: Selection,
    rows: Vec<TrackId>,
    lines: Vec<table::Line>,
    search: String,
    /// The panes, as the last query computed them. §7.2: contextual counts are
    /// the query engine's answer, not something the view recomputes.
    artists: Vec<panes::PaneRow>,
    artist_total: usize,
    /// The album pane's rows, and the ids they stand for, in step.
    ///
    /// Two lists rather than a list of pairs so the rows can be handed to the
    /// pane as a slice. Albums are filtered by id -- two can share a title --
    /// but the pane draws only labels and counts, and it draws them every
    /// frame.
    albums: Vec<panes::PaneRow>,
    album_ids: Vec<scoria_core::AlbumId>,
    album_total: usize,
    artist_scroll: f32,
    album_scroll: f32,

    /// The row the keyboard is on and the rows selected. See
    /// [`crate::selection::RowSelection`], which holds the rules.
    selection_rows: RowSelection,
    /// Modifiers currently held, so a click can tell plain from shift from
    /// control. Pointer events do not carry them.
    modifiers: crate::keymap::Mods,

    playing: Option<TrackId>,
    /// The playing track's artwork, and which track it was found for.
    ///
    /// Looking it up stats half a dozen candidate filenames, and `view` wants
    /// it twice -- the transport thumbnail and, in `Top` placement, the
    /// browser -- on every one of the forty frames a second the poll drives.
    /// Artwork follows what is playing (§9.1), so the track id is the whole
    /// key.
    artwork: Option<(Option<TrackId>, Option<std::path::PathBuf>)>,
    state: PlaybackState,
    position: Duration,
    /// Set while the seek bar is being dragged. See `TransportView::scrubbing`.
    scrubbing: Option<Duration>,
    shuffled: bool,
    repeat: RepeatMode,
    volume: f32,
    /// Set when the volume has moved since it was last written out, and when
    /// it last moved. See `save_volume_if_settled`.
    volume_unsaved: bool,
    volume_changed_at: Option<std::time::Instant>,

    scroll_offset: f32,
    viewport_height: f32,
    status: String,
    /// A scan in flight, as the last `ScanProgress` reported it.
    ///
    /// Held rather than left to the status line, because on first run it is the
    /// difference between "this is taking a while" and "this found nothing",
    /// and the two need different words.
    scanning: Option<welcome::Scan>,
    /// The settings drawer's Missing row has been pressed once and is asking
    /// to be sure (§7.4). Anything else that happens disarms it.
    forget_armed: bool,
    /// Likewise the Loudness row (§6.7).
    gain_armed: bool,
    /// A loudness analysis this frontend started, as its last progress
    /// report put it: albums done, albums in all, the last one finished.
    gain_progress: Option<(usize, usize, String)>,
    /// What the last analysis could not do, relative to the root, kept until
    /// the next one starts.
    gain_failures: Vec<(String, String)>,
    /// What an analysis would do now. Worked out when it could have changed
    /// rather than in the view, because it walks the whole library.
    loudness_counts: overlays::settings::LoudnessCounts,
    /// Whether the library root was on disk when last looked at.
    ///
    /// Cached rather than asked in the view, which runs every frame and should
    /// not be touching the filesystem. Looked at again whenever the answer
    /// could matter: the drawer opening, the library changing, a press.
    root_reachable: bool,

    /// Which overlay is open. One `Option`, so §9's "one at a time, never
    /// stacked" cannot be expressed otherwise.
    /// What is on screen, including anything still retracting.
    overlay: Option<overlays::Presenting>,
    dragging_drawer: bool,
    /// The playback snapshot, as the engine last reported it. This is what the
    /// queue shows -- §6.2's `tracks`, not the current view.
    queue: Option<scoria_core::PlaybackSource>,
    /// Tracks that were forgotten (§7.4) while the snapshot still held them.
    ///
    /// The snapshot is never rebuilt (§6.1), so their ids stay in the queue
    /// and are skipped -- but the index no longer knows them, and a row the
    /// table cannot look up is drawn blank. Kept here, still marked missing,
    /// so the queue goes on showing what it showed a moment earlier. Pruned
    /// whenever the snapshot changes.
    forgotten: std::collections::HashMap<TrackId, Track>,
    queue_lines: Vec<table::Line>,
    queue_scroll: f32,
    /// The row the queue's own cursor is on, independent of what is playing.
    ///
    /// A set of one, because the table it is drawn with takes a selection.
    queue_selection: std::collections::BTreeSet<usize>,
    /// The open menu, if there is one.
    ///
    /// Not an `Overlay`. §9's overlays are a family of drawers with one set of
    /// rules -- they rise from the bottom, animate, resize by dragging and
    /// never cover the transport -- and a menu at the pointer shares none of
    /// it. Folding it in would mean those rules stopped describing the family.
    menu: Option<OpenMenu>,
    /// The file's own tags, read when the properties overlay opens.
    properties: Vec<(String, String)>,
    properties_problem: Option<String>,
}

impl App {
    /// Opens the library and runs the first query.
    ///
    /// # Errors
    ///
    /// Whatever opening the session failed with.
    pub fn new() -> Result<Self, scoria_core::error::CoreError> {
        let paths = Paths::from_env()?;
        let settings = Settings::load(&paths)?;
        let session = Session::open(paths.clone())?;

        let mut app = Self::with_session(Some(session), String::new());
        app.theme = settings.theme();
        app.placement = settings.placement;
        // Shown from the settings straight away. The engine is not running
        // yet -- it starts on the first play -- and applies the same value
        // itself when it does, so the bar and the audio agree without the
        // frontend having to send anything.
        app.volume = app.session.as_ref().map_or(1.0, |s| s.config().volume());
        app.selection.sort = settings.sort.chain();
        app.paths = paths;
        app.settings = settings;
        app.refresh();
        app.begin_watching();
        app.begin_media_controls();
        Ok(app)
    }

    /// A window with no library behind it, showing why.
    #[must_use]
    pub fn disconnected(reason: String) -> Self {
        Self::with_session(None, reason)
    }

    fn with_session(session: Option<Session>, status: String) -> Self {
        Self {
            problem: session.is_none().then(|| status.clone()),
            session,
            theme: Theme::dark(),
            keymap: Keymap::standard(),
            placement: BrowserPlacement::default(),
            settings: Settings::default(),
            paths: Paths::rooted_at(std::env::temp_dir().join("scoria-unconfigured")),
            sort_override: SortOverride::default(),
            resizing_column: None,
            reordering: None,
            selection: Selection::all(),
            rows: Vec::new(),
            lines: Vec::new(),
            search: String::new(),
            artists: Vec::new(),
            artist_total: 0,
            albums: Vec::new(),
            album_ids: Vec::new(),
            album_total: 0,
            artist_scroll: 0.0,
            album_scroll: 0.0,
            selection_rows: RowSelection::default(),
            modifiers: crate::keymap::Mods::NONE,
            playing: None,
            artwork: None,
            state: PlaybackState::Stopped,
            position: Duration::ZERO,
            scrubbing: None,
            shuffled: false,
            repeat: RepeatMode::Off,
            volume: 1.0,
            volume_unsaved: false,
            volume_changed_at: None,
            scroll_offset: 0.0,
            viewport_height: 650.0,
            status,
            scanning: None,
            forget_armed: false,
            gain_armed: false,
            gain_progress: None,
            gain_failures: Vec::new(),
            loudness_counts: overlays::settings::LoudnessCounts::default(),
            root_reachable: true,
            overlay: None,
            dragging_drawer: false,
            queue: None,
            forgotten: std::collections::HashMap::new(),
            queue_lines: Vec::new(),
            queue_scroll: 0.0,
            queue_selection: std::collections::BTreeSet::new(),
            menu: None,
            properties: Vec::new(),
            properties_problem: None,
        }
    }

    /// Starts watching the library, if there is one to watch.
    ///
    /// A missing root is the ordinary first-run state (§5.1), not a failure, so
    /// it is reported in the status line rather than treated as an error.
    fn begin_watching(&mut self) {
        let Some(session) = &mut self.session else {
            return;
        };
        match session.start_watching() {
            // Watching begins with a catch-up scan (§5.2), so a library is
            // being read from this moment -- said now rather than when the
            // first progress arrives, or an empty library would claim to have
            // found nothing before it had looked.
            Ok(()) => {
                self.scanning = Some(welcome::Scan {
                    seen: 0,
                    total: None,
                })
            }
            // The ordinary first-run state, not a failure. The explorer asks
            // for a folder; saying anything here would be a second, quieter
            // copy of that.
            Err(scoria_core::error::CoreError::NoLibraryRoot) => {}
            Err(e) => self.status = format!("cannot watch the library: {e}"),
        }
    }

    /// Connects to the desktop's media controls (§12).
    ///
    /// A desktop with no MPRIS support, or no session bus at all, is a normal
    /// thing to run on: the failure is noted and the player carries on without
    /// media keys rather than refusing to start.
    fn begin_media_controls(&mut self) {
        let Some(session) = &mut self.session else {
            return;
        };
        if let Err(e) = session.start_media_controls() {
            self.status = format!("media keys unavailable: {e}");
        }
    }

    /// Re-runs the current selection.
    ///
    /// Goes through the session, which returns the ordered ids and the derived
    /// groups; the table is rebuilt from those rather than from anything the
    /// view reaches for itself.
    fn refresh(&mut self) {
        let Some(session) = &self.session else { return };
        match session.query(&self.selection) {
            Ok(outcome) => {
                self.lines = table::build_lines(outcome.tracks.len(), &outcome.groups);
                self.rows = outcome.tracks;

                self.artist_total = outcome.album_artists.total;
                self.artists = outcome
                    .album_artists
                    .rows
                    .into_iter()
                    .map(|r| panes::PaneRow {
                        label: r.label,
                        count: r.count,
                        selected: r.selected,
                    })
                    .collect();

                self.album_total = outcome.albums.total;
                self.album_ids = outcome.albums.rows.iter().map(|r| r.id).collect();
                self.albums = outcome
                    .albums
                    .rows
                    .into_iter()
                    .map(|r| panes::PaneRow {
                        label: r.label,
                        count: r.count,
                        selected: r.selected,
                    })
                    .collect();
                self.status = format!(
                    "{} of {} tracks{}",
                    self.rows.len(),
                    outcome.total,
                    if self.selection.is_filtered() {
                        " (filtered)"
                    } else {
                        ""
                    }
                );
                // Row indices refer to the previous result, so anything past
                // the end means nothing now.
                self.selection_rows.retain_within(self.rows.len());
            }
            Err(e) => self.status = format!("query failed: {e}"),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                self.drain_core_events();
                self.refresh_artwork();
                self.tick_overlay();
                self.save_volume_if_settled();
            }
            Message::Table(event) => self.on_table(event),
            Message::Transport(event) => return self.on_transport(event),
            Message::SearchChanged(query) => {
                self.search = query;
                self.selection.search = if self.search.is_empty() {
                    None
                } else {
                    Some(self.search.clone())
                };
                self.scroll_offset = 0.0;
                self.refresh();
            }
            Message::ClearFilters => self.clear_filters(),
            Message::DrawerDragStart => self.dragging_drawer = true,
            Message::DrawerDragTo(fraction) => {
                if let Some(state) = self.overlay {
                    self.settings
                        .drawer_heights
                        .insert(state.overlay.title().to_owned(), fraction);
                }
            }
            Message::DrawerDragEnd => {
                self.dragging_drawer = false;
                // On release rather than during: a drag is hundreds of pointer
                // events and one outcome.
                self.persist();
            }
            Message::Queue(event) => return self.on_queue(event),
            Message::Columns(event) => self.on_columns(event),
            Message::Settings(event) => return self.on_settings(event),
            Message::Welcome(event) => return self.on_welcome(event),
            Message::SetPlacement(placement) => self.set_placement(placement),
            Message::RootChosen(root) => self.set_root(root),
            Message::CloseOverlay => self.set_overlay(None),
            Message::ArtistPane(event) => self.on_artist_pane(event),
            Message::AlbumPane(event) => self.on_album_pane(event),
            Message::TrackMenu(action) => return self.on_track_menu(action),
            Message::HeaderMenu(action) => return self.on_header_menu(action),
            Message::OpenHeaderMenu => {
                self.menu = Some(OpenMenu::Header { at: pointer() });
            }
            Message::CloseMenu => self.menu = None,
            Message::Action(action) => return self.on_action(action),
            Message::Modifiers(modifiers) => self.modifiers = modifiers,
        }
        Task::none()
    }

    /// Reads the core's event stream. The only direction information flows in.
    fn drain_core_events(&mut self) {
        let Some(session) = &mut self.session else {
            return;
        };
        // Mutable because this is also where the desktop's media controls are
        // served: what they asked for is applied, and what is playing is
        // published back to them.
        let events = session.drain_events();
        let mut library_changed = None;
        for event in events {
            match event {
                CoreEvent::TrackChanged { track, .. } => {
                    self.playing = Some(track);
                    self.position = Duration::ZERO;
                    // A track change mid-drag means the drag refers to a track
                    // that is no longer playing.
                    self.scrubbing = None;
                }
                CoreEvent::PositionChanged(position) => {
                    // Ignored while dragging: the handle belongs to the pointer
                    // until it is released.
                    if self.scrubbing.is_none() {
                        self.position = position;
                    }
                }
                CoreEvent::PlaybackStateChanged(state) => {
                    self.state = state;
                    if state == PlaybackState::Stopped {
                        self.playing = None;
                    }
                }
                CoreEvent::QueueChanged(source) => {
                    self.shuffled = source.is_shuffled();
                    self.repeat = source.repeat();
                    self.queue_lines = table::build_lines(source.len(), &[]);
                    self.forgotten.retain(|id, _| source.tracks().contains(id));
                    self.queue = Some(source);
                }
                CoreEvent::LibraryChanged(delta) => {
                    // The scan happened on another thread against its own
                    // database handle, so this session's index is a stale copy
                    // until it is told to re-read.
                    library_changed = Some(delta);
                }
                CoreEvent::ScanProgress { seen, total } => {
                    // Forgetting or analysing mid-scan would race the scan,
                    // so an armed row stands down.
                    self.disarm();
                    self.scanning = Some(welcome::Scan { seen, total });
                    self.status = match total {
                        Some(total) => format!("scanning {seen}/{total}"),
                        None => format!("scanning {seen}"),
                    };
                }
                // The scan stopped, whether or not it found anything. Without
                // this the header would still read "scanning 338/338" an hour
                // later, and the first-run panel would still be waiting.
                CoreEvent::ScanFinished => {
                    self.scanning = None;
                    self.status = String::new();
                }
                CoreEvent::Error(e) => self.status = format!("{e}"),
                CoreEvent::GainProgress { done, total, album } => {
                    self.status = format!("levelling {done}/{total}");
                    self.gain_progress = Some((done, total, album));
                }
                CoreEvent::GainFinished(report) => {
                    self.gain_progress = None;
                    self.status = gain_summary(&report);
                    let root = self.session.as_ref().and_then(|s| s.config().root.clone());
                    self.gain_failures = report
                        .failed
                        .into_iter()
                        .map(|(path, reason)| (relative_to(&path, root.as_deref()), reason))
                        .collect();
                    // The new tags reach the index by the scan that follows,
                    // and `on_library_changed` counts again then; this catches
                    // an analysis that wrote nothing.
                    self.refresh_library_facts();
                }
            }
        }

        // Done after the drain rather than inside it, so a scan reporting three
        // deltas reloads once instead of three times.
        if let Some(delta) = library_changed {
            self.on_library_changed(&delta);
        }
    }

    /// Brings the view back in line with the library after a background scan.
    fn on_library_changed(&mut self, delta: &scoria_core::event::LibraryDelta) {
        let Some(session) = &mut self.session else {
            return;
        };
        if let Err(e) = session.reload_index() {
            self.status = format!("{e}");
            return;
        }

        // Keep the cursor on the same track rather than the same row number:
        // a scan can insert tracks above it, and holding the index would move
        // the selection to whatever landed there.
        // A scan caches artwork as it goes, so an album that had none may
        // have some now.
        self.artwork = None;

        let cursor = self.cursor();
        let anchored = cursor.and_then(|index| self.rows.get(index).copied());
        self.refresh();

        // Forgetting can leave a pane filter naming an artist or album that no
        // longer exists, and an empty table under a chip for something gone
        // reads as a fault. Only for forgetting: a search that matches nothing
        // is an answer, and so is a filter after any other change.
        if matches!(delta, scoria_core::event::LibraryDelta::TracksRemoved(_))
            && self.rows.is_empty()
        {
            self.clear_filters();
        }

        let survived = anchored.and_then(|track| self.rows.iter().position(|id| *id == track));
        if let Some(index) = refocus(survived, cursor, self.rows.len()) {
            self.selection_rows.focus(index, self.rows.len());
        }

        self.disarm();
        self.refresh_library_facts();
        self.status = describe(delta, self.rows.len());
    }

    /// What the settings drawer's Missing row should offer (§7.4).
    fn missing(&self) -> overlays::settings::Missing {
        let count = self.session.as_ref().map_or(0, |session| {
            session
                .index()
                .tracks()
                .iter()
                .filter(|t| !t.available)
                .count()
        });
        missing_state(
            count,
            self.root_reachable,
            self.scanning.is_some(),
            self.forget_armed,
        )
    }

    /// Stands down any armed question in the settings drawer, saying whether
    /// there was one.
    fn disarm(&mut self) -> bool {
        let was = self.forget_armed || self.gain_armed;
        self.forget_armed = false;
        self.gain_armed = false;
        was
    }

    /// Looks again at what the settings drawer's upkeep rows depend on.
    fn refresh_library_facts(&mut self) {
        self.refresh_reachability();
        self.loudness_counts = self
            .session
            .as_ref()
            .and_then(|session| session.gain_plan(false).ok())
            .map(|plan| overlays::settings::LoudnessCounts {
                levelled: plan.tracks_levelled,
                total: plan.tracks_total,
                albums: plan.albums.len(),
                files: plan.files(),
                skipped_missing: plan.skipped.missing_files,
                skipped_unsupported: plan.skipped.unsupported,
            })
            .unwrap_or_default();
    }

    /// What the settings drawer's Loudness row should offer (§6.7).
    fn loudness(&self) -> overlays::settings::Loudness {
        loudness_state(
            self.loudness_counts,
            self.root_reachable,
            self.scanning.is_some(),
            self.gain_armed,
            self.gain_progress.clone(),
        )
    }

    /// Arms the Loudness row, if pressing it would be right.
    fn arm_gain(&mut self) {
        self.refresh_library_facts();
        self.gain_armed = matches!(self.loudness(), overlays::settings::Loudness::Ready(_));
    }

    /// The second press (§6.7). Re-checks, because a scan can have started in
    /// between.
    fn start_gain(&mut self) {
        let armed = std::mem::take(&mut self.gain_armed);
        self.refresh_reachability();
        if !armed || self.scanning.is_some() || !self.root_reachable {
            return;
        }
        let Some(session) = &mut self.session else {
            return;
        };
        match session.start_gain_analysis(false) {
            Ok(total) => {
                self.gain_failures.clear();
                self.gain_progress = Some((0, total, String::new()));
                self.status = format!("levelling 0/{total}");
            }
            Err(e) => self.status = format!("Could not start the analysis: {e}"),
        }
    }

    fn refresh_reachability(&mut self) {
        self.root_reachable = self
            .session
            .as_ref()
            .is_some_and(Session::library_root_reachable);
    }

    /// Arms the Missing row, if pressing it would be right.
    fn arm_forget(&mut self) {
        self.refresh_reachability();
        self.forget_armed = matches!(self.missing(), overlays::settings::Missing::Ready(_));
    }

    /// The second press (§7.4). Re-checks everything the first press did,
    /// because a scan can have started in between.
    fn forget_missing(&mut self) {
        let armed = std::mem::take(&mut self.forget_armed);
        self.refresh_reachability();
        if !armed || self.scanning.is_some() {
            return;
        }
        let Some(session) = &mut self.session else {
            return;
        };

        // Copied before they go, for the queue's sake: see `forgotten`.
        let queued: std::collections::HashSet<TrackId> = self
            .queue
            .as_ref()
            .map(|q| q.tracks().iter().copied().collect())
            .unwrap_or_default();
        let keep: Vec<Track> = session
            .index()
            .tracks()
            .iter()
            .filter(|t| !t.available && queued.contains(&t.id))
            .cloned()
            .collect();

        // On success the status comes from the TracksRemoved this emits, by
        // way of `on_library_changed`, so it is said in one place.
        match session.forget_unavailable() {
            Ok(forgotten) if forgotten.tracks == 0 => {
                "No missing tracks.".clone_into(&mut self.status);
            }
            Ok(_) => self.forgotten.extend(keep.into_iter().map(|t| (t.id, t))),
            Err(e) => self.status = format!("Could not forget missing tracks: {e}"),
        }
    }

    fn on_table(&mut self, event: table::Event) {
        match event {
            table::Event::Select(index) => self.select(index, self.modifiers),
            table::Event::Activate(index) => {
                self.selection_rows.focus(index, self.rows.len());
                self.play_from_cursor();
            }
            table::Event::Context(index) => {
                // A right-click on an unselected row moves the selection to it;
                // on a selected one it leaves the selection alone, so acting on
                // several rows is not lost to a stray click.
                if !self.selection_rows.rows().contains(&index) {
                    self.select(index, crate::keymap::Mods::NONE);
                }
                // §9.1's escape hatch from artwork following playback is now
                // one item in here rather than the whole gesture, which is
                // what gives §9.3's metadata a mouse at all.
                if let Some(track) = self.rows.get(index).copied() {
                    self.menu = Some(OpenMenu::Track {
                        track,
                        at: pointer(),
                    });
                }
            }
            table::Event::Scrolled { offset, viewport } => {
                // A *real* scroll dismisses the menu, because the row it was
                // opened on has moved out from under it. Being told the offset
                // it already had does not: a relayout can report that, and the
                // menu vanishing because the table was rebuilt underneath it
                // is precisely how this used to fail.
                if (offset - self.scroll_offset).abs() > f32::EPSILON
                    && matches!(self.menu, Some(OpenMenu::Track { .. }))
                {
                    // Only the track menu is anchored to a row. The header's
                    // is not attached to anything that scrolled.
                    self.menu = None;
                }
                self.scroll_offset = offset;
                self.viewport_height = viewport;
            }
            table::Event::SortBy(column) => self.sort_by(column),
            // §7.3: configured from a right-click on the header.
            table::Event::HeadingMenu => {
                self.set_overlay(overlays::toggle(self.open_overlay_kind(), Overlay::Columns));
            }
            table::Event::ResizeStart(index) => self.resizing_column = Some(index),
            table::Event::ResizeTo(fraction) => {
                if let Some(index) = self.resizing_column {
                    self.settings.columns.drag_edge(index, fraction);
                }
            }
            table::Event::ResizeEnd => {
                self.resizing_column = None;
                // Saved on release rather than on every pointer move: a drag is
                // hundreds of events and one outcome.
                self.persist();
            }
            // A press on a heading. Whether it was a click or the start of a
            // drag is not known yet -- see `ReorderEnd`.
            table::Event::ReorderStart(index) => {
                self.reordering = Some(table::Reorder {
                    from: index,
                    to: index,
                });
            }
            table::Event::ReorderTo(fraction) => {
                if let Some(reorder) = &mut self.reordering {
                    reorder.to = self.settings.columns.visible_at(fraction);
                }
            }
            table::Event::ReorderEnd => {
                let Some(reorder) = self.reordering.take() else {
                    return;
                };
                // Let go where it started: that was a click, and a click on a
                // heading sorts by it. Anything else is a move.
                if reorder.to == reorder.from {
                    let clicked = self
                        .settings
                        .columns
                        .visible()
                        .nth(reorder.from)
                        .map(|setting| setting.column);
                    if let Some(column) = clicked {
                        self.sort_by(column);
                    }
                } else {
                    self.settings.columns.move_visible(reorder.from, reorder.to);
                    self.persist();
                }
            }
        }
    }

    /// The settings drawer (§9.4).
    fn on_settings(&mut self, event: overlays::settings::Event) -> Task<Message> {
        use overlays::settings::Event as E;
        match event {
            E::ChooseRoot => return self.choose_root(),
            E::SetPlayBehaviour(behaviour) => {
                if let Some(session) = &mut self.session
                    && let Err(e) = session.set_play_behaviour(behaviour)
                {
                    self.status = format!("{e}");
                }
            }
            E::ChooseTheme(name) => {
                self.theme = Theme::all()
                    .into_iter()
                    .find(|t| t.name == name)
                    .unwrap_or_default();
                self.settings.theme = self.theme.name.clone();
                self.persist();
            }
            E::Rescan => self.rescan(),
            E::ForgetMissing => self.arm_forget(),
            E::ConfirmForget => self.forget_missing(),
            E::CancelForget => self.forget_armed = false,
            E::AnalyseGain => self.arm_gain(),
            E::ConfirmAnalyse => self.start_gain(),
            E::CancelAnalyse => self.gain_armed = false,
            E::StopAnalyse => {
                if let Some(session) = &self.session {
                    session.stop_gain_analysis();
                    "stopping after the current track…".clone_into(&mut self.status);
                }
            }
        }
        Task::none()
    }

    /// The first-run panel (§5.1).
    ///
    /// Both of its actions exist in the settings drawer too, and both come
    /// through the same methods -- there is one way to choose a folder and one
    /// way to ask for a scan, whichever screen asked.
    fn on_welcome(&mut self, event: welcome::Event) -> Task<Message> {
        match event {
            welcome::Event::ChooseRoot => return self.choose_root(),
            welcome::Event::Rescan => self.rescan(),
        }
        Task::none()
    }

    /// Opens the folder chooser, starting somewhere useful.
    fn choose_root(&self) -> Task<Message> {
        let current = self.session.as_ref().and_then(|s| s.config().root.clone());
        Task::perform(
            crate::picker::choose_folder(crate::picker::suggested_start(current)),
            Message::RootChosen,
        )
    }

    /// Asks the scan thread to look again.
    fn rescan(&mut self) {
        match self.session.as_ref().map(Session::request_rescan) {
            Some(Ok(())) => {
                self.scanning = Some(welcome::Scan {
                    seen: 0,
                    total: None,
                });
                self.status = "rescanning…".to_owned();
            }
            Some(Err(e)) => self.status = format!("{e}"),
            None => {}
        }
    }

    /// Points the library somewhere else and starts watching it.
    fn set_root(&mut self, root: Option<std::path::PathBuf>) {
        let Some(root) = root else { return };
        let Some(session) = &mut self.session else {
            return;
        };

        match session.set_root(root) {
            // Watching restarts against the new root, and the catch-up scan it
            // performs does the indexing on the scan thread. Marked as scanning
            // from here rather than from the first progress event, so the
            // moment between choosing a folder and reading it does not look
            // like an empty one.
            Ok(()) => {
                self.scanning = Some(welcome::Scan {
                    seen: 0,
                    total: None,
                });
                self.status = "scanning the new library…".to_owned();
            }
            Err(e) => self.status = format!("{e}"),
        }
    }

    /// Writes the volume out once it has stopped moving.
    ///
    /// A drag across the bar is hundreds of changes and one outcome, and the
    /// wheel has no release to hang the write on -- so instead of saving per
    /// change, a tick that finds the volume stale for long enough saves it
    /// once. The delay is short enough that closing the window straight after
    /// a change keeps it.
    fn save_volume_if_settled(&mut self) {
        const SETTLE: Duration = Duration::from_millis(400);

        if !self.volume_unsaved {
            return;
        }
        let moved_recently = self
            .volume_changed_at
            .is_some_and(|at| at.elapsed() < SETTLE);
        if moved_recently {
            return;
        }

        self.volume_unsaved = false;
        if let Some(session) = &self.session
            && let Err(e) = session.save_config()
        {
            self.status = format!("cannot save settings: {e}");
        }
    }

    /// Puts the browser somewhere (§7.1).
    fn set_placement(&mut self, placement: BrowserPlacement) {
        self.placement = placement;
        // `persist` copies the placement across itself, so there is one place
        // that knows how a placement reaches the settings file.
        self.persist();
    }

    /// Column visibility, order, and the sort preset (§7.3).
    fn on_columns(&mut self, event: overlays::columns::Event) {
        use overlays::columns::Event as E;
        match event {
            E::ChooseSort(preset) => {
                self.settings.sort = preset;
                // Choosing a preset clears the transient override, or a heading
                // would keep quietly winning over the thing just picked.
                self.sort_override.clear();
                self.persist();
                self.apply_sort();
            }
            E::ClearOverride => {
                self.sort_override.clear();
                self.apply_sort();
            }
            E::ToggleColumn(column) => {
                self.settings.columns.toggle(column);
                self.persist();
            }
            E::MoveColumn(column, delta) => {
                self.settings.columns.reorder(column, isize::from(delta));
                self.persist();
            }
        }
    }

    /// §7.3: a header click applies a transient override on top of the
    /// configured sort, with a visible indicator and one action to drop back.
    /// It never replaces the configured chain.
    fn sort_by(&mut self, column: Column) {
        self.sort_override.click(column.sort_field());
        self.apply_sort();
    }

    /// Rebuilds the sort chain from the configured preset and any override.
    fn apply_sort(&mut self) {
        self.selection.sort = self.sort_override.over(&self.settings.sort.chain());
        self.refresh();
    }

    /// Writes the frontend's settings, reporting rather than failing.
    fn persist(&mut self) {
        self.settings.placement = self.placement;
        if let Err(e) = self.settings.save(&self.paths) {
            self.status = format!("cannot save settings: {e}");
        }
    }

    /// A track's right-click menu (§9.6).
    ///
    /// Every item is something the application could already do and a mouse
    /// could not reach. Nothing here is new behaviour; it is the door.
    fn on_track_menu(&mut self, event: menu::TrackAction) -> Task<Message> {
        let Some(OpenMenu::Track { track, .. }) = self.menu.take() else {
            return Task::none();
        };
        let Some(album) = self.track(track).map(|t| t.album_id) else {
            return Task::none();
        };

        match event {
            // §6.4: playing an album snapshots just that album. The core has
            // always handled this and nothing has ever asked for it.
            menu::TrackAction::PlayAlbum => {
                self.send(Command::PlayFrom {
                    source: scoria_core::SourceSpec::Album(album),
                    at: track,
                });
            }
            // Invariant 6's one exception to artwork following playback.
            menu::TrackAction::ShowArtwork => self.set_overlay(Some(Overlay::Artwork(Some(album)))),
            // §9.3, and the reason this menu exists.
            menu::TrackAction::Properties => return self.open_overlay(Overlay::Properties(track)),
            menu::TrackAction::FilterArtist => {
                if let Some(artist) = self.track(track).map(|t| t.album_artist.clone()) {
                    self.selection.album_artists = vec![artist];
                    // §7.2's cascade runs one way: choosing an artist resets
                    // what is below it.
                    self.selection.albums.clear();
                    self.reset_view();
                }
            }
            menu::TrackAction::FilterAlbum => {
                self.selection.albums = vec![album];
                self.reset_view();
            }
            // §7.4: one confirmation, and it lives in the drawer. Opened
            // rather than toggled, so a drawer already open stays open; armed
            // only if forgetting would be right, and otherwise the row says
            // why not.
            menu::TrackAction::ForgetMissing => {
                self.set_overlay(Some(Overlay::Settings));
                self.arm_forget();
            }
        }
        Task::none()
    }

    /// The header's overflow menu.
    ///
    /// Each item opens the panel its old button did and sends the same message
    /// doing it, so nothing below this changed -- only the way in.
    fn on_header_menu(&mut self, action: menu::HeaderAction) -> Task<Message> {
        self.menu = None;
        match action {
            menu::HeaderAction::Columns => self.open_overlay(Overlay::Columns),
            menu::HeaderAction::Settings => self.open_overlay(Overlay::Settings),
            menu::HeaderAction::Shortcuts => self.open_overlay(Overlay::Help),
        }
    }

    /// The queue's own table events. §9.2: double-click sets `position`.
    fn on_queue(&mut self, event: table::Event) -> Task<Message> {
        match event {
            // A click marks the row and does nothing else, and that is fine: a
            // click producing no response at all reads as a broken window,
            // whatever the model says selection ought to mean here.
            table::Event::Select(index) | table::Event::Context(index) => {
                self.queue_selection = std::iter::once(index).collect();
            }
            table::Event::Activate(index) => {
                self.queue_selection = std::iter::once(index).collect();
                self.send(Command::SetPosition(index));
            }
            table::Event::Scrolled { offset, viewport } => {
                self.queue_scroll = offset;
                self.viewport_height = viewport;
            }
            // The queue is a play order, not a view to sort or rearrange, so
            // its headings offer none of this.
            table::Event::SortBy(_)
            | table::Event::HeadingMenu
            | table::Event::ResizeStart(_)
            | table::Event::ResizeTo(_)
            | table::Event::ResizeEnd
            | table::Event::ReorderStart(_)
            | table::Event::ReorderTo(_)
            | table::Event::ReorderEnd => {}
        }
        Task::none()
    }

    /// The single row the queue's cursor is on.
    fn queue_cursor(&self) -> Option<usize> {
        self.queue_selection.iter().copied().next()
    }

    /// Moves the queue's cursor, for the arrow keys.
    fn move_queue_cursor(&mut self, delta: isize) -> Task<Message> {
        let Some(queue) = &self.queue else {
            return Task::none();
        };
        let Some(last) = queue.len().checked_sub(1) else {
            return Task::none();
        };

        let next = match self.queue_cursor() {
            None => queue.position().unwrap_or(0),
            Some(current) => current.saturating_add_signed(delta).min(last),
        };
        self.queue_selection = std::iter::once(next).collect();
        self.queue_scroll =
            table::centre_offset(next, self.queue_lines.len(), self.viewport_height);

        iced::widget::operation::scroll_to(
            iced::widget::Id::new(QUEUE_ID),
            iced::widget::scrollable::AbsoluteOffset {
                x: 0.0,
                y: self.queue_scroll,
            },
        )
    }

    fn on_artist_pane(&mut self, event: panes::Event) {
        match event {
            panes::Event::SelectAll => {
                self.selection.album_artists.clear();
                // The album pane is downstream, so its selection may name an
                // album that is no longer reachable. §7.2's cascade flows one
                // way; changing an upstream pane resets what is below it.
                self.selection.albums.clear();
                self.reset_view();
            }
            panes::Event::Select(index) => {
                if let Some(row) = self.artists.get(index) {
                    self.selection.album_artists = vec![row.label.clone()];
                    self.selection.albums.clear();
                    self.reset_view();
                }
            }
            panes::Event::Scrolled { offset, .. } => self.artist_scroll = offset,
        }
    }

    fn on_album_pane(&mut self, event: panes::Event) {
        match event {
            panes::Event::SelectAll => {
                self.selection.albums.clear();
                self.reset_view();
            }
            panes::Event::Select(index) => {
                if let Some(id) = self.album_ids.get(index) {
                    self.selection.albums = vec![*id];
                    self.reset_view();
                }
            }
            panes::Event::Scrolled { offset, .. } => self.album_scroll = offset,
        }
    }

    /// Opens, closes or replaces an overlay, and prepares whatever it needs.
    fn open_overlay(&mut self, requested: Overlay) -> Task<Message> {
        self.set_overlay(overlays::toggle(self.open_overlay_kind(), requested));

        // Reading a file's tags is I/O, so it happens once on open rather than
        // in the view, which runs every frame.
        if let Some(Overlay::Properties(id)) = self.open_overlay_kind() {
            self.load_properties(id);
        }

        // The queue opens showing where you are, which means scrolled to it.
        // §9.2 asks for the current position marked *and* scrolled into view.
        if self.open_overlay_kind() == Some(Overlay::Queue) {
            // Opens with the cursor where playback is, which is also the row
            // it scrolls to. Arriving with nothing selected would make the
            // first arrow press jump somewhere arbitrary.
            let position = self
                .queue
                .as_ref()
                .and_then(scoria_core::PlaybackSource::position);
            self.queue_selection = position.into_iter().collect();
            if let Some(position) = position {
                self.queue_scroll =
                    table::centre_offset(position, self.queue_lines.len(), self.viewport_height);
                return iced::widget::operation::scroll_to(
                    iced::widget::Id::new(QUEUE_ID),
                    iced::widget::scrollable::AbsoluteOffset {
                        x: 0.0,
                        y: self.queue_scroll,
                    },
                );
            }
        }
        Task::none()
    }

    /// Changes which overlay is open, restarting the transition.
    ///
    /// Every path that opens or replaces one goes through here. A second
    /// overlay appearing fully formed, because only the first reset the clock,
    /// is exactly the "did anything happen?" problem this exists to solve.
    fn set_overlay(&mut self, requested: Option<Overlay>) {
        if requested == Some(Overlay::Settings) {
            self.refresh_library_facts();
        } else {
            self.disarm();
        }
        let now = std::time::Instant::now();
        match (requested, self.overlay) {
            // Dismissed: retract rather than vanish.
            (None, Some(open)) => self.overlay = Some(open.closing(now)),
            (None, None) => {}
            (Some(overlay), current) => {
                if current.and_then(overlays::Presenting::effective) == Some(overlay) {
                    return;
                }
                self.overlay = Some(overlays::Presenting::opening(overlay, now));
            }
        }
    }

    /// How tall this kind of drawer should be, as a fraction.
    ///
    /// Whatever it was last dragged to, or the design's default for that kind.
    fn drawer_fraction(&self, overlay: Overlay) -> f32 {
        self.settings
            .drawer_heights
            .get(overlay.title())
            .copied()
            .unwrap_or_else(|| overlay.default_fraction())
    }

    /// What is open, ignoring anything on its way out.
    fn open_overlay_kind(&self) -> Option<Overlay> {
        self.overlay.and_then(overlays::Presenting::effective)
    }

    /// Drops an overlay once it has finished retracting.
    ///
    /// Runs on the tick that already exists, so the drawer keeps being drawn
    /// while it retracts and disappears only when there is nothing left of it.
    fn tick_overlay(&mut self) {
        if let Some(state) = self.overlay
            && state.is_finished(std::time::Instant::now())
        {
            self.overlay = None;
        }
    }

    /// How far through its opening transition the overlay is.
    fn overlay_progress(&self) -> f32 {
        self.overlay
            .map_or(0.0, |state| state.shown(std::time::Instant::now()))
    }

    /// Reads the file's own tags for the properties overlay.
    ///
    /// Done on open rather than on render: it is file I/O, and a view function
    /// that touched the disk would do it on every frame.
    fn load_properties(&mut self, id: TrackId) {
        self.properties.clear();
        self.properties_problem = None;

        let Some(session) = &self.session else { return };
        let Some(track) = session.index().track(id) else {
            return;
        };
        let Ok(root) = session.config().root() else {
            self.properties_problem = Some("no library root is set".to_owned());
            return;
        };

        match scoria_core::library::tags::read_all(&track.path.to_absolute(root)) {
            Ok(tags) => self.properties = tags,
            Err(e) => self.properties_problem = Some(format!("{e}")),
        }
    }

    /// Drops every pane filter.
    ///
    /// Reachable from the chip in the header and from Escape, and both matter:
    /// filters survive a layout change, and `Off` shows no panes, so without a
    /// way out that does not involve the panes there is a state you can enter
    /// and not leave. The search box is left alone -- it is visible in every
    /// placement and clears itself.
    fn clear_filters(&mut self) {
        if self.selection.album_artists.is_empty() && self.selection.albums.is_empty() {
            return;
        }
        self.selection.album_artists.clear();
        self.selection.albums.clear();
        self.reset_view();
    }

    /// What the panes are currently narrowing to, if anything.
    ///
    /// Search is deliberately excluded: it has its own visible field in every
    /// placement, so restating it here would be noise.
    fn filter_summary(&self) -> Option<String> {
        let artists = &self.selection.album_artists;
        let albums: Vec<&str> = self
            .album_ids
            .iter()
            .zip(&self.albums)
            .filter(|(id, _)| self.selection.albums.contains(id))
            .map(|(_, row)| row.label.as_str())
            .collect();

        if artists.is_empty() && albums.is_empty() {
            return None;
        }

        // Read in cascade order, which is the order they were chosen in.
        let mut parts: Vec<String> = Vec::new();
        if !artists.is_empty() {
            parts.push(artists.join(", "));
        }
        if !albums.is_empty() {
            parts.push(albums.join(", "));
        }
        Some(parts.join(" › "))
    }

    /// Re-runs the query and returns the table to the top.
    ///
    /// Keeping the old scroll position after a filter change would leave the
    /// view somewhere arbitrary in a list that is now a different length.
    fn reset_view(&mut self) {
        self.scroll_offset = 0.0;
        self.selection_rows.clear();
        self.refresh();
    }

    fn on_transport(&mut self, event: transport::Event) -> Task<Message> {
        let command = match event {
            transport::Event::PlayPause => {
                if self.state == PlaybackState::Playing {
                    Command::Pause
                } else {
                    Command::Resume
                }
            }
            transport::Event::Next => Command::Next,
            transport::Event::Previous => Command::Previous,
            transport::Event::ToggleShuffle => Command::SetShuffle(!self.shuffled),
            transport::Event::CycleRepeat => Command::SetRepeat(next_repeat(self.repeat)),
            transport::Event::SetVolume(v) => {
                self.volume = v;
                // Marked rather than saved: a drag or a spun wheel is hundreds
                // of these, and the file is written on the next tick that finds
                // it stale. See `save_volume_if_settled`.
                self.volume_unsaved = true;
                self.volume_changed_at = Some(std::time::Instant::now());
                Command::SetVolume(v)
            }
            transport::Event::JumpToPlaying => return self.jump_to_playing(),
            transport::Event::Scrub(seconds) => {
                self.scrubbing = Some(Duration::from_secs_f32(seconds.max(0.0)));
                return Task::none();
            }
            transport::Event::SeekTo(position) => {
                self.scrubbing = None;
                // Show the requested position at once rather than waiting for
                // the engine to report it, so the handle does not snap back for
                // a fraction of a second on release.
                self.position = position;
                Command::Seek(position)
            }
            transport::Event::ToggleQueue => return self.open_overlay(Overlay::Queue),
            // §9.1: the resting state is the thumbnail; clicking it opens the
            // playing track's art full size.
            transport::Event::ShowArtwork => {
                self.set_overlay(overlays::toggle(
                    self.open_overlay_kind(),
                    Overlay::Artwork(None),
                ));
                return Task::none();
            }
        };
        self.send(command);
        Task::none()
    }

    fn on_action(&mut self, action: Action) -> Task<Message> {
        // §9: an overlay is modal, so movement belongs to it while it is open.
        // Letting these through would move a selection nobody can see, behind
        // something covering it.
        if self.open_overlay_kind() == Some(Overlay::Queue) {
            match action {
                Action::MoveDown => return self.move_queue_cursor(1),
                Action::MoveUp => return self.move_queue_cursor(-1),
                Action::PageDown => return self.move_queue_cursor(self.page()),
                Action::PageUp => return self.move_queue_cursor(-self.page()),
                Action::PlaySelection => {
                    if let Some(index) = self.queue_cursor() {
                        self.send(Command::SetPosition(index));
                    }
                    return Task::none();
                }
                _ => {}
            }
        }

        match action {
            Action::MoveDown => return self.move_selection(1, false),
            Action::MoveUp => return self.move_selection(-1, false),
            Action::PageDown => return self.move_selection(self.page(), false),
            Action::PageUp => return self.move_selection(-self.page(), false),
            Action::MoveToStart => {
                if !self.rows.is_empty() {
                    self.select(0, crate::keymap::Mods::NONE);
                }
                return self.scroll_to_cursor();
            }
            Action::MoveToEnd => {
                if let Some(last) = self.rows.len().checked_sub(1) {
                    self.select(last, crate::keymap::Mods::NONE);
                }
                return self.scroll_to_cursor();
            }
            Action::ExtendUp => return self.move_selection(-1, true),
            Action::ExtendDown => return self.move_selection(1, true),
            Action::PlaySelection => self.play_from_cursor(),
            Action::PlayPause => {
                let command = if self.state == PlaybackState::Playing {
                    Command::Pause
                } else {
                    Command::Resume
                };
                self.send(command);
            }
            Action::Next => self.send(Command::Next),
            Action::Previous => self.send(Command::Previous),
            Action::ToggleShuffle => self.send(Command::SetShuffle(!self.shuffled)),
            Action::CycleRepeat => self.send(Command::SetRepeat(next_repeat(self.repeat))),
            Action::JumpToPlaying => return self.jump_to_playing(),
            Action::Rescan => match self.session.as_ref().map(Session::request_rescan) {
                Some(Ok(())) => self.status = "rescanning...".to_owned(),
                Some(Err(e)) => self.status = format!("{e}"),
                None => {}
            },
            Action::CycleLayout => {
                self.set_placement(self.placement.cycle());
                // Said in words for the keyboard path only. The buttons show
                // which placement is current, so a click needs no commentary.
                self.status = self.placement.label().to_owned();
            }
            Action::FocusSearch => {
                return iced::widget::operation::focus(iced::widget::Id::new(SEARCH_ID));
            }
            Action::ToggleQueue => return self.open_overlay(Overlay::Queue),
            Action::ToggleColumns => return self.open_overlay(Overlay::Columns),
            Action::ToggleSettings => return self.open_overlay(Overlay::Settings),
            Action::ShowProperties => {
                if let Some(id) = self
                    .cursor()
                    .and_then(|index| self.rows.get(index).copied())
                {
                    return self.open_overlay(Overlay::Properties(id));
                }
                self.status = "select a track first".to_owned();
            }
            Action::ToggleHelp => return self.open_overlay(Overlay::Help),
            Action::Dismiss => {
                // §9: Escape closes the overlay. With none open it clears the
                // filters instead -- one key backing out of whatever is
                // currently narrowing the view.
                //
                // The menu goes first, and on its own: it is the most recent
                // thing to have opened, so it is what Escape is reaching for.
                if self.menu.take().is_some() {
                    return Task::none();
                }
                // Likewise an armed question in the settings drawer: it is
                // the most recent thing on screen, so Escape answers it
                // before it closes the drawer it is asked in.
                if self.disarm() {
                    return Task::none();
                }
                if self.open_overlay_kind().is_none() {
                    self.clear_filters();
                } else {
                    self.set_overlay(None);
                }
            }
        }
        Task::none()
    }

    /// §6.5: restore an unfiltered full-library view and scroll to the playing
    /// track. Deliberately not scoped to its album or artist -- the default
    /// sort puts it inside its album anyway, with the artist's other albums
    /// immediately above and below.
    fn jump_to_playing(&mut self) -> Task<Message> {
        let Some(playing) = self.playing else {
            self.status = "nothing is playing".to_owned();
            return Task::none();
        };

        self.search.clear();
        self.selection = Selection::all();
        self.refresh();

        if let Some(index) = self.rows.iter().position(|id| *id == playing) {
            self.selection_rows.focus(index, self.rows.len());
            // Centred, not merely brought into view. §6.5 argues that jumping
            // to the playing track needs no album or artist filter because the
            // default sort already surrounds it with its own album and the
            // artist's neighbouring ones -- and that context only exists if the
            // track lands in the middle rather than against an edge.
            return self.centre_on_cursor();
        }
        Task::none()
    }

    /// Scrolls so the cursor sits in the middle of the table.
    ///
    /// Distinct from [`Self::scroll_to_cursor`], which moves as little as it
    /// can. Arrow keys want the minimum, or the view lurches on every press;
    /// arriving somewhere from elsewhere wants the context that surrounds it.
    fn centre_on_cursor(&mut self) -> Task<Message> {
        let Some(cursor) = self.cursor() else {
            return Task::none();
        };
        let Some(line) = self.line_of_row(cursor) else {
            return Task::none();
        };

        self.scroll_offset = table::centre_offset(line, self.lines.len(), self.viewport_height);
        iced::widget::operation::scroll_to(
            iced::widget::Id::new(TABLE_ID),
            iced::widget::scrollable::AbsoluteOffset {
                x: 0.0,
                y: self.scroll_offset,
            },
        )
    }

    /// Applies a click to the selection. The rules live in [`RowSelection`].
    fn select(&mut self, index: usize, modifiers: crate::keymap::Mods) {
        self.selection_rows.click(index, modifiers, self.rows.len());
    }

    /// Moves the cursor, replacing or extending the selection.
    fn move_selection(&mut self, delta: isize, extend: bool) -> Task<Message> {
        self.selection_rows.move_by(delta, extend, self.rows.len());
        self.scroll_to_cursor()
    }

    /// The row the keyboard is on.
    fn cursor(&self) -> Option<usize> {
        self.selection_rows.cursor()
    }

    fn play_from_cursor(&mut self) {
        let Some(index) = self.cursor() else { return };
        let Some(at) = self.rows.get(index).copied() else {
            return;
        };
        let Some(session) = &self.session else { return };

        // Several rows chosen means "play these", which is what a materialised
        // snapshot is for (§6.2): the chosen tracks *are* the play order, and
        // nothing re-derives them afterwards.
        let spec = if self.selection_rows.len() > 1 {
            let tracks: Vec<_> = self
                .selection_rows
                .rows()
                .iter()
                .filter_map(|i| self.rows.get(*i).copied())
                .collect();
            scoria_core::SourceSpec::Shown {
                label: Some(format!("{} tracks", tracks.len())),
                tracks,
            }
        } else {
            // One row means "play from here", so the §6.3 setting decides what
            // gets snapshotted around it.
            session.source_spec(self.rows.clone(), self.selection.label())
        };
        self.send(Command::PlayFrom { source: spec, at });
    }

    fn send(&mut self, command: Command) {
        let Some(session) = &mut self.session else {
            self.status = "no library is open".to_owned();
            return;
        };
        if let Err(e) = session.handle(command) {
            self.status = format!("{e}");
        }
    }

    fn page(&self) -> isize {
        ((self.viewport_height / table::ROW_HEIGHT).floor() as isize - 1).max(1)
    }

    /// Keeps the cursor inside the viewport by nudging the scroll offset.
    ///
    /// Returns a task that moves the real scrollable. Changing our own offset
    /// alone would only change which rows get built -- the widget would stay
    /// where it was, and the two would disagree.
    fn scroll_to_cursor(&mut self) -> Task<Message> {
        let Some(cursor) = self.cursor() else {
            return Task::none();
        };
        let Some(line) = self.line_of_row(cursor) else {
            return Task::none();
        };

        let top = line as f32 * table::ROW_HEIGHT;
        let bottom = top + table::ROW_HEIGHT;
        let previous = self.scroll_offset;

        if top < self.scroll_offset {
            self.scroll_offset = top;
        } else if bottom > self.scroll_offset + self.viewport_height {
            self.scroll_offset = bottom - self.viewport_height;
        }

        if (self.scroll_offset - previous).abs() < f32::EPSILON {
            return Task::none();
        }
        iced::widget::operation::scroll_to(
            iced::widget::Id::new(TABLE_ID),
            iced::widget::scrollable::AbsoluteOffset {
                x: 0.0,
                y: self.scroll_offset,
            },
        )
    }

    /// Which display line a row sits on, accounting for group headers.
    fn line_of_row(&self, row: usize) -> Option<usize> {
        self.lines
            .iter()
            .position(|line| matches!(line, table::Line::Track(i) if *i == row))
    }

    fn track(&self, id: TrackId) -> Option<&Track> {
        self.session.as_ref()?.index().track(id)
    }

    pub fn view(&self) -> Element<'_, Message, Theme> {
        // Why there is nothing to list, when there is nothing to list (§5.1).
        // It takes the table's place rather than covering it, so the window
        // keeps its shape and the header stays usable.
        let welcome = welcome::state(
            self.problem.as_deref(),
            self.session
                .as_ref()
                .and_then(|s| s.config().root.as_deref()),
            self.rows.len(),
            self.scanning,
        );

        let table_view = table::TableView {
            lines: &self.lines,
            // Resolved as the table draws, so a six-figure result costs a
            // few dozen lookups a frame rather than a hundred thousand.
            rows: Box::new(|index| self.rows.get(index).and_then(|id| self.track(*id))),
            selected: self.selection_rows.rows(),
            playing: self.playing,
            playing_row: None,
            columns: &self.settings.columns,
            sort: self.sort_override.get(),
            resizing: self.resizing_column.is_some(),
            reordering: self.reordering,
            scroll_offset: self.scroll_offset,
            theme: &self.theme,
            id: TABLE_ID,
        };

        let now_playing = self.playing.and_then(|id| self.track(id));
        let transport_view = transport::TransportView {
            track: now_playing,
            artwork: self.playing_artwork(),
            queue_open: self.open_overlay_kind() == Some(Overlay::Queue),
            state: self.state,
            position: self.position,
            scrubbing: self.scrubbing,
            shuffled: self.shuffled,
            repeat: self.repeat,
            volume: self.volume,
            theme: &self.theme,
        };

        let muted = self.theme.text_style(self.theme.muted_colour());
        let mut header = row![
            text_input("Search", &self.search)
                .id(iced::widget::Id::new(SEARCH_ID))
                .on_input(Message::SearchChanged)
                .padding(4)
                .size(13)
                .width(Length::Fixed(320.0)),
            text(self.status.clone())
                .size(12)
                .style(move |_t: &Theme| muted),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center);

        // The filter, stated and dismissible, in every placement. `Off` shows
        // no panes, so this is the only thing standing between a filtered view
        // and a dead end.
        if let Some(summary) = self.filter_summary() {
            header = header.push(
                button(
                    row![text(summary).size(12), text("✕").size(12)]
                        .spacing(6)
                        .align_y(iced::Alignment::Center),
                )
                .padding([2, 8])
                .on_press(Message::ClearFilters)
                .style(|t: &Theme, status| t.chip_button(status)),
            );
        }

        // Pushed to the right so the buttons sit where chrome is looked for,
        // and so the chip stays beside the text it qualifies.
        header = header.push(iced::widget::Space::new().width(Fill));

        // Where the browser sits (§7.1). Three cells rather than one button
        // that cycles: a mouse should point at the layout it wants rather than
        // clicking twice to pass through the one it does not, and three cells
        // say what the alternatives are, which a cycling key never could. The
        // keystroke stays a cycle, because that is what a keystroke is good at.
        //
        // Segmented, because it is one setting with three values. As three
        // separate buttons beside a label it read as four separate items --
        // and its accent meant "the current placement" two pixels from where
        // the same accent meant "this panel is open".
        header = header.push(text("Browser").size(11).style(move |_t: &Theme| muted));
        let mut segments = row![].align_y(iced::Alignment::Center);
        for placement in [
            BrowserPlacement::Off,
            BrowserPlacement::Top,
            BrowserPlacement::Left,
        ] {
            let current = placement == self.placement;
            segments = segments.push(
                button(text(placement.short_label()).size(12))
                    .padding([3, 9])
                    .on_press(Message::SetPlacement(placement))
                    .style(move |t: &Theme, status| t.segment_button(status, current)),
            );
        }
        let group = self.theme.segment_group();
        header = header.push(
            container(segments)
                // So a filled end cell stays inside the rounded corner.
                .clip(true)
                .style(move |_t: &Theme| group),
        );

        // One way in for three panels that used to have a button each, in a row
        // of seven look-alikes next to a setting that is not a panel at all.
        //
        // Each of them has a keystroke and none of them said so anywhere a
        // mouse would find it; the menu says all three. A drawn glyph rather
        // than a `⋯`, for the reason §8 gives: the transport's symbols were
        // drawn so that no font could decide what they look like.
        header = header.push(
            button(crate::widgets::icon::view(
                crate::widgets::icon::Icon::More,
                self.menu
                    .is_some_and(|open| matches!(open, OpenMenu::Header { .. })),
            ))
            .padding([3, 8])
            .on_press(Message::OpenHeaderMenu)
            .style(|t: &Theme, status| t.transport_button(status, false)),
        );

        let search = container(header.spacing(6)).padding([6, 10]).width(Fill);

        // Built only when a placement will use it: `Off` shows no panes, and
        // constructing them would be work thrown away every frame. Suppressed
        // entirely while the welcome panel is up -- two empty filter panes
        // beside "choose a folder" are furniture with nothing in them.
        let browser =
            (self.placement != BrowserPlacement::Off && welcome.is_none()).then(|| Browser {
                artists: panes::view(
                    panes::PaneView {
                        heading: "ALBUM ARTIST",
                        rows: &self.artists,
                        total: self.artist_total,
                        scroll_offset: self.artist_scroll,
                        theme: &self.theme,
                        id: ARTIST_PANE_ID,
                    },
                    Message::ArtistPane,
                ),
                albums: panes::view(
                    panes::PaneView {
                        heading: "ALBUM",
                        rows: &self.albums,
                        total: self.album_total,
                        scroll_offset: self.album_scroll,
                        theme: &self.theme,
                        id: ALBUM_PANE_ID,
                    },
                    Message::AlbumPane,
                ),
                artwork: self.artwork_view(self.playing_artwork()),
            });
        // Handed to the layout rather than wrapped around the finished view, so
        // it covers the explorer and leaves the transport reachable.
        // Drawn while retracting too, which is what makes closing an animation
        // rather than a disappearance.
        let overlay = self.overlay.map(|state| {
            overlays::present(overlays::Presentation {
                overlay: state.overlay,
                content: self.overlay_content(state.overlay),
                theme: &self.theme,
                shown: self.overlay_progress(),
                fraction: self.drawer_fraction(state.overlay),
                dragging: self.dragging_drawer,
                on_close: Message::CloseOverlay,
                on_drag_start: Message::DrawerDragStart,
                on_drag_to: Message::DrawerDragTo,
                on_drag_end: Message::DrawerDragEnd,
            })
        });

        let body = match &welcome {
            Some(state) => welcome::view(state, &self.theme, Message::Welcome),
            None => table::view(table_view, Message::Table),
        };

        let window = layouts::arrange(
            self.placement,
            body,
            transport::view(&transport_view, Message::Transport),
            search.into(),
            browser,
            overlay,
            &self.theme,
        );

        // Stacked here rather than handed to `arrange`, which arranges the
        // application and should not have to learn about a menu. It goes over
        // everything, the transport included: unlike §9's drawers this is a
        // momentary thing at the pointer, and a menu the window can clip is a
        // menu with items nobody can choose.
        let menu_layer: Option<Element<'_, Message, Theme>> = match self.menu {
            Some(OpenMenu::Track { track, at }) => self.track(track).map(|track| {
                menu::view(
                    menu::track_items(track, &self.keymap),
                    at,
                    &self.theme,
                    Message::TrackMenu,
                    Message::CloseMenu,
                )
            }),
            Some(OpenMenu::Header { at }) => Some(menu::view(
                menu::header_items(&self.keymap),
                at,
                &self.theme,
                Message::HeaderMenu,
                Message::CloseMenu,
            )),
            None => None,
        };

        // **Always a stack, menu or no menu.**
        //
        // iced diffs the widget tree by type, so swapping the root between a
        // container and a stack throws away the state of everything under it
        // -- including the table's scroll offset. The scrollable then reset to
        // the top and reported it, and `Scrolled` closes the menu, so the menu
        // destroyed itself on the very next event.
        //
        // It only did that when there was an offset to lose, which is why a
        // filtered view of eleven rows worked and the whole library never did.
        //
        // Opaque for the reason `layouts::arrange` wraps its overlay: a layer
        // that is not lets mouse events through to the rows underneath. Both
        // lengths for the reason that one is `Shrink` by default, which sets
        // the limits compressing and collapses every `Fill` inside.
        let layer: Element<'_, Message, Theme> = match menu_layer {
            Some(menu) => iced::widget::opaque(menu),
            None => Space::new().width(0).height(0).into(),
        };

        iced::widget::stack![window, layer]
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn overlay_content(&self, overlay: Overlay) -> Element<'_, Message, Theme> {
        match overlay {
            Overlay::Settings => overlays::settings::view(
                overlays::settings::SettingsView {
                    root: self.session.as_ref().and_then(|s| s.config().root.clone()),
                    play_behaviour: self
                        .session
                        .as_ref()
                        .map_or(scoria_core::PlayBehaviour::WhatsShown, |s| {
                            s.config().play_behaviour
                        }),
                    theme_name: &self.theme.name,
                    config_file: self.paths.config_file(),
                    database: self.paths.database(),
                    artwork_cache: self.paths.artwork_cache(),
                    track_count: self.session.as_ref().map_or(0, |s| s.index().len()),
                    missing: self.missing(),
                    loudness: self.loudness(),
                    gain_failures: &self.gain_failures,
                    theme: &self.theme,
                },
                Message::Settings,
            ),
            Overlay::Help => overlays::help::view(overlays::help::HelpView {
                keymap: &self.keymap,
                theme: &self.theme,
            }),
            Overlay::Columns => overlays::columns::view(
                overlays::columns::ColumnsView {
                    layout: &self.settings.columns,
                    sort: self.settings.sort,
                    overridden: self.sort_override.is_set(),
                    theme: &self.theme,
                },
                Message::Columns,
            ),
            Overlay::Queue => overlays::queue::view(
                overlays::queue::QueueView {
                    columns: &self.settings.columns,
                    lines: &self.queue_lines,
                    rows: Box::new(|index| {
                        let queue = self.queue.as_ref()?;
                        let id = *queue.tracks().get(index)?;
                        self.track(id).or_else(|| self.forgotten.get(&id))
                    }),
                    position: self
                        .queue
                        .as_ref()
                        .and_then(scoria_core::PlaybackSource::position),
                    cursor: &self.queue_selection,
                    scroll_offset: self.queue_scroll,
                    divergence: self.queue_divergence(),
                    theme: &self.theme,
                    id: QUEUE_ID,
                },
                Message::Queue,
            ),
            Overlay::Artwork(album) => {
                // §9.1: normally what is playing. An album is only named when
                // the right-click action named one.
                let album = album.or_else(|| {
                    self.playing
                        .and_then(|id| self.track(id))
                        .map(|t| t.album_id)
                });
                overlays::artwork::view(overlays::artwork::ArtworkView {
                    image: album.and_then(|id| self.session.as_ref()?.artwork_for(id)),
                    caption: album.and_then(|id| self.album_caption(id)),
                    theme: &self.theme,
                })
            }
            Overlay::Properties(id) => {
                overlays::properties::view(overlays::properties::PropertiesView {
                    summary: self.track(id).map(track_summary).unwrap_or_default(),
                    tags: self.properties.clone(),
                    problem: self.properties_problem.clone(),
                    theme: &self.theme,
                })
            }
        }
    }

    /// A caption naming the album some artwork belongs to.
    ///
    /// Through the album rather than by hunting for a track on it: the index
    /// holds albums by id, and this ran while the overlay was open, which is
    /// to say on every frame.
    fn album_caption(&self, album: scoria_core::AlbumId) -> Option<String> {
        let album = self.session.as_ref()?.index().album(album)?;
        Some(format!("{} — {}", album.album_artist, album.title))
    }

    /// §9.2: a one-line explanation, *only* when the view and the snapshot
    /// disagree.
    ///
    /// This is the question the queue exists to answer. §6.2 lets `tracks` and
    /// `origin` diverge on purpose, and without saying so a Next button that
    /// disagrees with the visible list reads as a bug.
    fn queue_divergence(&self) -> Option<String> {
        let queue = self.queue.as_ref()?;
        if queue.tracks() == self.rows.as_slice() {
            return None;
        }
        Some(format!(
            "Playing a snapshot of {} track(s) taken from {}. The list you are browsing has changed since.",
            queue.len(),
            queue.origin().label(),
        ))
    }

    /// The playing album's artwork, found at most once per track.
    ///
    /// §9.1: artwork always follows what is playing. There is no mode and no
    /// toggle, here or anywhere else it appears -- which is exactly why the
    /// playing track is a sufficient key.
    /// Brings the cached artwork into line with what is playing.
    ///
    /// Called from `update`, never from `view`: the lookup touches the
    /// filesystem, and a view function runs on every frame.
    fn refresh_artwork(&mut self) {
        if self.artwork.as_ref().map(|(for_track, _)| *for_track) == Some(self.playing) {
            return;
        }
        let path = self
            .playing
            .and_then(|id| self.track(id))
            .and_then(|track| self.session.as_ref()?.artwork_for(track.album_id));
        self.artwork = Some((self.playing, path));
    }

    /// The artwork found for what is playing, as last looked up.
    fn playing_artwork(&self) -> Option<std::path::PathBuf> {
        self.artwork.as_ref().and_then(|(_, path)| path.clone())
    }

    /// The now-playing album's artwork, or a placeholder.
    fn artwork_view(&self, path: Option<std::path::PathBuf>) -> Element<'_, Message, Theme> {
        match path {
            Some(path) => iced::widget::image(path)
                .width(Fill)
                .height(Fill)
                .content_fit(iced::ContentFit::Contain)
                .into(),
            None => {
                let muted = self.theme.text_style(self.theme.muted_colour());
                container(text("♪").size(40).style(move |_t: &Theme| muted))
                    .width(Fill)
                    .height(Fill)
                    .align_x(iced::Alignment::Center)
                    .align_y(iced::Alignment::Center)
                    .into()
            }
        }
    }

    pub fn theme(&self) -> Theme {
        self.theme.clone()
    }

    pub fn title(&self) -> String {
        match self.playing.and_then(|id| self.track(id)) {
            Some(track) => format!("{} — {} — Scoria", track.title, track.artist),
            None => "Scoria".to_owned(),
        }
    }

    /// Ticks to drain the event stream, and key presses resolved through the
    /// keymap.
    ///
    /// §10: the keymap is a data structure. This is the only place a key press
    /// is looked at, and all it does is ask the keymap what the key means.
    pub fn subscription(&self) -> Subscription<Message> {
        // Published to the thread-local the resolver reads, because
        // `listen_with` takes a plain `fn` and the map cannot travel in with
        // the closure. Without this the cheatsheet lists one keymap and key
        // presses resolve against another -- which is invisible today, since
        // both are `Keymap::standard()`, and stops being invisible the moment
        // §10's second keymap is a value someone can choose.
        //
        // Guarded on equality because iced asks for the subscription after
        // every message, and this is a poll-driven application.
        publish_keymap(&self.keymap);

        let keys = iced::event::listen_with(|event, status, _window| {
            // Recorded whatever else happens, including while something else
            // has captured the event: the pointer is still where it is.
            if let iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) = event {
                shared::set_pointer(position);
                return None;
            }

            // A key typed into the search field belongs to the search field.
            if status == iced::event::Status::Captured {
                return None;
            }
            match event {
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    ref key,
                    ref modified_key,
                    modifiers,
                    ..
                }) => resolve_key(key, modified_key, modifiers).map(Message::Action),
                // Tracked separately because a mouse click carries no
                // modifiers of its own, and shift-click has to know.
                iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(modifiers)) => Some(
                    Message::Modifiers(crate::keymap::Mods::from_iced(modifiers)),
                ),
                _ => None,
            }
        });

        Subscription::batch([iced::time::every(POLL).map(|_| Message::Tick), keys])
    }
}

/// State the subscription and the application both touch.
///
/// **Shared across threads, not thread-local.** `listen_with` takes a plain
/// `fn`, so nothing can travel in with the closure and it has to reach for
/// something it can name. The obvious answer is a thread-local, and it is
/// wrong: iced's `thread-pool` feature is on by default, so a subscription
/// runs on a pool thread while `update` and `view` run on the main one. A
/// thread-local written by one is invisible to the other, and each simply sees
/// its own initial value.
///
/// That failed quietly in both directions. The keymap looked published because
/// the value the subscription thread never received was the same
/// `Keymap::standard()` it had started with. The pointer did not have that
/// luck: `view` read its own thread's untouched `ORIGIN`, and the menu opened
/// in the corner of the window every time.
mod shared {
    use crate::keymap::Keymap;
    use std::sync::{Mutex, PoisonError};

    /// The keymap key presses resolve against (§10).
    static KEYMAP: Mutex<Option<Keymap>> = Mutex::new(None);

    /// Where the pointer was, last time it moved.
    ///
    /// Recorded rather than sent as a message: every message rebuilds the
    /// view, and this changes on every pixel of every movement, so a message
    /// would mean rebuilding the table for a value almost nothing reads.
    static POINTER: Mutex<(f32, f32)> = Mutex::new((0.0, 0.0));

    /// A poisoned lock means a thread panicked holding it. Both of these are
    /// plain values with no invariant to violate, so reading through the
    /// poison is right: the panic is the failure worth reporting, not this.
    pub fn publish_keymap(keymap: &Keymap) {
        let mut published = KEYMAP.lock().unwrap_or_else(PoisonError::into_inner);
        if published.as_ref() != Some(keymap) {
            *published = Some(keymap.clone());
        }
    }

    pub fn with_keymap<T>(f: impl FnOnce(&Keymap) -> T) -> T {
        let published = KEYMAP.lock().unwrap_or_else(PoisonError::into_inner);
        match published.as_ref() {
            Some(keymap) => f(keymap),
            // Before the first subscription is asked for.
            None => f(&Keymap::standard()),
        }
    }

    pub fn set_pointer(position: iced::Point) {
        *POINTER.lock().unwrap_or_else(PoisonError::into_inner) = (position.x, position.y);
    }

    pub fn pointer() -> iced::Point {
        let (x, y) = *POINTER.lock().unwrap_or_else(PoisonError::into_inner);
        iced::Point::new(x, y)
    }
}

/// Makes `keymap` the one key presses resolve against.
///
/// Guarded on equality because iced asks for the subscription after every
/// message, and this is a poll-driven application.
fn publish_keymap(keymap: &Keymap) {
    shared::publish_keymap(keymap);
}

/// Where the pointer is now.
fn pointer() -> iced::Point {
    shared::pointer()
}

/// What a key press means, according to the published keymap.
///
/// iced hands over two keys and they are not interchangeable. `key` is
/// `key_without_modifiers`, so Shift+`/` arrives as `/`; `modified_key` is the
/// logical key, so the same press arrives as `?`. A binding on `?` can only
/// ever match the second, which is why binding it did nothing at all for as
/// long as this read only the first.
///
/// The typed character is tried first, then the key underneath it. The
/// fallback is what keeps `Ctrl`-chords working wherever a modified key comes
/// back as something other than the character -- the chord is on the key, not
/// on what it would have typed.
fn resolve_key(
    key: &iced::keyboard::Key,
    modified_key: &iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
) -> Option<Action> {
    shared::with_keymap(|keymap| {
        keymap
            .resolve_iced(modified_key, modifiers)
            .or_else(|| keymap.resolve_iced(key, modifiers))
    })
}

/// The open menu, if there is one.
///
/// One field rather than one per menu, so §9's "one at a time" is true by
/// construction and Escape has one thing to close before it reaches a drawer.
#[derive(Debug, Clone, Copy, PartialEq)]
enum OpenMenu {
    /// A track's, opened by right-clicking its row (§9.6).
    Track { track: TrackId, at: iced::Point },
    /// The header's overflow.
    Header { at: iced::Point },
}

const fn next_repeat(current: RepeatMode) -> RepeatMode {
    match current {
        RepeatMode::Off => RepeatMode::All,
        RepeatMode::All => RepeatMode::One,
        RepeatMode::One => RepeatMode::Off,
    }
}

/// A one-line summary of what a scan changed, for the status bar.
/// Where the cursor goes after the library changes under it.
///
/// On the same track if it survived. If it did not -- forgotten, in practice --
/// on the row that slid into its place, which is the nearest one; the top of
/// the library is not.
fn refocus(survived: Option<usize>, cursor: Option<usize>, len: usize) -> Option<usize> {
    survived.or_else(|| Some(cursor?.min(len.checked_sub(1)?)))
}

/// The Missing row's state (§7.4).
///
/// The order is the order of what makes forgetting wrong: nothing to forget,
/// then a library that is not there, then a scan in flight. Only past all
/// three does a press arm it.
fn missing_state(
    count: usize,
    reachable: bool,
    scanning: bool,
    armed: bool,
) -> overlays::settings::Missing {
    use overlays::settings::Missing;
    if count == 0 {
        Missing::None
    } else if !reachable {
        Missing::Unreachable(count)
    } else if scanning {
        Missing::Scanning(count)
    } else if armed {
        Missing::Armed(count)
    } else {
        Missing::Ready(count)
    }
}

/// The Loudness row's state (§6.7).
///
/// A running analysis outranks everything: it is what is happening. After
/// that, the order of what makes starting one wrong, as for the Missing row.
fn loudness_state(
    counts: overlays::settings::LoudnessCounts,
    reachable: bool,
    scanning: bool,
    armed: bool,
    progress: Option<(usize, usize, String)>,
) -> overlays::settings::Loudness {
    use overlays::settings::Loudness;
    if let Some((done, total, album)) = progress {
        return Loudness::Running { done, total, album };
    }
    if counts.total == 0 {
        Loudness::None
    } else if counts.levelled == counts.total {
        Loudness::AllLevelled
    } else if !reachable {
        Loudness::Unreachable(counts)
    } else if scanning {
        Loudness::Scanning(counts)
    } else if counts.albums == 0 {
        Loudness::Stuck(counts)
    } else if armed {
        Loudness::Armed(counts)
    } else {
        Loudness::Ready(counts)
    }
}

/// What the status line says when an analysis ends.
fn gain_summary(report: &scoria_core::library::gain::GainReport) -> String {
    let albums = |n: usize| {
        if n == 1 {
            "1 album".to_owned()
        } else {
            format!("{n} albums")
        }
    };
    let mut summary = if report.stopped {
        format!(
            "Stopped after {} of {}.",
            report.albums_done,
            albums(report.albums_total)
        )
    } else {
        format!("Levelled {}.", albums(report.albums_done))
    };
    let mut clauses = Vec::new();
    if report.skipped.missing_files > 0 {
        clauses.push(format!(
            "{} skipped (missing files)",
            report.skipped.missing_files
        ));
    }
    if report.skipped.unsupported > 0 {
        clauses.push(format!(
            "{} skipped (format not yet supported)",
            report.skipped.unsupported
        ));
    }
    if !report.failed.is_empty() {
        clauses.push(format!("{} couldn't be written", report.failed.len()));
    }
    if !clauses.is_empty() {
        summary.push(' ');
        summary.push_str(&clauses.join(" · "));
    }
    summary
}

/// A path as the person thinks of it: inside their library, when it is.
fn relative_to(path: &str, root: Option<&std::path::Path>) -> String {
    root.and_then(|root| std::path::Path::new(path).strip_prefix(root).ok())
        .map_or_else(|| path.to_owned(), |p| p.display().to_string())
}

fn describe(delta: &scoria_core::event::LibraryDelta, showing: usize) -> String {
    use scoria_core::event::LibraryDelta as D;
    match delta {
        D::TracksAdded(ids) => format!("{} track(s) added -- showing {showing}", ids.len()),
        D::TracksChanged(ids) => format!("{} track(s) retagged", ids.len()),
        // Only forgetting emits this (§7.4). The second sentence is the undo,
        // and the reason a confirmation was enough without a recycle bin.
        D::TracksRemoved(ids) => format!(
            "Forgot {} missing track(s). A rescan finds them again if the files come back.",
            ids.len()
        ),
        // §7.4: files that vanish are struck through, not dropped, so this is a
        // change in how rows look rather than in how many there are.
        D::AvailabilityChanged(ids) => format!("{} track(s) went missing or came back", ids.len()),
        D::Invalidated => format!("library reloaded -- showing {showing}"),
    }
}

/// What the application knows about a track, for the properties dialog.
///
/// Deliberately separate from the file's own tags: this is the indexed view,
/// including things the file does not say -- the resolved album id, whether the
/// file still exists.
fn track_summary(track: &Track) -> Vec<(&'static str, String)> {
    let mut summary = vec![
        ("Title", track.title.clone()),
        ("Artist", track.artist.clone()),
        ("Album artist", track.album_artist.clone()),
        ("Album", track.album.clone()),
    ];

    if let Some(sort) = &track.album_artist_sort {
        // §4.1: the structural value, which is what sorting and grouping used.
        summary.push(("Album artist sort", sort.clone()));
    }
    if let Some(date) = track.date {
        summary.push(("Date", date.to_string()));
    }
    if let Some(disc) = track.disc {
        summary.push(("Disc", disc.to_string()));
    }
    if let Some(number) = track.track_number {
        summary.push(("Track", number.to_string()));
    }
    summary.push(("Duration", crate::widgets::format_duration(track.duration)));
    summary.push(("Path", track.path.to_string()));

    let gain = track.replay_gain;
    summary.push((
        "ReplayGain",
        if gain.is_tagged() {
            format!(
                "track {} dB, album {} dB",
                gain.track_gain_db
                    .map_or_else(|| "-".to_owned(), |g| format!("{g:.2}")),
                gain.album_gain_db
                    .map_or_else(|| "-".to_owned(), |g| format!("{g:.2}")),
            )
        } else {
            // §6.7: untagged means unity, not silence, and saying so here saves
            // the question being asked of the loudness later.
            "not tagged (plays at 0 dB)".to_owned()
        },
    ));

    if !track.available {
        // §7.4: the row is kept and struck through, and this says why.
        summary.push(("Availability", "file is missing".to_owned()));
    }
    summary
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::keymap::{Binding, Trigger};
    use iced::keyboard::{Key, Modifiers};

    #[test]
    fn right_clicking_a_row_opens_its_menu() {
        // The handler, without a window. `App::disconnected` has no session,
        // which is enough: opening the menu reads `rows`, not the library.
        let mut app = App::disconnected(String::new());
        app.rows = vec![TrackId(11), TrackId(22), TrackId(33)];

        app.on_table(table::Event::Context(1));

        assert_eq!(
            app.menu.and_then(|open| match open {
                OpenMenu::Track { track, .. } => Some(track),
                OpenMenu::Header { .. } => None,
            }),
            Some(TrackId(22)),
            "a right-click did not open the menu on the row it was on"
        );
    }

    // ------------------------------------------------------------ forgetting --

    #[test]
    fn nothing_to_forget_means_no_row_whatever_else_is_true() {
        use overlays::settings::Missing;
        for reachable in [true, false] {
            for scanning in [true, false] {
                for armed in [true, false] {
                    assert_eq!(missing_state(0, reachable, scanning, armed), Missing::None);
                }
            }
        }
    }

    #[test]
    fn an_unreachable_library_is_never_offered_for_forgetting() {
        // §7.4's guard: an unplugged drive makes every track read as missing.
        use overlays::settings::Missing;
        for scanning in [true, false] {
            for armed in [true, false] {
                assert_eq!(
                    missing_state(5, false, scanning, armed),
                    Missing::Unreachable(5)
                );
            }
        }
    }

    #[test]
    fn a_scan_in_flight_outranks_an_armed_row() {
        use overlays::settings::Missing;
        assert_eq!(missing_state(5, true, true, true), Missing::Scanning(5));
        assert_eq!(missing_state(5, true, false, true), Missing::Armed(5));
        assert_eq!(missing_state(5, true, false, false), Missing::Ready(5));
    }

    #[test]
    fn escape_answers_the_question_before_it_closes_the_drawer() {
        let mut app = App::disconnected(String::new());
        app.set_overlay(Some(Overlay::Settings));
        app.forget_armed = true;

        let _ = app.on_action(Action::Dismiss);
        assert!(!app.forget_armed, "escape did not disarm");
        assert_eq!(
            app.open_overlay_kind(),
            Some(Overlay::Settings),
            "escape closed the drawer instead of answering the question in it"
        );

        let _ = app.on_action(Action::Dismiss);
        assert_eq!(app.open_overlay_kind(), None);
    }

    #[test]
    fn leaving_the_settings_drawer_disarms() {
        let mut app = App::disconnected(String::new());
        app.set_overlay(Some(Overlay::Settings));
        app.forget_armed = true;
        app.set_overlay(Some(Overlay::Help));
        assert!(!app.forget_armed);

        app.set_overlay(Some(Overlay::Settings));
        app.forget_armed = true;
        app.set_overlay(None);
        assert!(!app.forget_armed);
    }

    #[test]
    fn confirming_without_having_armed_forgets_nothing() {
        // A stale ConfirmForget -- a double click landing after the row
        // re-rendered -- must not act on a question nobody is being asked.
        let mut app = App::disconnected(String::new());
        app.forget_missing();
        assert!(!app.forget_armed);
    }

    // -------------------------------------------------------------- loudness --

    fn counts(levelled: usize, total: usize, albums: usize) -> overlays::settings::LoudnessCounts {
        overlays::settings::LoudnessCounts {
            levelled,
            total,
            albums,
            files: albums * 10,
            ..Default::default()
        }
    }

    #[test]
    fn a_running_analysis_is_shown_whatever_else_is_true() {
        use overlays::settings::Loudness;
        let running = Some((3, 9, "Vheissu".to_owned()));
        for reachable in [true, false] {
            for scanning in [true, false] {
                assert_eq!(
                    loudness_state(counts(0, 9, 9), reachable, scanning, true, running.clone()),
                    Loudness::Running {
                        done: 3,
                        total: 9,
                        album: "Vheissu".to_owned()
                    }
                );
            }
        }
    }

    #[test]
    fn the_loudness_row_says_why_it_cannot_start() {
        use overlays::settings::Loudness;
        let c = counts(5, 10, 2);
        assert_eq!(
            loudness_state(c, false, false, true, None),
            Loudness::Unreachable(c)
        );
        assert_eq!(
            loudness_state(c, true, true, true, None),
            Loudness::Scanning(c)
        );
        assert_eq!(
            loudness_state(c, true, false, true, None),
            Loudness::Armed(c)
        );
        assert_eq!(
            loudness_state(c, true, false, false, None),
            Loudness::Ready(c)
        );
    }

    #[test]
    fn gaps_nothing_can_fill_are_not_offered_a_button() {
        use overlays::settings::Loudness;
        let c = counts(5, 10, 0);
        assert_eq!(
            loudness_state(c, true, false, false, None),
            Loudness::Stuck(c)
        );
        assert_eq!(
            loudness_state(counts(10, 10, 0), true, false, false, None),
            Loudness::AllLevelled
        );
        assert_eq!(
            loudness_state(counts(0, 0, 0), true, false, false, None),
            Loudness::None
        );
    }

    #[test]
    fn escape_answers_the_loudness_question_first_too() {
        let mut app = App::disconnected(String::new());
        app.set_overlay(Some(Overlay::Settings));
        app.gain_armed = true;
        let _ = app.on_action(Action::Dismiss);
        assert!(!app.gain_armed);
        assert_eq!(app.open_overlay_kind(), Some(Overlay::Settings));
    }

    #[test]
    fn confirming_an_analysis_nobody_armed_starts_nothing() {
        let mut app = App::disconnected(String::new());
        app.start_gain();
        assert!(app.gain_progress.is_none());
    }

    #[test]
    fn the_summary_names_each_exception_once() {
        use scoria_core::library::gain::{GainReport, Skipped};
        let clean = GainReport {
            albums_done: 312,
            albums_total: 312,
            ..Default::default()
        };
        assert_eq!(gain_summary(&clean), "Levelled 312 albums.");

        let messy = GainReport {
            albums_done: 41,
            albums_total: 312,
            stopped: true,
            skipped: Skipped {
                missing_files: 4,
                unsupported: 12,
            },
            failed: vec![("a".to_owned(), "b".to_owned()); 2],
        };
        assert_eq!(
            gain_summary(&messy),
            "Stopped after 41 of 312 albums. 4 skipped (missing files) · \
             12 skipped (format not yet supported) · 2 couldn't be written"
        );
    }

    #[test]
    fn a_failed_file_is_named_inside_the_library() {
        let root = std::path::Path::new("/home/someone/Music");
        assert_eq!(
            relative_to("/home/someone/Music/A/B/01.flac", Some(root)),
            "A/B/01.flac"
        );
        assert_eq!(
            relative_to("/elsewhere/x.flac", Some(root)),
            "/elsewhere/x.flac"
        );
    }

    #[test]
    fn the_cursor_stays_on_a_track_that_survived() {
        assert_eq!(refocus(Some(3), Some(9), 20), Some(3));
    }

    #[test]
    fn the_cursor_on_a_forgotten_track_lands_on_its_neighbour_not_the_top() {
        assert_eq!(refocus(None, Some(9), 20), Some(9));
        // It was near the end, and the end moved up past it.
        assert_eq!(refocus(None, Some(9), 4), Some(3));
        assert_eq!(refocus(None, Some(9), 0), None, "nothing left to land on");
        assert_eq!(refocus(None, None, 20), None, "there was no cursor");
    }

    #[test]
    fn right_clicking_a_row_outside_the_result_opens_nothing() {
        let mut app = App::disconnected(String::new());
        app.rows = vec![TrackId(11)];
        app.on_table(table::Event::Context(99));
        assert!(app.menu.is_none());
    }

    #[test]
    fn escape_closes_the_menu_before_it_closes_anything_else() {
        // The one rule the menu shares with §9's overlays. It is the most
        // recent thing to have opened, so it is what Escape is reaching for --
        // and an overlay behind it has to survive that first press.
        let mut app = App::disconnected(String::new());
        app.rows = vec![TrackId(11)];
        app.set_overlay(Some(Overlay::Help));
        app.on_table(table::Event::Context(0));
        assert!(app.menu.is_some());

        let _ = app.on_action(Action::Dismiss);
        assert!(app.menu.is_none(), "escape did not close the menu");
        assert_eq!(
            app.open_overlay_kind(),
            Some(Overlay::Help),
            "escape closed the drawer behind the menu as well"
        );

        let _ = app.on_action(Action::Dismiss);
        assert_eq!(app.open_overlay_kind(), None, "a second escape did nothing");
    }

    #[test]
    fn being_told_the_offset_it_already_had_does_not_dismiss_the_menu() {
        // The second guard on the bug that made this work on a filtered view
        // and never on the whole library. Rebuilding the widget tree reset the
        // scrollable, which reported offset 0, which closed the menu on the
        // very next event. The tree is stable now; this makes a spurious
        // report harmless even if something else ever produces one.
        let mut app = App::disconnected(String::new());
        app.rows = vec![TrackId(11)];
        app.on_table(table::Event::Scrolled {
            offset: 0.0,
            viewport: 600.0,
        });
        app.on_table(table::Event::Context(0));
        assert!(app.menu.is_some());

        app.on_table(table::Event::Scrolled {
            offset: 0.0,
            viewport: 600.0,
        });
        assert!(
            app.menu.is_some(),
            "a relayout reporting the same offset closed the menu"
        );
    }

    #[test]
    fn scrolling_dismisses_the_menu() {
        // It was opened on a row, and the row has moved out from under it.
        let mut app = App::disconnected(String::new());
        app.rows = vec![TrackId(11)];
        app.on_table(table::Event::Context(0));
        app.on_table(table::Event::Scrolled {
            offset: 120.0,
            viewport: 600.0,
        });
        assert!(app.menu.is_none());
    }

    #[test]
    fn the_repeat_button_cycles_and_comes_back() {
        // §6.6, and the same shape as the layout cycle: three presses must
        // return to where they started, or the button is a trap.
        let mut mode = RepeatMode::Off;
        let mut seen = vec![mode];
        for _ in 0..2 {
            mode = next_repeat(mode);
            seen.push(mode);
        }
        assert_eq!(next_repeat(mode), RepeatMode::Off);
        let distinct: std::collections::HashSet<String> =
            seen.iter().map(|mode| format!("{mode:?}")).collect();
        assert_eq!(distinct.len(), 3, "cycling skipped a mode");
    }

    #[test]
    fn a_scan_says_what_it_did_in_words() {
        use scoria_core::event::LibraryDelta as D;

        assert!(describe(&D::TracksAdded(vec![TrackId(1), TrackId(2)]), 40).contains('2'));
        assert!(describe(&D::TracksChanged(vec![TrackId(1)]), 40).contains("retagged"));
        // §7.4: a vanished file is struck through, not removed, so this is a
        // change in how rows look rather than in how many there are.
        let availability = describe(&D::AvailabilityChanged(vec![TrackId(1)]), 40);
        assert!(availability.contains("missing") || availability.contains("came back"));
        assert!(
            !availability.contains("40"),
            "it reported a count that did not change"
        );
    }

    #[test]
    fn the_properties_summary_states_absent_metadata_rather_than_inventing_it() {
        // A file with nothing but the tags every track has. Built here rather
        // than borrowed from a fixture, because the point of the test is
        // exactly which fields are absent.
        let bare = Track {
            id: TrackId(1),
            album_id: scoria_core::AlbumId(1),
            path: scoria_core::RelativePath::new("a/01.flac").unwrap(),
            title: "Untitled".to_owned(),
            artist: "Someone".to_owned(),
            album_artist: "Someone".to_owned(),
            album_artist_sort: None,
            album: "An Album".to_owned(),
            date: None,
            disc: None,
            track_number: None,
            duration: Duration::from_secs(200),
            replay_gain: scoria_core::ReplayGain::default(),
            added_at: scoria_core::Timestamp(0),
            play_count: 0,
            genre: None,
            composer: None,
            audio: scoria_core::model::AudioProperties::default(),
            available: true,
        };

        let summary = track_summary(&bare);
        let keys: Vec<&str> = summary.iter().map(|(key, _)| *key).collect();

        // The fields that are always there.
        for key in ["Title", "Artist", "Album", "Duration", "Path", "ReplayGain"] {
            assert!(keys.contains(&key), "{key} is missing from the summary");
        }
        // And the ones that are not, when the tags do not say.
        for key in ["Date", "Disc", "Track", "Album artist sort"] {
            assert!(!keys.contains(&key), "{key} was invented");
        }
        // §6.7: untagged means unity, and saying so here saves the question
        // being asked of the loudness later.
        let gain = summary
            .iter()
            .find(|(key, _)| *key == "ReplayGain")
            .map(|(_, value)| value.as_str());
        assert_eq!(gain, Some("not tagged (plays at 0 dB)"));
    }

    /// Serialises the tests that publish a keymap.
    ///
    /// It is process-wide now rather than thread-local -- which is the fix --
    /// and the cost of that is that two tests publishing different keymaps at
    /// once will read each other's. Caught as a one-in-five flake rather than
    /// in CI a month from now, which is the argument for running the suite
    /// more than once after a change like this.
    fn published() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The pair iced delivers for one press: the key without modifiers, and
    /// the key with them applied.
    ///
    /// Written out because the first attempt at fixing `?` tested a
    /// `Key::Character("?")` that this application never receives, and so
    /// proved nothing about the press that produces it.
    fn press(key: &str, typed: &str, modifiers: Modifiers) -> Option<Action> {
        publish_keymap(&Keymap::standard());
        resolve_key(
            &Key::Character(key.into()),
            &Key::Character(typed.into()),
            modifiers,
        )
    }

    #[test]
    fn what_the_application_publishes_reaches_the_subscription_thread() {
        let _published = published();
        // The bug the previous test could not see. `listen_with` runs on a
        // thread pool -- iced turns that feature on by default -- while
        // `update` and `view` run on the main thread. Held in a thread-local,
        // the keymap was published on one and read on the other, and the
        // reader simply saw its own untouched initial value. It looked
        // published only because that value was `Keymap::standard()` already.
        //
        // The earlier test exercised both halves on one thread, so it passed
        // throughout. This one does not, which is the whole point of it.
        let vim = Keymap {
            name: "Vim".to_owned(),
            bindings: vec![Binding::new(
                Trigger::Char('j'),
                crate::keymap::Mods::NONE,
                Action::MoveDown,
            )],
        };
        publish_keymap(&vim);

        let resolved = std::thread::spawn(|| {
            resolve_key(
                &Key::Character("j".into()),
                &Key::Character("j".into()),
                Modifiers::empty(),
            )
        })
        .join();

        assert_eq!(
            resolved.ok().flatten(),
            Some(Action::MoveDown),
            "the keymap did not cross the thread it is read on"
        );
        publish_keymap(&Keymap::standard());
    }

    #[test]
    fn the_pointer_crosses_from_the_subscription_thread_to_the_view() {
        // The same fault, the other way round, and this one had no lucky
        // default to hide behind: the pointer is recorded by the subscription
        // and read by `view`, so a thread-local meant `view` always saw the
        // origin and the menu always opened in the corner of the window.
        std::thread::spawn(|| shared::set_pointer(iced::Point::new(640.0, 480.0)))
            .join()
            .ok();

        let seen = pointer();
        assert!(
            (seen.x - 640.0).abs() < f32::EPSILON && (seen.y - 480.0).abs() < f32::EPSILON,
            "the pointer did not cross threads: {seen:?}"
        );
    }

    #[test]
    fn shift_slash_opens_the_cheatsheet_rather_than_the_search() {
        let _published = published();
        // The press as iced actually reports it: `key` is the key without
        // modifiers, so `/`, and `modified_key` is what it typed, so `?`.
        // Reading only the first meant `?` could not be bound at all -- and
        // then discarding shift to compensate made Shift+/ open the search,
        // because `/` is what it looked like.
        assert_eq!(
            press("/", "?", Modifiers::SHIFT),
            Some(Action::ToggleHelp),
            "shift-slash did not reach the cheatsheet"
        );
    }

    #[test]
    fn a_bare_slash_still_opens_the_search() {
        let _published = published();
        // The binding `?` displaced. Unshifted, both keys are `/`.
        assert_eq!(
            press("/", "/", Modifiers::empty()),
            Some(Action::FocusSearch)
        );
    }

    #[test]
    fn a_control_chord_survives_the_modified_key() {
        let _published = published();
        // Ctrl chords are on the key, not on what it would have typed, and a
        // platform is free to report something odd as the modified key. The
        // fallback to the unmodified key is what keeps them working.
        assert_eq!(
            press("s", "s", Modifiers::CTRL),
            Some(Action::ToggleShuffle)
        );
        assert_eq!(
            press("s", "\u{13}", Modifiers::CTRL),
            Some(Action::ToggleShuffle),
            "a control character as the modified key lost the chord"
        );
    }

    #[test]
    fn key_presses_resolve_against_the_keymap_the_application_published() {
        let _published = published();
        // The seam §10 is built around, and the one that was not connected:
        // `App::keymap` fed the cheatsheet while key presses resolved against
        // a thread-local nothing ever wrote. Both were `Keymap::standard()`,
        // so the gap was invisible -- until a second keymap became a value
        // someone could choose, which is the entire point of §10.
        let vim = Keymap {
            name: "Vim".to_owned(),
            bindings: vec![
                Binding::new(
                    Trigger::Char('j'),
                    crate::keymap::Mods::NONE,
                    Action::MoveDown,
                ),
                Binding::new(
                    Trigger::Char('k'),
                    crate::keymap::Mods::NONE,
                    Action::MoveUp,
                ),
            ],
        };
        publish_keymap(&vim);

        assert_eq!(
            resolve_key(
                &Key::Character("j".into()),
                &Key::Character("j".into()),
                Modifiers::empty()
            ),
            Some(Action::MoveDown),
            "the published keymap did not reach the resolver"
        );
        // And the standard map's bindings are gone, rather than both applying.
        assert_eq!(
            resolve_key(
                &Key::Named(iced::keyboard::key::Named::Space),
                &Key::Named(iced::keyboard::key::Named::Space),
                Modifiers::empty()
            ),
            None
        );

        // Publishing again swaps it back, so the map really is a value.
        publish_keymap(&Keymap::standard());
        assert_eq!(
            resolve_key(
                &Key::Named(iced::keyboard::key::Named::Space),
                &Key::Named(iced::keyboard::key::Named::Space),
                Modifiers::empty()
            ),
            Some(Action::PlayPause)
        );
        assert_eq!(
            resolve_key(
                &Key::Character("j".into()),
                &Key::Character("j".into()),
                Modifiers::empty()
            ),
            None
        );
    }
}
