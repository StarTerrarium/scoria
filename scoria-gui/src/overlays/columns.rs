//! Column and sort configuration.
//!
//! `DESIGN.md` §7.3: show/hide, reorder and resize from the header, and the
//! sort presets. Resizing happens on the header itself, by dragging; the rest
//! lives here.
//!
//! ## Why a drawer rather than a context menu
//!
//! §7.3 says "from a right-click on the header", and a right-click opens this.
//! But it is one of §9's drawers rather than a floating menu, because §9 is
//! explicit that there is *one* family of overlays with one set of rules --
//! opened by a key or a click, closed by Escape or the same action, never
//! stacked. A popup would be a second presentation with its own dismissal
//! behaviour, which is the configuration burden §9 exists to avoid.
//!
//! It is also more discoverable. A titled panel listing every column, with the
//! hidden ones visible and restorable, is easier to find your way around than a
//! menu that only appears if you already knew to right-click.

use crate::theme::{RowState, Theme};
use crate::widgets::columns::{Column, ColumnLayout};
use iced::widget::{column, container, mouse_area, row, scrollable, text};
use iced::{Alignment, Element, Fill, Length};
use scoria_core::query::sort::SortPreset;

pub struct ColumnsView<'a> {
    pub layout: &'a ColumnLayout,
    pub sort: SortPreset,
    /// Set when a header click is overriding the preset (§7.3), so the panel
    /// can say so rather than appearing to disagree with the table.
    pub overridden: bool,
    pub theme: &'a Theme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    ChooseSort(SortPreset),
    /// Drop the transient header-click override (§7.3's "one action to drop
    /// back").
    ClearOverride,
    ToggleColumn(Column),
    /// Move a column one place; negative is towards the front.
    MoveColumn(Column, i8),
}

#[must_use]
pub fn view<'a, Message: Clone + 'a>(
    columns: ColumnsView<'a>,
    on_event: impl Fn(Event) -> Message + Copy + 'a,
) -> Element<'a, Message, Theme> {
    let theme = columns.theme;
    let muted = theme.text_style(theme.muted_colour());
    let accent = theme.text_style(theme.accent_colour());

    let mut body = column![].spacing(2).width(Fill);

    body = body.push(heading("SORT", theme));
    for preset in SortPreset::all() {
        let chosen = preset == columns.sort;
        body = body.push(choice(
            preset.label().to_owned(),
            chosen,
            theme,
            on_event(Event::ChooseSort(preset)),
        ));
    }

    // §7.3: the override is temporary and always one action from gone. Saying
    // so here is what stops the panel looking as though it disagrees with the
    // table.
    if columns.overridden {
        body = body.push(
            mouse_area(
                container(
                    text("A column heading is overriding this — click to drop it")
                        .size(12)
                        .style(move |_t: &Theme| accent),
                )
                .padding([4, 8])
                .width(Fill),
            )
            .on_press(on_event(Event::ClearOverride)),
        );
    }

    body = body.push(gap());
    body = body.push(heading("COLUMNS", theme));

    let all = columns.layout.all();
    let last = all.len().saturating_sub(1);
    for (index, setting) in all.iter().enumerate() {
        let name = setting.column.name().to_owned();
        let move_up = (index > 0).then(|| on_event(Event::MoveColumn(setting.column, -1)));
        let move_down = (index < last).then(|| on_event(Event::MoveColumn(setting.column, 1)));

        body = body.push(column_row(
            name,
            setting.visible,
            theme,
            on_event(Event::ToggleColumn(setting.column)),
            move_up,
            move_down,
        ));
    }

    body = body.push(
        container(
            text("Drag the dividers between headings to resize. Click a heading to sort by it.")
                .size(11)
                .style(move |_t: &Theme| muted),
        )
        .padding([8, 8]),
    );

    scrollable(body)
        .spacing(crate::widgets::SCROLLBAR_SPACING)
        .width(Fill)
        .height(Fill)
        .into()
}

fn gap<'a, Message: 'a>() -> Element<'a, Message, Theme> {
    iced::widget::Space::new()
        .height(Length::Fixed(10.0))
        .into()
}

fn heading<'a, Message: 'a>(label: &'static str, theme: &Theme) -> Element<'a, Message, Theme> {
    let muted = theme.text_style(theme.muted_colour());
    container(text(label).size(11).style(move |_t: &Theme| muted))
        .padding([4, 8])
        .width(Fill)
        .into()
}

/// A pickable row with a tick when it is the one in force.
fn choice<'a, Message: Clone + 'a>(
    label: String,
    chosen: bool,
    theme: &Theme,
    message: Message,
) -> Element<'a, Message, Theme> {
    let state = RowState {
        selected: chosen,
        playing: false,
        alternate: false,
        available: true,
    };
    let style = theme.row_style(state);
    let colour = theme.text_style(theme.row_text(state));

    let content = container(
        row![
            text(if chosen { "✓" } else { " " })
                .size(13)
                .style(move |_t: &Theme| colour),
            text(label).size(13).style(move |_t: &Theme| colour),
        ]
        .spacing(8)
        .padding([0, 8])
        .align_y(Alignment::Center)
        .height(Length::Fixed(24.0)),
    )
    .width(Fill)
    .style(move |_t: &Theme| style);

    mouse_area(content).on_press(message).into()
}

/// A column, with its visibility and its place in the order.
fn column_row<'a, Message: Clone + 'a>(
    label: String,
    visible: bool,
    theme: &Theme,
    toggle: Message,
    move_up: Option<Message>,
    move_down: Option<Message>,
) -> Element<'a, Message, Theme> {
    let colour = theme.text_style(if visible {
        theme.text_colour()
    } else {
        theme.muted_colour()
    });
    let arrow = theme.text_style(theme.muted_colour());

    let mut controls = row![].spacing(2).align_y(Alignment::Center);
    for (glyph, message) in [("↑", move_up), ("↓", move_down)] {
        // A disabled arrow is left in place rather than removed, so the row does
        // not change width at the ends of the list.
        let button = container(text(glyph).size(12).style(move |_t: &Theme| arrow))
            .width(Length::Fixed(18.0))
            .align_x(Alignment::Center);
        controls = controls.push(match message {
            Some(message) => Element::from(mouse_area(button).on_press(message)),
            None => Element::from(button),
        });
    }

    let toggleable = mouse_area(
        row![
            text(if visible { "✓" } else { " " })
                .size(13)
                .style(move |_t: &Theme| colour),
            text(label)
                .size(13)
                .style(move |_t: &Theme| colour)
                .width(Fill),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .on_press(toggle);

    container(
        row![toggleable, controls]
            .spacing(8)
            .padding([0, 8])
            .align_y(Alignment::Center)
            .height(Length::Fixed(24.0)),
    )
    .width(Fill)
    .into()
}
