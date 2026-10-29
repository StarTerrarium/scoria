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
    pub track_count: usize,
    pub missing: Missing,
    /// How much of the library is levelled, and what can be done about the
    /// rest (§6.7).
    ///
    /// Here because a library that is partly untagged sounds uneven, and
    /// nothing else in the application would ever say why.
    pub loudness: Loudness,
    /// Files the last analysis could not measure or write, with why, already
    /// phrased for display.
    pub gain_failures: &'a [(String, String)],
    pub theme: &'a Theme,
}

/// How many tracks are levelled, and what an analysis would do about the rest.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoudnessCounts {
    pub levelled: usize,
    pub total: usize,
    /// Albums an analysis would work through, and how many files that is.
    pub albums: usize,
    pub files: usize,
    /// Albums with gaps that an analysis has to leave alone, by reason.
    pub skipped_missing: usize,
    pub skipped_unsupported: usize,
}

/// The Loudness row's state (§6.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loudness {
    /// An empty library has nothing to say about loudness.
    None,
    AllLevelled,
    /// Gaps an analysis would fill.
    Ready(LoudnessCounts),
    /// Gaps, but none an analysis can fill: every album with one is skipped.
    Stuck(LoudnessCounts),
    /// Asked once; waiting to be asked again, or told no.
    Armed(LoudnessCounts),
    Running {
        done: usize,
        total: usize,
        album: String,
    },
    /// A scan is running; analysing now would race it.
    Scanning(LoudnessCounts),
    Unreachable(LoudnessCounts),
}

/// What can be done about tracks whose files are gone (§7.4, "Forgetting").
///
/// Each variant carries the count, because every one of them says it: the
/// number is what someone is deciding about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    /// Nothing is missing, so there is no row at all.
    None,
    Ready(usize),
    /// Asked once; waiting to be asked again, or told no.
    Armed(usize),
    /// A scan is running, and forgetting now would race it.
    Scanning(usize),
    /// The library root is not on disk, so every track reads as missing and
    /// forgetting them would empty a library that is only unplugged.
    Unreachable(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Open the folder picker (§5.1).
    ChooseRoot,
    SetPlayBehaviour(PlayBehaviour),
    ChooseTheme(String),
    Rescan,
    /// Arm the Missing row. Nothing is removed yet.
    ForgetMissing,
    ConfirmForget,
    CancelForget,
    /// Arm the Loudness row. Nothing is written yet.
    AnalyseGain,
    ConfirmAnalyse,
    CancelAnalyse,
    StopAnalyse,
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

    // §7.4: present only while something is missing, so a library with
    // nothing to forget has a settings screen that never mentions it.
    let gone = |n: usize| {
        if n == 1 {
            "1 file is gone".to_owned()
        } else {
            format!("{n} files are gone")
        }
    };
    match settings.missing {
        Missing::None => {}
        Missing::Ready(n) => {
            body = body.push(action_row(
                "Missing",
                gone(n),
                "Forget…",
                theme,
                on_event(Event::ForgetMissing),
            ));
        }
        Missing::Armed(n) => {
            let tracks = if n == 1 {
                "1 track".to_owned()
            } else {
                format!("{n} tracks")
            };
            body = body.push(confirm_row(
                "Missing",
                format!("Forget {tracks}? Their play counts go too."),
                Confirm::Destructive("Forget"),
                theme,
                on_event(Event::ConfirmForget),
                on_event(Event::CancelForget),
            ));
        }
        Missing::Scanning(n) => {
            body = body.push(value_row(
                "Missing",
                format!("{} · available when the scan finishes", gone(n)),
                theme,
            ));
        }
        Missing::Unreachable(n) => {
            body = body.push(value_row(
                "Missing",
                format!("{} · the library folder isn't reachable", gone(n)),
                theme,
            ));
        }
    }

    // §6.7: the feature is only as good as the library's tag coverage, and a
    // partly tagged library sounds uneven for a reason nothing else explains.
    body = loudness_rows(
        body,
        &settings.loudness,
        settings.gain_failures,
        theme,
        &on_event,
    );

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

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// The Loudness row, and the notes beneath it.
fn loudness_rows<'a, Message: Clone + 'a>(
    mut body: iced::widget::Column<'a, Message, Theme>,
    loudness: &Loudness,
    failures: &'a [(String, String)],
    theme: &Theme,
    on_event: &(impl Fn(Event) -> Message + Clone + 'a),
) -> iced::widget::Column<'a, Message, Theme> {
    let levelled = |c: &LoudnessCounts| format!("{} of {} tracks levelled", c.levelled, c.total);
    let skipped_notes = |c: &LoudnessCounts| {
        let mut notes = Vec::new();
        if c.skipped_missing > 0 {
            notes.push(format!(
                "{} left alone: some of their files are missing.",
                plural(c.skipped_missing, "album", "albums")
            ));
        }
        if c.skipped_unsupported > 0 {
            notes.push(format!(
                "{} left alone: only FLAC and Ogg Vorbis can be tagged so far.",
                plural(c.skipped_unsupported, "album", "albums")
            ));
        }
        notes
    };

    match loudness {
        Loudness::None => {}
        Loudness::AllLevelled => {
            body = body.push(value_row(
                "Loudness",
                "Every track is levelled".to_owned(),
                theme,
            ));
        }
        Loudness::Ready(c) => {
            body = body.push(action_row(
                "Loudness",
                levelled(c),
                "Analyse…",
                theme,
                on_event(Event::AnalyseGain),
            ));
            body = body.push(note_row(
                "Reads each album that lacks loudness tags and writes ReplayGain tags into its \
                 files, as `rsgain easy` would."
                    .to_owned(),
                theme,
            ));
            for note in skipped_notes(c) {
                body = body.push(note_row(note, theme));
            }
        }
        Loudness::Stuck(c) => {
            body = body.push(value_row("Loudness", levelled(c), theme));
            for note in skipped_notes(c) {
                body = body.push(note_row(note, theme));
            }
        }
        Loudness::Armed(c) => {
            body = body.push(confirm_row(
                "Loudness",
                format!(
                    "Write loudness tags into {} ({})?",
                    plural(c.files, "file", "files"),
                    plural(c.albums, "album", "albums")
                ),
                Confirm::Additive("Analyse"),
                theme,
                on_event(Event::ConfirmAnalyse),
                on_event(Event::CancelAnalyse),
            ));
        }
        Loudness::Running { done, total, album } => {
            let progress = if album.is_empty() {
                format!("Analysing {total} albums")
            } else {
                format!("Analysing {done} of {total} albums · {album}")
            };
            body = body.push(action_row(
                "Loudness",
                progress,
                "Stop",
                theme,
                on_event(Event::StopAnalyse),
            ));
        }
        Loudness::Scanning(c) => {
            body = body.push(value_row(
                "Loudness",
                format!("{} · available when the scan finishes", levelled(c)),
                theme,
            ));
        }
        Loudness::Unreachable(c) => {
            body = body.push(value_row(
                "Loudness",
                format!("{} · the library folder isn't reachable", levelled(c)),
                theme,
            ));
        }
    }

    // What the last analysis could not do, until the next one. A handful,
    // because a list of hundreds says nothing a count does not.
    const SHOWN: usize = 5;
    for (path, reason) in failures.iter().take(SHOWN) {
        body = body.push(note_row(format!("Couldn't write {path}: {reason}"), theme));
    }
    if let Some(more) = failures.len().checked_sub(SHOWN).filter(|n| *n > 0) {
        body = body.push(note_row(format!("…and {more} more"), theme));
    }
    body
}

/// What the confirming button of a [`confirm_row`] does, which decides how it
/// looks: removing something reads differently from adding to it.
#[derive(Debug, Clone, Copy)]
enum Confirm {
    Destructive(&'static str),
    Additive(&'static str),
}

/// A labelled value and nothing to press: the Missing row when pressing would
/// be wrong, which it says in words rather than by greying a button out.
fn value_row<'a, Message: Clone + 'a>(
    label: &'static str,
    value: String,
    theme: &Theme,
) -> Element<'a, Message, Theme> {
    let muted = theme.text_style(theme.muted_colour());
    container(
        row![
            container(text(label).size(12).style(move |_t: &Theme| muted))
                .width(Length::Fixed(120.0)),
            container(text(value).size(13)).width(Fill).clip(true),
        ]
        .spacing(8)
        .padding([0, 8])
        .align_y(Alignment::Center)
        .height(Length::Fixed(28.0)),
    )
    .width(Fill)
    .into()
}

/// An [`action_row`] that has been pressed once and wants to be sure.
///
/// The same height in the same place, so nothing moves under the pointer
/// between the first press and the second -- which is also why the confirming
/// button sits where the first one was.
fn confirm_row<'a, Message: Clone + 'a>(
    label: &'static str,
    question: String,
    kind: Confirm,
    theme: &Theme,
    confirm: Message,
    cancel: Message,
) -> Element<'a, Message, Theme> {
    let muted = theme.text_style(theme.muted_colour());
    let (word, destructive) = match kind {
        Confirm::Destructive(word) => (word, true),
        Confirm::Additive(word) => (word, false),
    };
    container(
        row![
            container(text(label).size(12).style(move |_t: &Theme| muted))
                .width(Length::Fixed(120.0)),
            container(text(question).size(13)).width(Fill).clip(true),
            iced::widget::button(text(word).size(12))
                .padding([2, 10])
                .on_press(confirm)
                .style(move |t: &Theme, status| if destructive {
                    t.danger_button(status)
                } else {
                    t.accent_button(status)
                }),
            iced::widget::button(text("Cancel").size(12))
                .padding([2, 10])
                .on_press(cancel)
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
