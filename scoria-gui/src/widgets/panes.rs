//! Filter panes.
//!
//! `DESIGN.md` §7.2: album artist, then album. `All (n)` is pinned at the top
//! of each, and counts are contextual -- computed against the filters applied
//! *before* this pane, never its own. Selecting an artist leaves every artist
//! listed and narrows only the album pane below it; a pane that collapsed to
//! its own selection would be a dead end.
//!
//! The counting is the query engine's job and already done by the time anything
//! here runs. This module draws the result.
//!
//! **There is no genre pane and there will not be one.** §7.2 explains why: a
//! pane implies its rows partition the library, and genre is many-valued per
//! track, so either the counts stop adding up to `All` or we invent a
//! primary-genre rule that is not in the tags.

use crate::theme::{RowState, Theme};
use crate::widgets::table::{ROW_HEIGHT, visible_range};
use iced::widget::{Space, column, container, mouse_area, responsive, row, scrollable, text};
use iced::{Alignment, Element, Fill, Length};

/// One row of a pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneRow {
    /// The display value. §4.1: the artist pane shows `The Omnific` even though
    /// it ordered under `O`.
    pub label: String,
    pub count: usize,
    pub selected: bool,
}

/// What a pane needs in order to draw itself.
pub struct PaneView<'a> {
    pub heading: &'a str,
    pub rows: Vec<PaneRow>,
    /// The pinned `All (n)` count.
    pub total: usize,
    pub scroll_offset: f32,
    pub theme: &'a Theme,
    pub id: &'static str,
}

impl PaneView<'_> {
    /// Nothing selected means everything is shown, which is what `All` means.
    fn all_selected(&self) -> bool {
        !self.rows.iter().any(|r| r.selected)
    }
}

/// What a pane can ask for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// Clear this pane's filter.
    SelectAll,
    Select(usize),
    Scrolled {
        offset: f32,
        viewport: f32,
    },
}

/// Builds a pane.
#[must_use]
pub fn view<'a, Message: Clone + 'a>(
    pane: PaneView<'a>,
    on_event: impl Fn(Event) -> Message + Copy + 'a,
) -> Element<'a, Message, Theme> {
    let theme = pane.theme;
    let heading_style = theme.header_style();
    let heading = container(text(pane.heading.to_owned()).size(11))
        .padding([0, 6])
        .height(Length::Fixed(ROW_HEIGHT))
        .align_y(Alignment::Center)
        .width(Fill)
        .style(move |_t: &Theme| heading_style);

    // §7.2: `All (n)` is pinned, not scrolled. It is the way back out of a
    // filter, so it must never be somewhere you have to look for it.
    let all = pane_row(
        "All".to_owned(),
        pane.total,
        pane.all_selected(),
        false,
        theme,
        on_event(Event::SelectAll),
    );

    // Virtualised for the same reason the table is: a large library has
    // thousands of album artists, and building a widget per row would cost the
    // same as building one per track. This reuses the table's arithmetic rather
    // than repeating it.
    let rows = pane.rows;
    let offset = pane.scroll_offset;
    let owned_theme = theme.clone();
    let id = pane.id;

    let body = responsive(move |size| {
        let (start, end) = visible_range(rows.len(), offset, size.height);
        let mut list = column![].width(Fill);

        if start > 0 {
            list = list.push(Space::new().height(Length::Fixed(start as f32 * ROW_HEIGHT)));
        }
        for index in start..end {
            let Some(row) = rows.get(index) else { break };
            list = list.push(pane_row(
                row.label.clone(),
                row.count,
                row.selected,
                index % 2 == 1,
                &owned_theme,
                on_event(Event::Select(index)),
            ));
        }
        if end < rows.len() {
            let remaining = rows.len() - end;
            list = list.push(Space::new().height(Length::Fixed(remaining as f32 * ROW_HEIGHT)));
        }

        scrollable(list)
            .id(iced::widget::Id::new(id))
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

    column![heading, all, body].width(Fill).height(Fill).into()
}

fn pane_row<'a, Message: Clone + 'a>(
    label: String,
    count: usize,
    selected: bool,
    alternate: bool,
    theme: &Theme,
    message: Message,
) -> Element<'a, Message, Theme> {
    let state = RowState {
        selected,
        playing: false,
        alternate,
        available: true,
    };
    let style = theme.row_style(state);
    let text_colour = theme.text_style(theme.row_text(state));
    let count_colour = theme.text_style(if selected {
        theme.row_text(state)
    } else {
        theme.muted_colour()
    });

    let content = container(
        row![
            container(text(label).size(13).style(move |_t: &Theme| text_colour))
                .width(Fill)
                .clip(true),
            text(count.to_string())
                .size(12)
                .style(move |_t: &Theme| count_colour),
        ]
        .spacing(6)
        .padding([0, 6])
        .align_y(Alignment::Center)
        .height(Length::Fixed(ROW_HEIGHT)),
    )
    .width(Fill)
    .style(move |_t: &Theme| style);

    mouse_area(content).on_press(message).into()
}
