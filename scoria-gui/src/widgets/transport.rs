//! The transport bar.
//!
//! `DESIGN.md` §8: always visible, identical across all three layouts.
//! Artwork thumbnail, symbol-only controls, shuffle and repeat, track title,
//! `artist — album`, jump-to-playing, elapsed and total, volume.
//!
//! Symbols only, with tooltips. §8 is clear that tooltips solve discovery for
//! mouse users only, and that the `?` cheatsheet covers the keyboard path --
//! so the tooltip text here is a convenience, not the documentation.

use crate::theme::Theme;
use crate::widgets::format_duration;
use iced::widget::{Space, button, column, container, row, slider, text, tooltip};
use iced::{Alignment, Element, Fill, Length};
use scoria_core::event::PlaybackState;
use scoria_core::model::Track;
use scoria_core::playback::RepeatMode;
use std::time::Duration;

/// What the bar shows.
pub struct TransportView<'a> {
    pub track: Option<&'a Track>,
    /// The playing album's cached artwork. §9.1: it follows what is playing,
    /// with no mode and no toggle.
    pub artwork: Option<std::path::PathBuf>,
    /// Whether the queue is open, so its button can show it.
    pub queue_open: bool,
    pub state: PlaybackState,
    pub position: Duration,
    /// Where the user is dragging the seek bar, if they are.
    ///
    /// While dragging, this is what the bar and the elapsed time show. Letting
    /// the engine's own position updates through would drag the handle back out
    /// from under the pointer several times a second.
    pub scrubbing: Option<Duration>,
    pub shuffled: bool,
    pub repeat: RepeatMode,
    pub volume: f32,
    pub theme: &'a Theme,
}

/// What the bar can ask for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    PlayPause,
    Next,
    Previous,
    ToggleShuffle,
    CycleRepeat,
    JumpToPlaying,
    SetVolume(f32),
    /// Clicking the thumbnail opens the artwork overlay (§9.1).
    ShowArtwork,
    /// §9.2's queue, reachable without knowing a keybinding.
    ToggleQueue,
    /// The seek bar is being dragged, to this position in seconds.
    Scrub(f32),
    /// The drag finished: go there.
    SeekTo(Duration),
}

#[must_use]
pub fn view<'a, Message: Clone + 'a>(
    transport: &TransportView<'_>,
    on_event: impl Fn(Event) -> Message + Copy + 'a,
) -> Element<'a, Message, Theme> {
    let theme = transport.theme;

    let symbol = move |glyph: &'static str, hint: &'static str, event: Event, active: bool| {
        let control = button(text(glyph).size(15))
            .padding([2, 8])
            .on_press(on_event(event))
            .style(move |t: &Theme, status| t.transport_button(status, active));
        tooltip(control, text(hint).size(12), tooltip::Position::Top)
    };

    // The thumbnail. Artwork always follows what is playing (§9.1), so this
    // needs no mode and no toggle -- it shows whatever the transport shows.
    let thumbnail: Element<'a, Message, Theme> = match transport.artwork.clone() {
        Some(path) => iced::widget::image(path)
            .width(Length::Fixed(38.0))
            .height(Length::Fixed(38.0))
            .content_fit(iced::ContentFit::Cover)
            .into(),
        None => text("♪").size(20).into(),
    };
    let artwork = button(
        container(thumbnail)
            .width(Length::Fixed(40.0))
            .height(Length::Fixed(40.0))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .clip(true),
    )
    .padding(1)
    .on_press(on_event(Event::ShowArtwork))
    .style(|t: &Theme, status| t.transport_button(status, false));

    let title = transport
        .track
        .map_or_else(|| "Nothing playing".to_owned(), |t| t.title.clone());
    let subtitle = transport.track.map_or_else(String::new, |t| {
        // §8: `artist — album`. The display artist, with its full credits.
        format!("{} — {}", t.artist, t.album)
    });

    let muted = theme.text_style(theme.muted_colour());

    // While dragging, the time reads where the pointer is rather than where
    // playback still is, so the number and the handle agree.
    let shown_position = transport.scrubbing.unwrap_or(transport.position);
    let duration = transport.track.map_or(Duration::ZERO, |t| t.duration);

    // The now-playing block gets a fixed width rather than filling.
    //
    // Letting it fill was the mistake in the first version: a title and one
    // subtitle line is about three hundred pixels of content, so filling the
    // bar left a void in the middle and pushed the clock and volume to the far
    // edge. Fixed width keeps them where the eye expects and hands the slack to
    // the seek bar, which is the one control that benefits from every pixel.
    let now_playing = container(
        column![
            text(title).size(14),
            text(subtitle).size(12).style(move |_t: &Theme| muted),
        ]
        .spacing(1),
    )
    .width(Length::Fixed(300.0))
    .clip(true);

    // The seek bar sits between the two times, which is where a clock belongs:
    // elapsed on the left of the bar it is measuring, total on the right.
    //
    // A track of unknown or zero length gets an inert strip rather than a
    // movable handle. There is nowhere to seek to, and a control that moves
    // without doing anything is worse than one that does not move.
    let seconds = duration.as_secs_f32();
    let seek: Element<'a, Message, Theme> = if seconds > 0.0 {
        slider(
            0.0..=seconds,
            shown_position.as_secs_f32().min(seconds),
            move |v| on_event(Event::Scrub(v)),
        )
        .step(0.1_f32)
        .height(6.0)
        .on_release(on_event(Event::SeekTo(shown_position)))
        .width(Fill)
        .into()
    } else {
        Space::new().height(Length::Fixed(6.0)).width(Fill).into()
    };

    let time = move |value: String| {
        container(text(value).size(12).style(move |_t: &Theme| muted))
            .width(Length::Fixed(44.0))
            .align_x(Alignment::Center)
    };

    let bar = row![
        artwork,
        symbol("⏮", "Previous", Event::Previous, false),
        symbol(
            match transport.state {
                PlaybackState::Playing => "⏸",
                PlaybackState::Paused | PlaybackState::Stopped => "▶",
            },
            "Play or pause",
            Event::PlayPause,
            false,
        ),
        symbol("⏭", "Next", Event::Next, false),
        now_playing,
        time(format_duration(shown_position)),
        seek,
        time(format_duration(duration)),
        // Shuffle and repeat sit past the seek bar rather than among the
        // transport buttons: they change what happens next, which is a
        // different kind of thing from moving through what is playing now.
        symbol("⇄", "Shuffle", Event::ToggleShuffle, transport.shuffled),
        symbol(
            match transport.repeat {
                RepeatMode::One => "⟳1",
                RepeatMode::Off | RepeatMode::All => "⟳",
            },
            "Repeat",
            Event::CycleRepeat,
            transport.repeat != RepeatMode::Off,
        ),
        // §8: a crosshair, matching the "reveal current item" convention from
        // editors and file managers.
        symbol("⌖", "Jump to what is playing", Event::JumpToPlaying, false),
        // The queue needs a way in that does not require knowing a keybinding.
        // §10 makes the keyboard a complete path, not the only one.
        symbol("≡", "Queue", Event::ToggleQueue, transport.queue_open),
        slider(0.0..=1.0, transport.volume, move |v| on_event(
            Event::SetVolume(v)
        ))
        .step(0.01)
        .width(Length::Fixed(90.0)),
    ]
    .spacing(8)
    .padding([6, 10])
    .align_y(Alignment::Center);

    let style = theme.surface_style();
    container(bar)
        .width(Fill)
        .style(move |_t: &Theme| style)
        .into()
}
