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
use iced::widget::{button, column, container, row, slider, text, tooltip};
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

    let elapsed = format_duration(transport.position);
    let total = transport
        .track
        .map_or_else(|| "0:00".to_owned(), |t| format_duration(t.duration));

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

    let style = theme.surface_style();
    container(bar)
        .width(Fill)
        .style(move |_t: &Theme| style)
        .into()
}
