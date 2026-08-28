//! Settings.
//!
//! `DESIGN.md` §9.4. A drawer like the others, under §9's rules.
//!
//! **What belongs here is what has nowhere else to live.** Columns and the sort
//! preset are configured from the header they affect, browser placement is a
//! keystroke, drawer heights are dragged. A setting edited where it applies
//! needs no screen, and moving those here would make them harder to find.
//!
//! That leaves the three with no natural place: the library root, the play
//! behaviour §6.3 calls the one user-facing option in the playback model, and
//! the theme.
//!
//! §2 says configuration tunes defaults and never constructs the application,
//! and the test is whether someone who never opens settings still has a fully
//! usable player. A settings screen is where that promise quietly dies if it
//! becomes the home for anything configurable, so adding to it is a reason to
//! re-read §2 rather than a routine act.

use crate::theme::{RowState, Theme};
use iced::widget::{column, container, mouse_area, row, scrollable, text};
use iced::{Alignment, Element, Fill, Length};
use scoria_core::config::PlayBehaviour;
use std::path::PathBuf;

pub struct SettingsView<'a> {
    pub root: Option<PathBuf>,
    pub play_behaviour: PlayBehaviour,
    pub theme_name: &'a str,
    /// Where things live on disk. Read-only, and genuinely hard to find
    /// otherwise.
    pub config_file: PathBuf,
    pub database: PathBuf,
    pub artwork_cache: PathBuf,
    /// How much of the library carries ReplayGain tags (§6.7).
    ///
    /// Here because a library that is mostly untagged sounds uneven, and
    /// nothing else in the application would ever say why.
    pub gain_tagged: usize,
    pub track_count: usize,
    pub theme: &'a Theme,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Open the folder picker (§5.1).
    ChooseRoot,
    SetPlayBehaviour(PlayBehaviour),
    ChooseTheme(String),
    Rescan,
}

#[must_use]
pub fn view<'a, Message: Clone + 'a>(
    settings: SettingsView<'a>,
    on_event: impl Fn(Event) -> Message + Clone + 'a,
) -> Element<'a, Message, Theme> {
    let theme = settings.theme;
    let muted = theme.text_style(theme.muted_colour());
    let mut body = column![].spacing(2).width(Fill);

    // ------------------------------------------------------------ library --

    body = body.push(heading("LIBRARY", theme));
    let root = settings
        .root
        .as_ref()
        .map_or_else(|| "Not set".to_owned(), |r| r.display().to_string());
    body = body.push(action_row(
        "Folder",
        root,
        "Choose…",
        theme,
        on_event(Event::ChooseRoot),
    ));
    body = body.push(action_row(
        "Contents",
        format!("{} tracks", settings.track_count),
        "Rescan",
        theme,
        on_event(Event::Rescan),
    ));

    // §6.7: the feature is only as good as the library's tag coverage, and a
    // partly tagged library sounds uneven for a reason nothing else explains.
    if let Some(percent) = (settings.gain_tagged * 100).checked_div(settings.track_count) {
        let note = if settings.gain_tagged == settings.track_count {
            "Every track is tagged for loudness levelling".to_owned()
        } else {
            format!(
                "{}% of tracks carry ReplayGain tags. Scoria reads them and never computes \
                 them; `rsgain easy` fills the gaps.",
                percent
            )
        };
        body = body.push(note_row(note, theme));
    }

    // ----------------------------------------------------------- playback --

    body = body.push(gap());
    body = body.push(heading("WHEN A TRACK IS PLAYED", theme));
    for (behaviour, label, explanation) in [
        (
            PlayBehaviour::WhatsShown,
            "Play what's shown",
            "The list you are looking at becomes the queue.",
        ),
        (
            PlayBehaviour::WholeLibrary,
            "Play the whole library",
            "Filters are ignored, but the sort you chose is kept.",
        ),
    ] {
        body = body.push(choice(
            label.to_owned(),
            Some(explanation.to_owned()),
            behaviour == settings.play_behaviour,
            theme,
            on_event(Event::SetPlayBehaviour(behaviour)),
        ));
    }

    // ----------------------------------------------------------- appearance --

    body = body.push(gap());
    body = body.push(heading("THEME", theme));
    for available in Theme::all() {
        let chosen = available.name == settings.theme_name;
        let name = available.name.clone();
        body = body.push(choice(
            name.clone(),
            None,
            chosen,
            theme,
            on_event(Event::ChooseTheme(name)),
        ));
    }

    // ---------------------------------------------------------------- paths --

    body = body.push(gap());
    body = body.push(heading("FILES", theme));
    for (label, path) in [
        ("Settings", settings.config_file),
        ("Database", settings.database),
        ("Artwork cache", settings.artwork_cache),
    ] {
        body = body.push(
            container(
                row![
                    container(text(label).size(12).style(move |_t: &Theme| muted))
                        .width(Length::Fixed(120.0)),
                    text(path.display().to_string()).size(12).width(Fill),
                ]
                .spacing(8)
                .padding([0, 8])
                .align_y(Alignment::Center)
                .height(Length::Fixed(22.0)),
            )
            .width(Fill),
        );
    }

    scrollable(body)
        .spacing(crate::widgets::SCROLLBAR_SPACING)
        .width(Fill)
        .height(Fill)
        .into()
}

fn gap<'a, Message: 'a>() -> Element<'a, Message, Theme> {
    iced::widget::Space::new()
        .height(Length::Fixed(12.0))
        .into()
}

fn heading<'a, Message: 'a>(label: &'static str, theme: &Theme) -> Element<'a, Message, Theme> {
    let muted = theme.text_style(theme.muted_colour());
    container(text(label).size(11).style(move |_t: &Theme| muted))
        .padding([4, 8])
        .width(Fill)
        .into()
}

fn note_row<'a, Message: 'a>(note: String, theme: &Theme) -> Element<'a, Message, Theme> {
    let muted = theme.text_style(theme.muted_colour());
    container(text(note).size(11).style(move |_t: &Theme| muted))
        .padding([2, 8])
        .width(Fill)
        .into()
}

/// A labelled value with a button beside it.
fn action_row<'a, Message: Clone + 'a>(
    label: &'static str,
    value: String,
    action: &'static str,
    theme: &Theme,
    message: Message,
) -> Element<'a, Message, Theme> {
    let muted = theme.text_style(theme.muted_colour());
    container(
        row![
            container(text(label).size(12).style(move |_t: &Theme| muted))
                .width(Length::Fixed(120.0)),
            container(text(value).size(13)).width(Fill).clip(true),
            iced::widget::button(text(action).size(12))
                .padding([2, 10])
                .on_press(message)
                .style(|t: &Theme, status| t.chip_button(status)),
        ]
        .spacing(8)
        .padding([0, 8])
        .align_y(Alignment::Center)
        .height(Length::Fixed(28.0)),
    )
    .width(Fill)
    .into()
}

/// A pickable row, ticked when it is the one in force.
fn choice<'a, Message: Clone + 'a>(
    label: String,
    explanation: Option<String>,
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
    let muted = theme.text_style(if chosen {
        theme.row_text(state)
    } else {
        theme.muted_colour()
    });

    let mut lines = column![
        row![
            text(if chosen { "✓" } else { " " })
                .size(13)
                .style(move |_t: &Theme| colour),
            text(label).size(13).style(move |_t: &Theme| colour),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(1);

    if let Some(explanation) = explanation {
        lines = lines.push(
            container(text(explanation).size(11).style(move |_t: &Theme| muted)).padding([0, 21]),
        );
    }

    let content = container(lines.padding([4, 8]))
        .width(Fill)
        .style(move |_t: &Theme| style);
    mouse_area(content).on_press(message).into()
}
