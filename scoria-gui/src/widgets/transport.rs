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
    let artwork = button(
        container(text("♪").size(20))
            .width(Length::Fixed(40.0))
            .height(Length::Fixed(40.0))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
    )
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
    let now_playing = column![
        text(title).size(14),
        text(subtitle).size(12).style(move |_t: &Theme| muted),
    ]
    .spacing(1)
    .width(Fill);

    // While dragging, the time reads where the pointer is rather than where
    // playback still is, so the number and the handle agree.
    let shown_position = transport.scrubbing.unwrap_or(transport.position);
    let elapsed = format_duration(shown_position);
    let duration = transport.track.map_or(Duration::ZERO, |t| t.duration);
    let total = format_duration(duration);

    let repeat_glyph = match transport.repeat {
        RepeatMode::Off | RepeatMode::All => "⟳",
        RepeatMode::One => "⟳¹",
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
        symbol("⤨", "Shuffle", Event::ToggleShuffle, transport.shuffled),
        symbol(
            repeat_glyph,
            "Repeat",
            Event::CycleRepeat,
            transport.repeat != RepeatMode::Off
        ),
        now_playing,
        // §8: a crosshair, matching the "reveal current item" convention from
        // editors and file managers.
        symbol("⌖", "Jump to what is playing", Event::JumpToPlaying, false),
        text(format!("{elapsed} / {total}"))
            .size(12)
            .style(move |_t: &Theme| muted),
        slider(0.0..=1.0, transport.volume, move |v| on_event(
            Event::SetVolume(v)
        ))
        .step(0.01)
        .width(Length::Fixed(90.0)),
    ]
    .spacing(8)
    .padding([6, 10])
    .align_y(Alignment::Center);

    // A full-width bar along the top edge of the transport.
    //
    // Inline between the title and the clock would have been the conventional
    // placement, but it would compress the one part of the bar that carries
    // information. Full width also gives the most precision, which is what a
    // ten-minute track needs. §2: dense over pretty.
    //
    // A track of unknown or zero length gets an inert strip rather than a
    // movable control: there is nowhere to seek to, and a handle that moves
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

    let style = theme.surface_style();
    container(column![seek, bar].spacing(2))
        .width(Fill)
        .style(move |_t: &Theme| style)
        .into()
}
