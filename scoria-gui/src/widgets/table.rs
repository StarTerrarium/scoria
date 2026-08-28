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
use iced::widget::{Space, column, container, mouse_area, row, scrollable, text};
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
    Header(String),
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
        lines.push(Line::Header(group.label.clone()));
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

/// What the table needs in order to draw itself.
pub struct TableView<'a> {
    pub lines: &'a [Line],
    pub rows: &'a [&'a Track],
    pub selected: Option<usize>,
    pub playing: Option<TrackId>,
    pub scroll_offset: f32,
    pub viewport_height: f32,
    pub theme: &'a Theme,
}

/// Messages the table produces. Translated by the application into commands;
/// the table itself knows nothing about playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A row was clicked: select it.
    Select(usize),
    /// A row was double-clicked, or a group header clicked: play from here.
    ///
    /// §7.3: group headers are click targets for playing the whole album.
    Activate(usize),
    /// Right-click, which §9.1 makes the one escape hatch from artwork
    /// following playback.
    Context(usize),
}

/// Builds the table.
#[must_use]
pub fn view<'a, Message: Clone + 'a>(
    table: &TableView<'_>,
    on_event: impl Fn(Event) -> Message + 'a,
) -> Element<'a, Message, Theme> {
    let (start, end) = visible_range(
        table.lines.len(),
        table.scroll_offset,
        table.viewport_height,
    );

    let mut body = column![].width(Fill);

    // The scroll extent above the first built row, so the scrollbar reflects
    // the whole library rather than the handful of rows that exist right now.
    if start > 0 {
        body = body.push(Space::new().height(Length::Fixed(start as f32 * ROW_HEIGHT)));
    }

    for index in start..end {
        let Some(line) = table.lines.get(index) else {
            break;
        };
        body = body.push(match line {
            Line::Header(label) => group_header(label, index, table.theme, &on_event),
            Line::Track(row_index) => match table.rows.get(*row_index) {
                Some(track) => track_row(track, *row_index, table, &on_event),
                None => Space::new().height(Length::Fixed(ROW_HEIGHT)).into(),
            },
        });
    }

    if end < table.lines.len() {
        let remaining = table.lines.len() - end;
        body = body.push(Space::new().height(Length::Fixed(remaining as f32 * ROW_HEIGHT)));
    }

    column![
        header(table.theme),
        scrollable(body).height(Fill).width(Fill)
    ]
    .width(Fill)
    .height(Fill)
    .into()
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
    .width(Fill)
    .into()
}

fn group_header<'a, Message: Clone + 'a>(
    label: &str,
    line: usize,
    theme: &Theme,
    on_event: &impl Fn(Event) -> Message,
) -> Element<'a, Message, Theme> {
    let style = theme.group_header_style();
    let content = container(text(label.to_owned()).size(13))
        .padding([0, 8])
        .height(Length::Fixed(ROW_HEIGHT))
        .align_y(Alignment::Center)
        .width(Fill)
        .style(move |_theme: &Theme| style);

    // §7.3: group headers are click targets for playing the whole album.
    mouse_area(content)
        .on_press(on_event(Event::Activate(line)))
        .into()
}

fn track_row<'a, Message: Clone + 'a>(
    track: &Track,
    row_index: usize,
    table: &TableView<'_>,
    on_event: &impl Fn(Event) -> Message,
) -> Element<'a, Message, Theme> {
    let state = RowState {
        selected: table.selected == Some(row_index),
        playing: table.playing == Some(track.id),
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
                Line::Header("Ghost".to_owned()),
                Line::Track(0),
                Line::Track(1),
                Line::Header("Alcest".to_owned()),
                Line::Track(2),
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
                Line::Header(_) => None,
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
    fn a_negative_offset_is_tolerated() {
        // Some scroll surfaces report a small negative offset while rubber
        // banding. Nothing here may panic on it.
        let (start, end) = visible_range(100, -50.0, 800.0);
        assert_eq!(start, 0);
        assert!(end <= 100);
    }
}
