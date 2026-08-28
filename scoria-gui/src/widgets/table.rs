//! The track table.
//!
//! **This is THE track table.** `CLAUDE.md` invariant 3: it is used by every
//! layout and by the queue overlay, and a second implementation anywhere is a
//! bug. Layouts arrange it; they never define one.
//!
//! ## Virtualisation
//!
//! §14 says to design for a six-figure library regardless of actual size, and
//! it is the same work either way. Building a widget per track would mean a
//! hundred thousand widgets laid out on every frame, so only the rows inside
//! the viewport are built. The rest of the scroll extent is two spacers, which
//! is what keeps the scrollbar the right size and the position honest.
//!
//! The display list is a flat `Vec<Line>` rather than nested groups, computed
//! when the query result changes rather than per frame. Group headers occupy a
//! line like any other row, which is what makes the arithmetic a single
//! division instead of a search.

use crate::theme::{RowState, Theme};
use iced::widget::{Space, column, container, mouse_area, responsive, row, scrollable, text};
use iced::{Alignment, Element, Fill, Length};
use scoria_core::id::TrackId;
use scoria_core::model::Track;
use scoria_core::query::sort::Group;

/// Every row is the same height, which is what makes virtualisation arithmetic.
pub const ROW_HEIGHT: f32 = 22.0;
/// Rows built beyond the viewport, so a fast scroll does not show gaps.
const OVERSCAN: usize = 6;

/// One line of the display list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// A group header (§7.3), inserted where the primary sort key changes.
    ///
    /// Carries the row its group starts at as well as its label. Nothing reads
    /// that yet -- headers are inert (see [`group_header`]) -- but §7.1 makes
    /// type-to-jump the way `Off` placement is navigated, and jumping to a
    /// group means landing on the row it starts at. The flattening already
    /// knows the answer, so it says so rather than making the caller rederive
    /// it from line indices, which is what produced the bug this replaced.
    Header { label: String, first_row: usize },
    /// A track, by its index into the query result.
    Track(usize),
}

/// Flattens rows and their groups into a display list.
///
/// Done once per query rather than per frame: at six figures this allocates a
/// couple of megabytes, which is fine once and wasteful sixty times a second.
#[must_use]
pub fn build_lines(track_count: usize, groups: &[Group]) -> Vec<Line> {
    if groups.is_empty() {
        return (0..track_count).map(Line::Track).collect();
    }

    let mut lines = Vec::with_capacity(track_count + groups.len());
    for group in groups {
        if group.start >= track_count {
            continue;
        }
        lines.push(Line::Header {
            label: group.label.clone(),
            first_row: group.start,
        });
        for index in group.start..group.end().min(track_count) {
            lines.push(Line::Track(index));
        }
    }
    lines
}

/// Which lines are worth building for a given scroll position.
#[must_use]
pub fn visible_range(lines: usize, scroll_offset: f32, viewport_height: f32) -> (usize, usize) {
    if lines == 0 {
        return (0, 0);
    }
    let first = (scroll_offset / ROW_HEIGHT).floor().max(0.0) as usize;
    let visible = (viewport_height / ROW_HEIGHT).ceil() as usize;

    // Both ends are clamped to the line count. Clamping only `end` would let a
    // scroll offset past the end produce a start beyond it, and then a range
    // that runs backwards.
    let start = first.saturating_sub(OVERSCAN).min(lines);
    let end = first
        .saturating_add(visible)
        .saturating_add(OVERSCAN)
        .min(lines)
        .max(start);
    (start, end)
}

/// The scroll offset that puts `line` in the middle of the viewport.
///
/// Clamped to the real scroll range. That is not tidiness: the application
/// tracks the offset itself in order to decide which rows to build, and the
/// widget clamps whatever it is given. An unclamped offset would leave the two
/// disagreeing, and the table would build a window the viewer is not looking at.
#[must_use]
pub fn centre_offset(line: usize, lines: usize, viewport_height: f32) -> f32 {
    let content = lines as f32 * ROW_HEIGHT;
    let furthest = (content - viewport_height).max(0.0);

    let row_top = line as f32 * ROW_HEIGHT;
    let centred = row_top - (viewport_height - ROW_HEIGHT) / 2.0;
    centred.clamp(0.0, furthest)
}

/// What the table needs in order to draw itself.
pub struct TableView<'a> {
    pub lines: &'a [Line],
    /// Owned, so the whole view can move into the `responsive` closure -- which
    /// is what lets the table measure its own viewport.
    pub rows: Vec<&'a Track>,
    pub selected: Option<usize>,
    /// The playing track, by id. What the library view knows.
    pub playing: Option<TrackId>,
    /// The playing row, by index. What the queue knows.
    ///
    /// Both exist because they answer the same question from different sides: a
    /// library view has no position, and a snapshot can hold one track twice,
    /// where only one instance is playing.
    pub playing_row: Option<usize>,
    pub scroll_offset: f32,
    pub theme: &'a Theme,
    /// Identifies the scrollable, so moving the cursor by keyboard can move the
    /// view with it.
    pub id: &'static str,
}

/// Messages the table produces. Translated by the application into commands;
/// the table itself knows nothing about playback.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// A row was clicked: select it.
    Select(usize),
    /// A row was double-clicked: play from here.
    Activate(usize),
    /// Right-click, which §9.1 makes the one escape hatch from artwork
    /// following playback.
    Context(usize),
    /// The scroll position moved, and the viewport is this tall.
    ///
    /// Virtualisation cannot work without this. The window of built rows comes
    /// from where the scroll is, so a table that is never told about scrolling
    /// renders one screenful and blank space for the rest of the library.
    Scrolled { offset: f32, viewport: f32 },
}

/// Builds the table.
#[must_use]
pub fn view<'a, Message: Clone + 'a>(
    table: TableView<'a>,
    on_event: impl Fn(Event) -> Message + Copy + 'a,
) -> Element<'a, Message, Theme> {
    let header_row = header(table.theme);

    // `responsive` hands the closure the space actually available, which is the
    // only honest source for the viewport height. Deriving it from the window
    // size would mean tracking the height of every piece of chrome around the
    // table and getting it wrong whenever one changed.
    let body = responsive(move |size| {
        let (start, end) = visible_range(table.lines.len(), table.scroll_offset, size.height);
        let mut rows = column![].width(Fill);

        // The scroll extent above the first built row, so the scrollbar
        // reflects the whole library rather than the rows that exist now.
        if start > 0 {
            rows = rows.push(Space::new().height(Length::Fixed(start as f32 * ROW_HEIGHT)));
        }

        for index in start..end {
            let Some(line) = table.lines.get(index) else {
                break;
            };
            rows = rows.push(match line {
                Line::Header { label, .. } => group_header(label, table.theme),
                Line::Track(row_index) => match table.rows.get(*row_index) {
                    Some(track) => track_row(track, *row_index, &table, &on_event),
                    None => Space::new().height(Length::Fixed(ROW_HEIGHT)).into(),
                },
            });
        }

        if end < table.lines.len() {
            let remaining = table.lines.len() - end;
            rows = rows.push(Space::new().height(Length::Fixed(remaining as f32 * ROW_HEIGHT)));
        }

        scrollable(rows)
            .id(iced::widget::Id::new(table.id))
            // Embedded rather than floating, so the bar sits beside the last
            // column instead of on top of it.
            .spacing(crate::widgets::SCROLLBAR_SPACING)
            .height(Fill)
            .width(Fill)
            .on_scroll(move |viewport| {
                on_event(Event::Scrolled {
                    offset: viewport.absolute_offset().y,
                    viewport: viewport.bounds().height,
                })
            })
            .into()
    });

    column![header_row, body].width(Fill).height(Fill).into()
}

/// The column header. §7.3 gives the default column set.
fn header<'a, Message: 'a>(theme: &Theme) -> Element<'a, Message, Theme> {
    let style = theme.header_style();
    let cell = |label: &'static str, width: Length| {
        container(text(label).size(12)).width(width).padding([0, 6])
    };

    container(
        row![
            cell("#", Length::Fixed(34.0)),
            cell("TITLE", Length::FillPortion(4)),
            cell("ALBUM ARTIST", Length::FillPortion(3)),
            cell("ARTIST", Length::FillPortion(3)),
            cell("ALBUM", Length::FillPortion(3)),
            cell("TIME", Length::Fixed(60.0)),
            cell("YEAR", Length::Fixed(50.0)),
        ]
        .align_y(Alignment::Center)
        .height(Length::Fixed(ROW_HEIGHT)),
    )
    .style(move |_theme: &Theme| style)
    .padding(iced::Padding {
        top: 0.0,
        right: crate::widgets::SCROLLBAR_ALLOWANCE,
        bottom: 0.0,
        left: 0.0,
    })
    .width(Fill)
    .into()
}

/// A group header.
///
/// **Deliberately not a click target**, though §7.3 originally called for one.
/// Playing a group on a single click made the most destructive action in the
/// table -- replacing the playback snapshot -- the easiest one to trigger by
/// accident, while every other activation is a double-click. §6.1 is explicit
/// that browsing never disturbs playback, and a stray click on a header is
/// browsing.
///
/// It also bought nothing: the group's first track sits one row below, and
/// double-clicking it does the same thing. A header click would earn its place
/// if groups could collapse, since then it would be the only way to reach the
/// group at all -- but collapse is not in the design.
fn group_header<'a, Message: Clone + 'a>(
    label: &str,
    theme: &Theme,
) -> Element<'a, Message, Theme> {
    let style = theme.group_header_style();
    let content = container(text(label.to_owned()).size(13))
        .padding([0, 8])
        .height(Length::Fixed(ROW_HEIGHT))
        .align_y(Alignment::Center)
        .width(Fill)
        .style(move |_theme: &Theme| style);

    content.into()
}

fn track_row<'a, Message: Clone + 'a>(
    track: &Track,
    row_index: usize,
    table: &TableView<'_>,
    on_event: &impl Fn(Event) -> Message,
) -> Element<'a, Message, Theme> {
    let state = RowState {
        selected: table.selected == Some(row_index),
        playing: table.playing == Some(track.id) || table.playing_row == Some(row_index),
        alternate: row_index % 2 == 1,
        available: track.available,
    };

    let theme = table.theme;
    let text_colour = theme.text_style(theme.row_text(state));
    let cell = move |value: String, width: Length| {
        container(
            text(value)
                .size(13)
                .style(move |_theme: &Theme| text_colour),
        )
        .width(width)
        .padding([0, 6])
        .clip(true)
    };

    // §7.4: a missing file is marked rather than removed, so what will happen
    // is stated rather than left to be discovered.
    let title = if track.available {
        track.title.clone()
    } else {
        format!("{}  (missing)", track.title)
    };

    let style = theme.row_style(state);
    let content = container(
        row![
            cell(
                track.track_number.map_or_default_number(),
                Length::Fixed(34.0)
            ),
            cell(title, Length::FillPortion(4)),
            // §4.1: album artist is structural, artist is display. Both are
            // shown by default, deliberately.
            cell(track.album_artist.clone(), Length::FillPortion(3)),
            cell(track.artist.clone(), Length::FillPortion(3)),
            cell(track.album.clone(), Length::FillPortion(3)),
            cell(
                crate::widgets::format_duration(track.duration),
                Length::Fixed(60.0)
            ),
            cell(
                track
                    .date
                    .map_or_else(String::new, |d| d.year().to_string()),
                Length::Fixed(50.0)
            ),
        ]
        .align_y(Alignment::Center)
        .height(Length::Fixed(ROW_HEIGHT)),
    )
    .width(Fill)
    .style(move |_theme: &Theme| style);

    mouse_area(content)
        .on_press(on_event(Event::Select(row_index)))
        .on_double_click(on_event(Event::Activate(row_index)))
        .on_right_press(on_event(Event::Context(row_index)))
        .into()
}

/// Small helper so the track-number cell reads cleanly at the call site.
trait NumberCell {
    fn map_or_default_number(self) -> String;
}

impl NumberCell for Option<u16> {
    fn map_or_default_number(self) -> String {
        self.map_or_else(String::new, |n| n.to_string())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    fn group(label: &str, start: usize, len: usize) -> Group {
        Group {
            label: label.to_owned(),
            start,
            len,
        }
    }

    #[test]
    fn without_groups_every_line_is_a_track() {
        let lines = build_lines(3, &[]);
        assert_eq!(lines, vec![Line::Track(0), Line::Track(1), Line::Track(2)]);
    }

    #[test]
    fn groups_become_header_lines_among_their_tracks() {
        let lines = build_lines(3, &[group("Ghost", 0, 2), group("Alcest", 2, 1)]);
        assert_eq!(
            lines,
            vec![
                Line::Header {
                    label: "Ghost".to_owned(),
                    first_row: 0
                },
                Line::Track(0),
                Line::Track(1),
                Line::Header {
                    label: "Alcest".to_owned(),
                    first_row: 2
                },
                Line::Track(2),
            ]
        );
    }

    #[test]
    fn a_header_names_the_row_its_group_starts_at() {
        // Headers are inert now, but the mapping still has to be right: §7.1's
        // type-to-jump lands on the row a group starts at, and the bug this
        // guards against was exactly that arithmetic -- line and row indices
        // diverge by one for every header above.
        let lines = build_lines(
            25,
            &[group("a", 0, 13), group("b", 13, 10), group("c", 23, 2)],
        );

        let headers: Vec<(&str, usize)> = lines
            .iter()
            .filter_map(|l| match l {
                Line::Header { label, first_row } => Some((label.as_str(), *first_row)),
                Line::Track(_) => None,
            })
            .collect();

        assert_eq!(headers, vec![("a", 0), ("b", 13), ("c", 23)]);

        // And the row a header names must be the first track drawn under it,
        // which is the property a click depends on.
        for (index, line) in lines.iter().enumerate() {
            if let Line::Header { first_row, .. } = line {
                assert_eq!(
                    lines.get(index + 1),
                    Some(&Line::Track(*first_row)),
                    "the header does not sit directly above the row it names"
                );
            }
        }
    }

    #[test]
    fn a_group_reaching_past_the_rows_is_dropped_rather_than_left_empty() {
        // A stale group against a shorter result would otherwise leave a header
        // naming a row that is not there.
        let lines = build_lines(2, &[group("a", 0, 2), group("b", 5, 3)]);
        assert_eq!(
            lines,
            vec![
                Line::Header {
                    label: "a".to_owned(),
                    first_row: 0
                },
                Line::Track(0),
                Line::Track(1),
            ]
        );
    }

    #[test]
    fn every_track_appears_exactly_once() {
        let groups = [group("a", 0, 5), group("b", 5, 3), group("c", 8, 2)];
        let lines = build_lines(10, &groups);
        let mut tracks: Vec<usize> = lines
            .iter()
            .filter_map(|l| match l {
                Line::Track(i) => Some(*i),
                Line::Header { .. } => None,
            })
            .collect();
        tracks.sort_unstable();
        assert_eq!(tracks, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn an_empty_table_has_no_lines() {
        assert!(build_lines(0, &[]).is_empty());
        assert_eq!(visible_range(0, 0.0, 800.0), (0, 0));
    }

    #[test]
    fn only_the_viewport_is_built() {
        // The point of virtualising: a six-figure library must not become a
        // six-figure widget tree.
        let (start, end) = visible_range(100_000, 0.0, 800.0);
        assert_eq!(start, 0);
        assert!(end < 100, "built {end} rows for an 800px viewport");
    }

    #[test]
    fn scrolling_moves_the_window_rather_than_growing_it() {
        let (first_start, first_end) = visible_range(100_000, 0.0, 800.0);
        let (later_start, later_end) = visible_range(100_000, 50_000.0, 800.0);

        assert!(later_start > first_start, "the window should have moved");

        // Not equal sizes: at the very top there is nothing above to overscan
        // into, so that window is shorter by the overscan. What matters is that
        // scrolling does not make the window grow without bound.
        let viewport_rows = (800.0 / ROW_HEIGHT).ceil() as usize;
        let ceiling = viewport_rows + 2 * OVERSCAN + 1;
        assert!(first_end - first_start <= ceiling);
        assert!(later_end - later_start <= ceiling);
    }

    #[test]
    fn an_offset_past_the_end_yields_an_empty_range_rather_than_an_inverted_one() {
        // A range that runs backwards would either build nothing or panic on
        // the slice, depending on how it was used.
        let (start, end) = visible_range(10, 1_000_000.0, 800.0);
        assert!(start <= end);
        assert!(end <= 10);
    }

    #[test]
    fn the_window_never_runs_past_the_end() {
        let lines = 50;
        let (start, end) = visible_range(lines, 10_000.0, 800.0);
        assert!(end <= lines);
        assert!(
            start <= end,
            "an inverted range would build nothing or panic"
        );
    }

    #[test]
    fn overscan_covers_rows_just_outside_the_viewport() {
        // Without it, a fast scroll shows blank strips where rows have not been
        // built yet.
        let (start, _) = visible_range(1000, 10.0 * ROW_HEIGHT, 800.0);
        assert!(start < 10, "should start above the first visible row");
    }

    #[test]
    fn centring_puts_the_row_in_the_middle() {
        // §6.5 leans on this: landing on the playing track should put the
        // artist's other albums immediately above and below it. At the top edge
        // there is nothing above.
        let viewport = 800.0;
        let offset = centre_offset(500, 1000, viewport);

        let row_top = 500.0 * ROW_HEIGHT;
        let middle = offset + viewport / 2.0;
        assert!(
            (row_top + ROW_HEIGHT / 2.0 - middle).abs() < ROW_HEIGHT,
            "row centre {} is not near viewport centre {middle}",
            row_top + ROW_HEIGHT / 2.0
        );
    }

    #[test]
    fn centring_near_the_start_stops_at_the_top() {
        // There is nothing above row 2 to scroll to, so asking for it centred
        // must not produce a negative offset.
        assert!(centre_offset(2, 1000, 800.0) >= 0.0);
        assert_eq!(centre_offset(0, 1000, 800.0), 0.0);
    }

    #[test]
    fn centring_near_the_end_stops_at_the_bottom() {
        // Past the end the widget would clamp and we would not, and then the
        // built window and the visible one would be different rows.
        let viewport = 800.0;
        let lines = 100;
        let offset = centre_offset(99, lines, viewport);
        let furthest = (lines as f32 * ROW_HEIGHT - viewport).max(0.0);
        assert!(
            offset <= furthest,
            "{offset} overshoots the end at {furthest}"
        );
    }

    #[test]
    fn a_list_shorter_than_the_viewport_never_scrolls() {
        assert_eq!(centre_offset(3, 5, 800.0), 0.0);
    }

    #[test]
    fn a_negative_offset_is_tolerated() {
        // Some scroll surfaces report a small negative offset while rubber
        // banding. Nothing here may panic on it.
        let (start, end) = visible_range(100, -50.0, 800.0);
        assert_eq!(start, 0);
        assert!(end <= 100);
    }
}
