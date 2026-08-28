//! The properties dialog.
//!
//! `DESIGN.md` §9.3: a full metadata dump for a track, scrolling, modal. A
//! complete dump has no editorial layer, which is why nothing here decides what
//! is worth showing -- it shows what is in the file, under the format's own key
//! names.
//!
//! Read-only in 1.0. Tag editing lands here post-1.0, and this is where it
//! belongs: an editor needs a save/cancel boundary, which a docked pane that
//! mutates on selection change has nowhere to put. Keeping tag writing behind a
//! deliberate modal action also protects a carefully tagged library from
//! accidental edits.

use crate::theme::Theme;
use crate::widgets::SCROLLBAR_SPACING;
use iced::widget::{column, container, row, scrollable, text};
use iced::{Element, Fill, Length};

pub struct PropertiesView<'a> {
    /// What the application knows about the track, in its own words.
    pub summary: Vec<(&'static str, String)>,
    /// Every tag in the file, under the format's own names.
    pub tags: Vec<(String, String)>,
    /// Set when the file could not be read.
    pub problem: Option<String>,
    pub theme: &'a Theme,
}

#[must_use]
pub fn view<'a, Message: 'a>(properties: PropertiesView<'a>) -> Element<'a, Message, Theme> {
    let theme = properties.theme;
    let muted = theme.text_style(theme.muted_colour());
    let mut body = column![].spacing(2).width(Fill);

    if let Some(problem) = properties.problem {
        body = body.push(entry("Problem", &problem, theme));
        body = body.push(gap());
    }

    for (key, value) in &properties.summary {
        body = body.push(entry(key, value, theme));
    }

    if !properties.tags.is_empty() {
        body = body.push(gap());
        body = body
            .push(container(text("TAGS").size(11).style(move |_t: &Theme| muted)).padding([4, 0]));
        for (key, value) in &properties.tags {
            body = body.push(entry(key, value, theme));
        }
    }

    scrollable(body)
        .spacing(SCROLLBAR_SPACING)
        .width(Fill)
        .height(Fill)
        .into()
}

fn gap<'a, Message: 'a>() -> Element<'a, Message, Theme> {
    iced::widget::Space::new().height(Length::Fixed(8.0)).into()
}

/// One key and its value.
///
/// The key column is fixed so the values line up: a dump nobody can scan is
/// not much better than no dump.
fn entry<'a, Message: 'a>(key: &str, value: &str, theme: &Theme) -> Element<'a, Message, Theme> {
    let muted = theme.text_style(theme.muted_colour());
    row![
        container(text(key.to_owned()).size(12).style(move |_t: &Theme| muted))
            .width(Length::Fixed(200.0))
            .clip(true),
        text(value.to_owned()).size(12).width(Fill),
    ]
    .spacing(10)
    .into()
}
