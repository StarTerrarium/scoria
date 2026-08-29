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
use crate::settings::Settings;
use crate::theme::Theme;
use crate::widgets::columns::Column;
use crate::widgets::{panes, table, transport, welcome};
use iced::widget::{button, container, row, text, text_input};
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
    sort_override: Option<(scoria_core::SortField, scoria_core::Direction)>,
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
    albums: Vec<(scoria_core::AlbumId, panes::PaneRow)>,
    album_total: usize,
    artist_scroll: f32,
    album_scroll: f32,

    /// The row the keyboard is on, and the rows selected.
    ///
    /// Selection is deliberately independent of playback -- §6.1: browsing
    /// never disturbs what is playing, and the two are allowed to disagree.
    ///
    /// `anchor` is where a range selection started, so shift-clicking or
    /// shift-arrowing extends from the same place rather than from wherever the
    /// cursor happens to have reached.
    cursor: Option<usize>,
    selected: std::collections::BTreeSet<usize>,
    anchor: Option<usize>,
    /// Modifiers currently held, so a click can tell plain from shift from
    /// control. Pointer events do not carry them.
    modifiers: crate::keymap::Mods,

    playing: Option<TrackId>,
    state: PlaybackState,
    position: Duration,
    /// Set while the seek bar is being dragged. See `TransportView::scrubbing`.
    scrubbing: Option<Duration>,
    shuffled: bool,
    repeat: RepeatMode,
    volume: f32,

    scroll_offset: f32,
    viewport_height: f32,
    status: String,
    /// A scan in flight, as the last `ScanProgress` reported it.
    ///
    /// Held rather than left to the status line, because on first run it is the
    /// difference between "this is taking a while" and "this found nothing",
    /// and the two need different words.
    scanning: Option<welcome::Scan>,

    /// Which overlay is open. One `Option`, so §9's "one at a time, never
    /// stacked" cannot be expressed otherwise.
    /// What is on screen, including anything still retracting.
    overlay: Option<overlays::Presenting>,
    dragging_drawer: bool,
    /// The playback snapshot, as the engine last reported it. This is what the
    /// queue shows -- §6.2's `tracks`, not the current view.
    queue: Option<scoria_core::PlaybackSource>,
    queue_lines: Vec<table::Line>,
    queue_scroll: f32,
    /// The row the queue's own cursor is on, independent of what is playing.
    ///
    /// A set of one, because the table it is drawn with takes a selection.
    queue_selection: std::collections::BTreeSet<usize>,
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
            sort_override: None,
            resizing_column: None,
            reordering: None,
            selection: Selection::all(),
            rows: Vec::new(),
            lines: Vec::new(),
            search: String::new(),
            artists: Vec::new(),
            artist_total: 0,
            albums: Vec::new(),
            album_total: 0,
            artist_scroll: 0.0,
            album_scroll: 0.0,
            cursor: None,
            selected: std::collections::BTreeSet::new(),
            anchor: None,
            modifiers: crate::keymap::Mods::NONE,
            playing: None,
            state: PlaybackState::Stopped,
            position: Duration::ZERO,
            scrubbing: None,
            shuffled: false,
            repeat: RepeatMode::Off,
            volume: 1.0,
            scroll_offset: 0.0,
            viewport_height: 650.0,
            status,
            scanning: None,
            overlay: None,
            dragging_drawer: false,
            queue: None,
            queue_lines: Vec::new(),
            queue_scroll: 0.0,
            queue_selection: std::collections::BTreeSet::new(),
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
                self.albums = outcome
                    .albums
                    .rows
                    .into_iter()
                    .map(|r| {
                        (
                            r.id,
                            panes::PaneRow {
                                label: r.label,
                                count: r.count,
                                selected: r.selected,
                            },
                        )
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
                self.selected.retain(|index| *index < self.rows.len());
                self.anchor = self.anchor.filter(|a| *a < self.rows.len());
                if self.cursor.is_some_and(|c| c >= self.rows.len()) {
                    self.cursor = None;
                }
            }
            Err(e) => self.status = format!("query failed: {e}"),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                self.drain_core_events();
                self.tick_overlay();
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
                    self.queue = Some(source);
                }
                CoreEvent::LibraryChanged(delta) => {
                    // The scan happened on another thread against its own
                    // database handle, so this session's index is a stale copy
                    // until it is told to re-read.
                    library_changed = Some(delta);
                }
                CoreEvent::ScanProgress { seen, total } => {
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
                CoreEvent::QueryResult(..) => {}
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
        let anchored = self.cursor.and_then(|index| self.rows.get(index).copied());
        self.refresh();
        if let Some(track) = anchored {
            self.cursor = self.rows.iter().position(|id| *id == track);
        }

        self.status = describe(delta, self.rows.len());
    }

    fn on_table(&mut self, event: table::Event) {
        match event {
            table::Event::Select(index) => self.select(index, self.modifiers),
            table::Event::Activate(index) => {
                self.cursor = Some(index);
                self.play_from_cursor();
            }
            // The right-click menu arrives with the overlays at M9. §9.1 makes
            // "Show album art" the one escape hatch from artwork following
            // playback, so it belongs there rather than here.
            // §9.1's one escape hatch from artwork following playback: a
            // right-click opens the overlay for *that* album, without turning
            // into a follow-selection mode.
            table::Event::Context(index) => {
                // A right-click on an unselected row moves the selection to it;
                // on a selected one it leaves the selection alone, so acting on
                // several rows is not lost to a stray click.
                if !self.selected.contains(&index) {
                    self.select(index, crate::keymap::Mods::NONE);
                }
                let album = self
                    .rows
                    .get(index)
                    .and_then(|id| self.track(*id))
                    .map(|t| t.album_id);
                self.set_overlay(Some(Overlay::Artwork(album)));
            }
            table::Event::Scrolled { offset, viewport } => {
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
                    if let Some(setting) = self.settings.columns.visible().get(reorder.from) {
                        self.sort_by(setting.column);
                    }
                } else {
                    self.settings.columns.move_visible(reorder.from, reorder.to);
                    self.persist();
                }
            }
        }
    }

    /// How many tracks carry ReplayGain tags (§6.7).
    fn gain_tagged(&self) -> usize {
        self.session.as_ref().map_or(0, |session| {
            session
                .index()
                .tracks()
                .iter()
                .filter(|t| t.replay_gain.is_tagged())
                .count()
        })
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
                self.sort_override = None;
                self.persist();
                self.apply_sort();
            }
            E::ClearOverride => {
                self.sort_override = None;
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
        use scoria_core::Direction;
        let field = column.sort_field();

        self.sort_override = match self.sort_override {
            // Clicking the same heading again reverses it, and a third time
            // returns to the configured sort -- so the override is always one
            // click from being gone.
            Some((current, Direction::Ascending)) if current == field => {
                Some((field, Direction::Descending))
            }
            Some((current, Direction::Descending)) if current == field => None,
            _ => Some((field, Direction::Ascending)),
        };
        self.apply_sort();
    }

    /// Rebuilds the sort chain from the configured preset and any override.
    fn apply_sort(&mut self) {
        let configured = self.settings.sort.chain();
        self.selection.sort = match self.sort_override {
            None => configured,
            Some((field, direction)) => {
                // On top of, not instead of: the configured chain still breaks
                // ties, so an override on a coarse field keeps albums in order
                // within it.
                let mut keys = vec![scoria_core::SortKey { field, direction }];
                keys.extend(configured.keys().iter().copied());
                scoria_core::SortChain::new(keys)
            }
        };
        self.refresh();
    }

    /// Writes the frontend's settings, reporting rather than failing.
    fn persist(&mut self) {
        self.settings.placement = self.placement;
        if let Err(e) = self.settings.save(&self.paths) {
            self.status = format!("cannot save settings: {e}");
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
                if let Some((id, _)) = self.albums.get(index) {
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
            .albums
            .iter()
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
        self.cursor = None;
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
            // The artwork overlay arrives at M9.
            // §9.1: the resting state is the thumbnail; clicking it opens the
            // playing track's art full size.
            transport::Event::ToggleQueue => return self.open_overlay(Overlay::Queue),
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
                if let Some(id) = self.cursor.and_then(|index| self.rows.get(index).copied()) {
                    return self.open_overlay(Overlay::Properties(id));
                }
                self.status = "select a track first".to_owned();
            }
            Action::ToggleHelp => return self.open_overlay(Overlay::Help),
            Action::Dismiss => {
                // §9: Escape closes the overlay. With none open it clears the
                // filters instead -- one key backing out of whatever is
                // currently narrowing the view.
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
            self.cursor = Some(index);
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
        let Some(cursor) = self.cursor else {
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

    /// Turns the cursor into a snapshot and plays it.
    ///
    /// The §6.3 setting decides what gets snapshotted, and the session is the
    /// one place that decision is made -- which is why this hands over what is
    /// shown rather than deciding for itself.
    /// Applies a click to the selection.
    ///
    /// Plain replaces, shift extends from the anchor, control toggles. The
    /// anchor is where a range started rather than wherever the cursor has
    /// reached, so extending twice grows one range instead of walking one.
    fn select(&mut self, index: usize, modifiers: crate::keymap::Mods) {
        if index >= self.rows.len() {
            return;
        }
        self.cursor = Some(index);

        if modifiers.shift {
            let anchor = self.anchor.unwrap_or(index);
            let (from, to) = if anchor <= index {
                (anchor, index)
            } else {
                (index, anchor)
            };
            self.selected = (from..=to).collect();
            return;
        }

        if modifiers.ctrl {
            if !self.selected.remove(&index) {
                self.selected.insert(index);
            }
            self.anchor = Some(index);
            return;
        }

        self.selected = std::iter::once(index).collect();
        self.anchor = Some(index);
    }

    /// Moves the cursor, replacing or extending the selection.
    fn move_selection(&mut self, delta: isize, extend: bool) -> Task<Message> {
        if self.rows.is_empty() {
            return Task::none();
        }
        let last = self.rows.len().saturating_sub(1);
        let next = match self.cursor {
            None => {
                if delta < 0 {
                    last
                } else {
                    0
                }
            }
            Some(current) => current.saturating_add_signed(delta).min(last),
        };

        self.select(
            next,
            crate::keymap::Mods {
                shift: extend,
                ctrl: false,
                alt: false,
            },
        );
        self.scroll_to_cursor()
    }

    fn play_from_cursor(&mut self) {
        let Some(index) = self.cursor else { return };
        let Some(at) = self.rows.get(index).copied() else {
            return;
        };
        let Some(session) = &self.session else { return };

        // Several rows chosen means "play these", which is what a materialised
        // snapshot is for (§6.2): the chosen tracks *are* the play order, and
        // nothing re-derives them afterwards.
        let spec = if self.selected.len() > 1 {
            let tracks: Vec<_> = self
                .selected
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
        let Some(cursor) = self.cursor else {
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

    fn tracks(&self) -> Vec<&Track> {
        self.session
            .as_ref()
            .map_or_else(Vec::new, |s| s.index().rows(&self.rows))
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
            rows: self.tracks(),
            selected: &self.selected,
            playing: self.playing,
            playing_row: None,
            columns: &self.settings.columns,
            sort: self.sort_override,
            resizing: self.resizing_column.is_some(),
            reordering: self.reordering,
            scroll_offset: self.scroll_offset,
            theme: &self.theme,
            id: TABLE_ID,
        };

        let now_playing = self.playing.and_then(|id| self.track(id));
        let transport_view = transport::TransportView {
            track: now_playing,
            artwork: now_playing.and_then(|t| self.session.as_ref()?.artwork_for(t.album_id)),
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

        // Where the browser sits (§7.1). Three buttons rather than one that
        // cycles: a mouse should point at the layout it wants rather than
        // clicking twice to pass through the one it does not, and three
        // buttons say what the alternatives are, which a cycling key never
        // could. The keystroke stays a cycle, because that is what a keystroke
        // is good at.
        header = header.push(text("Browser").size(11).style(move |_t: &Theme| muted));
        for placement in [
            BrowserPlacement::Off,
            BrowserPlacement::Top,
            BrowserPlacement::Left,
        ] {
            let current = placement == self.placement;
            header = header.push(
                button(text(placement.short_label()).size(12))
                    .padding([3, 8])
                    .on_press(Message::SetPlacement(placement))
                    .style(move |t: &Theme, status| t.transport_button(status, current)),
            );
        }

        // Both of these have keybindings, and a keybinding is not a way in for
        // anyone who has not been told about it. Words rather than glyphs:
        // there is room here, and a word cannot fail to render the way a
        // missing symbol can.
        for (label, message, open) in [
            (
                "Columns",
                Message::Action(Action::ToggleColumns),
                self.open_overlay_kind() == Some(Overlay::Columns),
            ),
            (
                "Settings",
                Message::Action(Action::ToggleSettings),
                self.open_overlay_kind() == Some(Overlay::Settings),
            ),
            // Appended rather than inserted: the other two have been in the
            // same place for a while, and a new button should not move them.
            (
                "Shortcuts",
                Message::Action(Action::ToggleHelp),
                self.open_overlay_kind() == Some(Overlay::Help),
            ),
        ] {
            header = header.push(
                button(text(label).size(12))
                    .padding([3, 10])
                    .on_press(message)
                    .style(move |t: &Theme, status| t.transport_button(status, open)),
            );
        }

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
                        rows: self.artists.clone(),
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
                        rows: self.albums.iter().map(|(_, row)| row.clone()).collect(),
                        total: self.album_total,
                        scroll_offset: self.album_scroll,
                        theme: &self.theme,
                        id: ALBUM_PANE_ID,
                    },
                    Message::AlbumPane,
                ),
                artwork: self.artwork_view(),
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

        layouts::arrange(
            self.placement,
            body,
            transport::view(&transport_view, Message::Transport),
            search.into(),
            browser,
            overlay,
            &self.theme,
        )
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
                    gain_tagged: self.gain_tagged(),
                    track_count: self.session.as_ref().map_or(0, |s| s.index().len()),
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
                    overridden: self.sort_override.is_some(),
                    theme: &self.theme,
                },
                Message::Columns,
            ),
            Overlay::Queue => overlays::queue::view(
                overlays::queue::QueueView {
                    columns: &self.settings.columns,
                    lines: &self.queue_lines,
                    rows: self.queue_rows(),
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

    /// The snapshot's tracks, resolved through the index.
    fn queue_rows(&self) -> Vec<&Track> {
        let Some(queue) = &self.queue else {
            return Vec::new();
        };
        let Some(session) = &self.session else {
            return Vec::new();
        };
        session.index().rows(queue.tracks())
    }

    /// A caption naming the album some artwork belongs to.
    fn album_caption(&self, album: scoria_core::AlbumId) -> Option<String> {
        let track = self
            .session
            .as_ref()?
            .index()
            .tracks()
            .iter()
            .find(|t| t.album_id == album)?;
        Some(format!("{} — {}", track.album_artist, track.album))
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

    /// The now-playing album's artwork, or a placeholder.
    ///
    /// §9.1: artwork always follows what is playing. There is no mode and no
    /// toggle, here or anywhere else it appears.
    fn artwork_view(&self) -> Element<'_, Message, Theme> {
        let path = self
            .playing
            .and_then(|id| self.track(id))
            .and_then(|track| self.session.as_ref()?.artwork_for(track.album_id));

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
        let keymap = self.keymap.clone();
        let keys = iced::event::listen_with(move |event, status, _window| {
            // A key typed into the search field belongs to the search field.
            if status == iced::event::Status::Captured {
                return None;
            }
            match event {
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    ref key,
                    modifiers,
                    ..
                }) => KEYMAP
                    .with(|k| k.borrow().resolve_iced(key, modifiers))
                    .map(Message::Action),
                // Tracked separately because a mouse click carries no
                // modifiers of its own, and shift-click has to know.
                iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(modifiers)) => Some(
                    Message::Modifiers(crate::keymap::Mods::from_iced(modifiers)),
                ),
                _ => None,
            }
        });
        let _ = keymap;

        Subscription::batch([iced::time::every(POLL).map(|_| Message::Tick), keys])
    }
}

thread_local! {
    /// The keymap the subscription resolves against.
    ///
    /// `listen_with` takes a plain `fn`, not a closure, so the map cannot be
    /// captured. It is thread-local rather than global because iced's event
    /// loop is single-threaded, and this keeps the keymap a value that can be
    /// swapped -- which is the whole point of §10.
    static KEYMAP: std::cell::RefCell<Keymap> = std::cell::RefCell::new(Keymap::standard());
}

const fn next_repeat(current: RepeatMode) -> RepeatMode {
    match current {
        RepeatMode::Off => RepeatMode::All,
        RepeatMode::All => RepeatMode::One,
        RepeatMode::One => RepeatMode::Off,
    }
}

/// A one-line summary of what a scan changed, for the status bar.
fn describe(delta: &scoria_core::event::LibraryDelta, showing: usize) -> String {
    use scoria_core::event::LibraryDelta as D;
    match delta {
        D::TracksAdded(ids) => format!("{} track(s) added -- showing {showing}", ids.len()),
        D::TracksRemoved(ids) => format!("{} track(s) removed -- showing {showing}", ids.len()),
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
