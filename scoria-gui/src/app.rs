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
use crate::theme::Theme;
use crate::widgets::{panes, table, transport};
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
    Queue(table::Event),
    CloseOverlay,
    SearchChanged(String),
    /// A key press, already resolved through the keymap.
    Action(Action),
}

/// Everything the window shows.
pub struct App {
    /// `None` when the library could not be opened. The window still opens and
    /// says why: §5.1 makes first run a "point me at your library" moment, and
    /// a failure to read the database is the same shape of problem.
    session: Option<Session>,
    theme: Theme,
    keymap: Keymap,
    placement: BrowserPlacement,

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

    /// The row the keyboard is on. Selection is deliberately independent of
    /// playback -- §6.1: browsing never disturbs what is playing, and the two
    /// are allowed to disagree.
    cursor: Option<usize>,

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

    /// Which overlay is open. One `Option`, so §9's "one at a time, never
    /// stacked" cannot be expressed otherwise.
    overlay: Option<Overlay>,
    /// The playback snapshot, as the engine last reported it. This is what the
    /// queue shows -- §6.2's `tracks`, not the current view.
    queue: Option<scoria_core::PlaybackSource>,
    queue_lines: Vec<table::Line>,
    queue_scroll: f32,
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
        let session = Session::open(paths)?;
        let mut app = Self::with_session(Some(session), String::new());
        app.refresh();
        app.begin_watching();
        Ok(app)
    }

    /// A window with no library behind it, showing why.
    #[must_use]
    pub fn disconnected(reason: String) -> Self {
        Self::with_session(None, reason)
    }

    fn with_session(session: Option<Session>, status: String) -> Self {
        Self {
            session,
            theme: Theme::dark(),
            keymap: Keymap::standard(),
            placement: BrowserPlacement::default(),
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
            overlay: None,
            queue: None,
            queue_lines: Vec::new(),
            queue_scroll: 0.0,
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
            Ok(()) => {}
            Err(scoria_core::error::CoreError::NoLibraryRoot) => {
                self.status = "no library root set -- run `scoria scan <path>`".to_owned();
            }
            Err(e) => self.status = format!("cannot watch the library: {e}"),
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
                if self.cursor.is_some_and(|c| c >= self.rows.len()) {
                    self.cursor = None;
                }
            }
            Err(e) => self.status = format!("query failed: {e}"),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => self.drain_core_events(),
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
            Message::Queue(event) => return self.on_queue(event),
            Message::CloseOverlay => self.overlay = None,
            Message::ArtistPane(event) => self.on_artist_pane(event),
            Message::AlbumPane(event) => self.on_album_pane(event),
            Message::Action(action) => return self.on_action(action),
        }
        Task::none()
    }

    /// Reads the core's event stream. The only direction information flows in.
    fn drain_core_events(&mut self) {
        let Some(session) = &self.session else { return };
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
                    self.status = match total {
                        Some(total) => format!("scanning {seen}/{total}"),
                        None => format!("scanning {seen}"),
                    };
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
            table::Event::Select(index) => self.cursor = Some(index),
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
                self.cursor = Some(index);
                let album = self
                    .rows
                    .get(index)
                    .and_then(|id| self.track(*id))
                    .map(|t| t.album_id);
                self.overlay = Some(Overlay::Artwork(album));
            }
            table::Event::Scrolled { offset, viewport } => {
                self.scroll_offset = offset;
                self.viewport_height = viewport;
            }
        }
    }

    /// The queue's own table events. §9.2: double-click sets `position`.
    fn on_queue(&mut self, event: table::Event) -> Task<Message> {
        match event {
            table::Event::Activate(index) => self.send(Command::SetPosition(index)),
            table::Event::Scrolled { offset, viewport } => {
                self.queue_scroll = offset;
                self.viewport_height = viewport;
            }
            // Selection in the queue *is* the position, so a single click
            // marking a row would imply something it cannot do.
            table::Event::Select(_) | table::Event::Context(_) => {}
        }
        Task::none()
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
        self.overlay = overlays::toggle(self.overlay, requested);

        // Reading a file's tags is I/O, so it happens once on open rather than
        // in the view, which runs every frame.
        if let Some(Overlay::Properties(id)) = self.overlay {
            self.load_properties(id);
        }

        // The queue opens showing where you are, which means scrolled to it.
        // §9.2 asks for the current position marked *and* scrolled into view.
        if self.overlay == Some(Overlay::Queue) {
            let position = self
                .queue
                .as_ref()
                .and_then(scoria_core::PlaybackSource::position);
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
            transport::Event::ShowArtwork => {
                self.overlay = overlays::toggle(self.overlay, Overlay::Artwork(None));
                return Task::none();
            }
        };
        self.send(command);
        Task::none()
    }

    fn on_action(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::MoveDown => return self.move_cursor(1),
            Action::MoveUp => return self.move_cursor(-1),
            Action::PageDown => return self.move_cursor(self.page()),
            Action::PageUp => return self.move_cursor(-self.page()),
            Action::MoveToStart => {
                self.cursor = (!self.rows.is_empty()).then_some(0);
                return self.scroll_to_cursor();
            }
            Action::MoveToEnd => {
                self.cursor = self.rows.len().checked_sub(1);
                return self.scroll_to_cursor();
            }
            // Multiple selection arrives with the selection model at M10; the
            // actions exist now so the keymap is complete.
            Action::ExtendUp => return self.move_cursor(-1),
            Action::ExtendDown => return self.move_cursor(1),
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
                self.placement = self.placement.cycle();
                self.status = self.placement.label().to_owned();
            }
            Action::FocusSearch => {
                return iced::widget::operation::focus(iced::widget::Id::new(SEARCH_ID));
            }
            // Overlays arrive at M9.
            Action::ToggleQueue => return self.open_overlay(Overlay::Queue),
            Action::ShowProperties => {
                if let Some(id) = self.cursor.and_then(|index| self.rows.get(index).copied()) {
                    return self.open_overlay(Overlay::Properties(id));
                }
                self.status = "select a track first".to_owned();
            }
            // The keybinding cheatsheet is a §8 promise the overlays make
            // possible; it arrives with the column work at M10.
            Action::ToggleHelp => {}
            Action::Dismiss => {
                // §9: Escape closes the overlay. With none open it clears the
                // filters instead -- one key backing out of whatever is
                // currently narrowing the view.
                if self.overlay.take().is_none() {
                    self.clear_filters();
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
    fn play_from_cursor(&mut self) {
        let Some(index) = self.cursor else { return };
        let Some(at) = self.rows.get(index).copied() else {
            return;
        };

        let Some(session) = &self.session else { return };
        let label = self.selection.label();
        let spec = session.source_spec(self.rows.clone(), label);
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

    fn move_cursor(&mut self, delta: isize) -> Task<Message> {
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
        self.cursor = Some(next);
        self.scroll_to_cursor()
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
        let table_view = table::TableView {
            lines: &self.lines,
            rows: self.tracks(),
            selected: self.cursor,
            playing: self.playing,
            scroll_offset: self.scroll_offset,
            theme: &self.theme,
            id: TABLE_ID,
        };

        let now_playing = self.playing.and_then(|id| self.track(id));
        let transport_view = transport::TransportView {
            track: now_playing,
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

        let search = container(header).padding([6, 10]).width(Fill);

        // Built only when a placement will use it: `Off` shows no panes, and
        // constructing them would be work thrown away every frame.
        let browser = (self.placement != BrowserPlacement::Off).then(|| Browser {
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

        let base = layouts::arrange(
            self.placement,
            table::view(table_view, Message::Table),
            transport::view(&transport_view, Message::Transport),
            search.into(),
            browser,
            &self.theme,
        );

        match self.overlay {
            None => base,
            Some(overlay) => overlays::present(
                base,
                overlay,
                self.overlay_content(overlay),
                &self.theme,
                Message::CloseOverlay,
            ),
        }
    }

    fn overlay_content(&self, overlay: Overlay) -> Element<'_, Message, Theme> {
        match overlay {
            Overlay::Queue => overlays::queue::view(
                overlays::queue::QueueView {
                    lines: &self.queue_lines,
                    rows: self.queue_rows(),
                    position: self
                        .queue
                        .as_ref()
                        .and_then(scoria_core::PlaybackSource::position),
                    playing: self.playing,
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
                    image: album.and_then(|id| self.session.as_ref()?.artwork().get(id)),
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
            .and_then(|track| self.session.as_ref()?.artwork().get(track.album_id));

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
