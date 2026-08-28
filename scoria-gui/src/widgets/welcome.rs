//! What the explorer shows when there is no library to show.
//!
//! `DESIGN.md` §5.1. First run is a "point me at your library" moment, and this
//! is it.
//!
//! **Not an overlay, deliberately.** The drawers in §9 rise over an explorer
//! that is already there, and every one of them is dismissible because there is
//! something behind it to go back to. On first run there is nothing behind it,
//! so a dismissible prompt would leave an empty table and no stated reason for
//! it. This takes the table's place instead: the window keeps its shape, the
//! transport bar and the header buttons stay where they are, and the only thing
//! that differs is that the space which would list tracks explains why it
//! cannot.
//!
//! It is not a layout either (invariant 3). It defines no table and no pane,
//! and `layouts::arrange` is unchanged -- it is handed a different element.

use crate::theme::Theme;
use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Element, Fill, Length};
use std::path::{Path, PathBuf};

/// A scan in flight, as the last `ScanProgress` reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scan {
    pub seen: usize,
    pub total: Option<usize>,
}

/// Why the track list is empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// No root has ever been chosen. The reason this screen exists.
    NoRoot,
    /// A root is set and the first scan has not produced anything yet.
    ///
    /// Distinct from `Empty` because the first scan of a real library takes
    /// long enough that, without this, waiting looks exactly like failure.
    Scanning(Scan),
    /// The scan finished and found nothing. Usually the wrong folder.
    Empty { root: PathBuf },
    /// The library itself could not be opened, so choosing a folder would not
    /// help and is not offered.
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    ChooseRoot,
    Rescan,
}

/// Whether the explorer should show this instead of the table, and which of it.
///
/// A pure function of what the frontend already knows, so the decision is
/// testable without a window. Order matters: anything indexed wins, because a
/// library that can be browsed should be browsed even while a scan is still
/// running behind it.
///
/// `problem` is why there is no session at all, when there is no session at
/// all. It outranks everything else, because none of the other answers are
/// knowable without one.
#[must_use]
pub fn state(
    problem: Option<&str>,
    root: Option<&Path>,
    tracks: usize,
    scanning: Option<Scan>,
) -> Option<State> {
    if let Some(reason) = problem {
        return Some(State::Unavailable {
            reason: reason.to_owned(),
        });
    }
    let Some(root) = root else {
        return Some(State::NoRoot);
    };
    if tracks > 0 {
        return None;
    }
    match scanning {
        Some(scan) => Some(State::Scanning(scan)),
        None => Some(State::Empty {
            root: root.to_path_buf(),
        }),
    }
}

/// How much of the window's width the panel occupies.
///
/// A share rather than a width in pixels: a column sized for a 1080p window is
/// a stripe on a 4K one, and `CLAUDE.md` invariant 4 says sizes are proportions
/// (this is the same reasoning as the drawer heights).
const PANEL_PORTION: u16 = 3;
const SIDE_PORTION: u16 = 1;

// Wide enough that a folder path is readable without wrapping three times,
// narrow enough that the prose does not run the full width of a large monitor,
// which is where long lines become hard to track back from.
const _: () = assert!(
    PANEL_PORTION > SIDE_PORTION,
    "the panel is narrower than its margins"
);
const _: () = assert!(
    PANEL_PORTION < SIDE_PORTION * 6,
    "the prose runs the full width"
);

/// Resolution of the scan progress bar, in portions.
const BAR_STEPS: u16 = 1000;

#[must_use]
pub fn view<'a, Message: Clone + 'a>(
    state: &State,
    theme: &Theme,
    on_event: impl Fn(Event) -> Message + Clone + 'a,
) -> Element<'a, Message, Theme> {
    let muted = theme.text_style(theme.muted_colour());

    let (title, body, actions): (String, String, Vec<(&str, Event)>) = match state {
        State::NoRoot => (
            "Scoria has not been pointed at your music yet".to_owned(),
            "Choose the folder your library lives in. Everything underneath it is scanned, \
             and Scoria keeps watching it, so music added later appears on its own."
                .to_owned(),
            vec![("Choose folder…", Event::ChooseRoot)],
        ),
        State::Scanning(scan) => (
            "Reading your library".to_owned(),
            match scan.total {
                Some(total) => format!("{} of {total} files.", scan.seen),
                None => format!("{} files so far.", scan.seen),
            },
            Vec::new(),
        ),
        State::Empty { root } => (
            "No music found".to_owned(),
            format!(
                "Nothing playable is in {}. If the music is somewhere else, point Scoria at \
                 it; if it was just added, scan again.",
                root.display()
            ),
            vec![
                ("Choose folder…", Event::ChooseRoot),
                ("Scan again", Event::Rescan),
            ],
        ),
        State::Unavailable { reason } => (
            "The library could not be opened".to_owned(),
            format!(
                "{reason}. Scoria cannot read or write its database, so there is nothing to show."
            ),
            Vec::new(),
        ),
    };

    let mut panel = column![
        text(title).size(20),
        text(body).size(13).style(move |_t: &Theme| muted),
    ]
    .spacing(10)
    .width(Fill);

    if let State::Scanning(scan) = state {
        panel = panel.push(progress(*scan, theme));
    }

    if !actions.is_empty() {
        let mut buttons = row![].spacing(8);
        for (label, event) in actions {
            buttons = buttons.push(
                button(text(label).size(13))
                    .padding([6, 16])
                    .on_press(on_event(event))
                    .style(|t: &Theme, status| t.chip_button(status)),
            );
        }
        panel = panel.push(container(buttons).padding([6, 0]));
    }

    // Centred by portions rather than by a fixed width, so it is the same
    // shape of thing on every monitor.
    container(
        row![
            iced::widget::Space::new().width(Length::FillPortion(SIDE_PORTION)),
            container(panel).width(Length::FillPortion(PANEL_PORTION)),
            iced::widget::Space::new().width(Length::FillPortion(SIDE_PORTION)),
        ]
        .align_y(Alignment::Center),
    )
    .center_y(Fill)
    .width(Fill)
    .height(Fill)
    .into()
}

/// A bar, for the one state where something is happening and the numbers alone
/// do not show how far through it is.
fn progress<'a, Message: 'a>(scan: Scan, theme: &Theme) -> Element<'a, Message, Theme> {
    let done = fraction(scan);
    let track = theme.surface_style();
    let filled = theme.group_header_style();

    container(
        row![
            container(iced::widget::Space::new().height(Length::Fixed(4.0)))
                .width(Length::FillPortion(done))
                .style(move |_t: &Theme| filled),
            container(iced::widget::Space::new().height(Length::Fixed(4.0)))
                .width(Length::FillPortion(BAR_STEPS.saturating_sub(done)))
                .style(move |_t: &Theme| track),
        ]
        .height(Length::Fixed(4.0)),
    )
    .width(Fill)
    .into()
}

/// How far through the scan is, in [`BAR_STEPS`].
///
/// A total the scanner has not worked out yet leaves the bar empty rather than
/// guessing at one, and a `seen` past the total -- files appearing while the
/// walk runs -- clamps instead of overflowing the row.
fn fraction(scan: Scan) -> u16 {
    let Some(total) = scan.total else {
        return 0;
    };
    let Some(share) = scan
        .seen
        .saturating_mul(usize::from(BAR_STEPS))
        .checked_div(total)
    else {
        return 0;
    };
    u16::try_from(share).unwrap_or(BAR_STEPS).min(BAR_STEPS)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const ROOT: Option<&Path> = None;

    fn root() -> PathBuf {
        PathBuf::from("/home/someone/Music")
    }

    #[test]
    fn a_library_with_tracks_shows_the_table() {
        assert_eq!(state(None, Some(&root()), 338, None), None);
    }

    #[test]
    fn a_scan_running_behind_a_populated_library_does_not_take_the_table_away() {
        // §5.2 scans in the background precisely so browsing is not
        // interrupted. Covering the list someone is using would undo that.
        let scan = Scan {
            seen: 12,
            total: Some(338),
        };
        assert_eq!(state(None, Some(&root()), 338, Some(scan)), None);
    }

    #[test]
    fn no_root_asks_for_one() {
        assert_eq!(state(None, ROOT, 0, None), Some(State::NoRoot));
    }

    #[test]
    fn a_first_scan_reports_progress_rather_than_emptiness() {
        // Without this the half-minute after choosing a folder is
        // indistinguishable from having chosen the wrong one.
        let scan = Scan {
            seen: 12,
            total: Some(338),
        };
        assert_eq!(
            state(None, Some(&root()), 0, Some(scan)),
            Some(State::Scanning(scan))
        );
    }

    #[test]
    fn a_finished_scan_that_found_nothing_says_where_it_looked() {
        assert_eq!(
            state(None, Some(&root()), 0, None),
            Some(State::Empty { root: root() })
        );
    }

    #[test]
    fn a_broken_library_is_not_offered_a_folder_chooser() {
        // Choosing a folder cannot fix a database that will not open, and
        // offering it invites someone to try until they believe their music is
        // at fault.
        let state = state(Some("the disk is full"), Some(&root()), 0, None).unwrap();
        assert_eq!(
            state,
            State::Unavailable {
                reason: "the disk is full".to_owned()
            }
        );
    }

    #[test]
    fn the_bar_stays_inside_its_row() {
        for scan in [
            Scan {
                seen: 0,
                total: Some(338),
            },
            Scan {
                seen: 338,
                total: Some(338),
            },
            // A file appearing during the walk, and a total that is not known
            // yet -- neither may overflow the row or divide by zero.
            Scan {
                seen: 400,
                total: Some(338),
            },
            Scan {
                seen: 12,
                total: Some(0),
            },
            Scan {
                seen: 12,
                total: None,
            },
            Scan {
                seen: usize::MAX,
                total: Some(1),
            },
        ] {
            assert!(fraction(scan) <= BAR_STEPS, "{scan:?} overflowed the bar");
        }
    }

    #[test]
    fn the_bar_fills_as_the_scan_runs() {
        let quarter = fraction(Scan {
            seen: 25,
            total: Some(100),
        });
        let half = fraction(Scan {
            seen: 50,
            total: Some(100),
        });
        assert!(quarter < half, "{quarter} is not behind {half}");
        assert_eq!(
            fraction(Scan {
                seen: 100,
                total: Some(100)
            }),
            BAR_STEPS
        );
    }
}
