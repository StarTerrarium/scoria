//! What a key press means.
//!
//! `DESIGN.md` §10: two keymaps resolve to one shared action enum -- the same
//! enum the TUI will drive. Actions are the vocabulary of the interface; the
//! keymap is one way of reaching them, and a menu or a button is another.
//!
//! Note what is *not* here. There is no `Action::Play(TrackId)`: actions say
//! what the user asked for, and turning "play the selection" into a
//! `Command::PlayFrom` with a materialised snapshot is the application's job.
//! Keeping ids out of actions is what lets the same enum serve a TUI whose
//! selection model is its own.

use serde::{Deserialize, Serialize};

/// Something the user can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    // Moving around the table.
    MoveUp,
    MoveDown,
    PageUp,
    PageDown,
    MoveToStart,
    MoveToEnd,
    ExtendUp,
    ExtendDown,

    /// Play whatever is selected, snapshotting per the §6.3 setting.
    PlaySelection,

    // Transport (§8).
    PlayPause,
    Next,
    Previous,
    ToggleShuffle,
    CycleRepeat,

    /// Restore an unfiltered library view and scroll to the playing track
    /// (§6.5).
    JumpToPlaying,
    /// Rescan the library now.
    ///
    /// §5.2: the root is watched continuously *and* an explicit rescan is
    /// always available. Watching cannot see everything -- a network mount, a
    /// filesystem that does not report events -- so the manual path is not a
    /// convenience.
    Rescan,
    /// Cycle `BrowserPlacement` (§7.1). One keystroke, three layouts.
    CycleLayout,
    FocusSearch,

    // Overlays (§9). One at a time, escape closes, the opening key toggles.
    ToggleQueue,
    /// Column visibility, order and the sort presets (§7.3).
    ToggleColumns,
    /// The settings with nowhere else to live (§9.4).
    ToggleSettings,
    /// A metadata dump for the selected track (§9.3).
    ShowProperties,
    ToggleHelp,
    /// Back out of whatever is currently narrowing the view.
    ///
    /// One action with a precedence rather than two bindings competing for
    /// Escape: close the overlay if one is open, otherwise clear the filters.
    /// That is how Escape behaves everywhere else, and it means the key always
    /// does the least surprising thing available.
    ///
    /// The filter half matters because filters survive a layout change, and
    /// `Off` shows no panes -- without this there is a state you can enter and
    /// not leave.
    Dismiss,
}

impl Action {
    /// Every action, so a keymap can be checked for gaps.
    #[must_use]
    pub const fn all() -> [Self; 24] {
        [
            Self::MoveUp,
            Self::MoveDown,
            Self::PageUp,
            Self::PageDown,
            Self::MoveToStart,
            Self::MoveToEnd,
            Self::ExtendUp,
            Self::ExtendDown,
            Self::PlaySelection,
            Self::PlayPause,
            Self::Next,
            Self::Previous,
            Self::ToggleShuffle,
            Self::CycleRepeat,
            Self::JumpToPlaying,
            // Ordered as the cheatsheet reads them: the one used constantly
            // before the two that are not.
            Self::FocusSearch,
            Self::CycleLayout,
            Self::Rescan,
            Self::ToggleQueue,
            Self::ToggleColumns,
            Self::ToggleSettings,
            Self::ShowProperties,
            Self::ToggleHelp,
            Self::Dismiss,
        ]
    }

    /// Which part of the cheatsheet it belongs under.
    ///
    /// A method rather than a list in the overlay, so that adding an action
    /// fails to compile until it has been given a home. A list would simply
    /// leave the new one out, silently.
    #[must_use]
    pub const fn group(self) -> Group {
        match self {
            Self::MoveUp
            | Self::MoveDown
            | Self::PageUp
            | Self::PageDown
            | Self::MoveToStart
            | Self::MoveToEnd
            | Self::ExtendUp
            | Self::ExtendDown => Group::Browsing,

            Self::PlaySelection
            | Self::PlayPause
            | Self::Next
            | Self::Previous
            | Self::ToggleShuffle
            | Self::CycleRepeat
            | Self::JumpToPlaying => Group::Playing,

            Self::FocusSearch | Self::CycleLayout | Self::Rescan => Group::Library,

            Self::ToggleQueue
            | Self::ToggleColumns
            | Self::ToggleSettings
            | Self::ShowProperties
            | Self::ToggleHelp
            | Self::Dismiss => Group::Panels,
        }
    }

    /// How the `?` cheatsheet describes it.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::MoveUp => "Move up",
            Self::MoveDown => "Move down",
            Self::PageUp => "Page up",
            Self::PageDown => "Page down",
            Self::MoveToStart => "Go to the first track",
            Self::MoveToEnd => "Go to the last track",
            Self::ExtendUp => "Extend the selection up",
            Self::ExtendDown => "Extend the selection down",
            Self::PlaySelection => "Play the selected track",
            Self::PlayPause => "Play or pause",
            Self::Next => "Next track",
            Self::Previous => "Previous track",
            Self::ToggleShuffle => "Shuffle",
            Self::CycleRepeat => "Repeat: off, all, one",
            Self::JumpToPlaying => "Jump to what is playing",
            Self::Rescan => "Rescan the library",
            Self::CycleLayout => "Cycle the browser placement",
            Self::FocusSearch => "Search",
            Self::ToggleQueue => "Show the queue",
            Self::ToggleColumns => "Columns and sorting",
            Self::ToggleSettings => "Settings",
            Self::ShowProperties => "Show track metadata",
            Self::ToggleHelp => "Show this list",
            Self::Dismiss => "Close, or clear the filters",
        }
    }
}

/// The headings the cheatsheet is divided into, in the order it shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Browsing,
    Playing,
    Library,
    Panels,
}

impl Group {
    /// In reading order: what is in front of you, then what it does, then the
    /// library as a whole, then the things that open over it.
    pub const ALL: &'static [Self] = &[Self::Browsing, Self::Playing, Self::Library, Self::Panels];

    #[must_use]
    pub const fn heading(self) -> &'static str {
        match self {
            Self::Browsing => "MOVING AROUND",
            Self::Playing => "PLAYING",
            Self::Library => "THE LIBRARY",
            Self::Panels => "PANELS",
        }
    }
}
