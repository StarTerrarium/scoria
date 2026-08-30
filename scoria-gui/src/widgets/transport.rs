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
use crate::widgets::icon::{self, Icon};
use iced::widget::{Space, button, column, container, mouse_area, row, slider, text, tooltip};
use iced::{Alignment, Element, Fill, Length};
use scoria_core::event::PlaybackState;
use scoria_core::model::Track;
use scoria_core::playback::RepeatMode;
use std::time::Duration;

/// How much one notch of the wheel moves the volume.
///
/// A twentieth of the range: twenty notches from silent to full is enough to
/// land on a particular loudness without spinning, and coarse enough that one
/// notch is worth doing.
const VOLUME_STEP: f32 = 0.05;

/// Roughly one wheel notch, in pixels.
///
/// Trackpads report scrolling in pixels rather than in lines, and there is no
/// authoritative conversion -- this is the conventional approximation. Getting
/// it wrong makes a trackpad faster or slower than a wheel, not broken.
const PIXELS_PER_NOTCH: f32 = 50.0;

/// How tall a slider is to the pointer.
///
/// Not how thick it looks. The theme draws a 3px rail and a 10px handle, and
/// both are centred in whatever box they are given, so this decides only what
/// answers a click.
///
/// The seek bar was 6px -- smaller than the handle drawn on it, so the target
/// you could see was bigger than the one that responded, and catching it
/// mid-drag took aim. iced hit-tests a slider against its layout rectangle,
/// and its own default height is 16.
///
/// **Both sliders read this.** The volume bar was already 16px, by never
/// having overridden the default, so for a while the two agreed by accident.
/// They are the same control drawn at two widths and they should take the same
/// aim, which is a fact worth holding in one place rather than leaving to two
/// defaults that happen to match.
const SLIDER_HEIGHT: f32 = 16.0;

/// The side of the artwork slot.
///
/// One number for the image and the frame around it, so the slot does not
/// change size depending on whether there is anything in it.
const ARTWORK: f32 = 38.0;

/// The volume after a scroll over the bar.
///
/// Up is louder, which is the only arrangement anyone expects.
#[must_use]
fn scrolled_volume(current: f32, delta: iced::mouse::ScrollDelta) -> f32 {
    let notches = match delta {
        iced::mouse::ScrollDelta::Lines { y, .. } => y,
        iced::mouse::ScrollDelta::Pixels { y, .. } => y / PIXELS_PER_NOTCH,
    };
    // Clamped rather than wrapped: a wheel that rolls past full and starts
    // again at silence would be a very unpleasant surprise.
    (current + notches * VOLUME_STEP).clamp(0.0, 1.0)
}

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
    // Copied out before the closures below, which outlive the borrow.
    let volume = transport.volume;

    // Drawn, not typed. See `widgets::icon` -- the symbols used to be whatever
    // the machine's fonts supplied, which is why some of them arrived orange.
    let symbol = move |icon: Icon, hint: &'static str, event: Event, active: bool| {
        let control = button(icon::view(icon, active))
            .padding([2, 8])
            .on_press(on_event(event))
            .style(move |t: &Theme, status| t.transport_button(status, active));
        tooltip(control, text(hint).size(12), tooltip::Position::Top)
    };

    // The thumbnail. Artwork always follows what is playing (§9.1), so this
    // needs no mode and no toggle -- it shows whatever the transport shows.
    //
    // The slot is drawn either way. With nothing playing this used to be a
    // music note floating in the corner of the bar with no edge to it, which
    // read as a stray character rather than as a place art goes -- and it was
    // the last symbol here still coming from whatever font had one.
    let slot = theme.artwork_slot_style();
    let thumbnail: Element<'a, Message, Theme> = match transport.artwork.clone() {
        Some(path) => iced::widget::image(path)
            .width(Length::Fixed(ARTWORK))
            .height(Length::Fixed(ARTWORK))
            .content_fit(iced::ContentFit::Cover)
            .into(),
        None => icon::view(Icon::Note, false),
    };
    let artwork = button(
        container(thumbnail)
            .width(Length::Fixed(ARTWORK))
            .height(Length::Fixed(ARTWORK))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .clip(true)
            .style(move |_t: &Theme| slot),
    )
    .padding(1)
    .on_press(on_event(Event::ShowArtwork))
    .style(|t: &Theme, status| t.transport_button(status, false));

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
    //
    // With nothing playing it is one line, not a title over an empty second
    // one. The row centres what it is given, so a block of one line centres
    // itself; a block of two with the bottom one blank sat high and looked
    // like the subtitle had failed to load.
    let now_playing = container(match transport.track {
        Some(track) => Element::from(
            column![
                text(track.title.clone()).size(14),
                // §8: `artist — album`. The display artist, with full credits.
                text(format!("{} — {}", track.artist, track.album))
                    .size(12)
                    .style(move |_t: &Theme| muted),
            ]
            .spacing(1),
        ),
        // Muted: it is the absence of information rather than information,
        // and at full strength it was the brightest thing in an idle bar.
        None => Element::from(
            text("Nothing playing")
                .size(14)
                .style(move |_t: &Theme| muted),
        ),
    })
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
        .height(SLIDER_HEIGHT)
        .on_release(on_event(Event::SeekTo(shown_position)))
        .width(Fill)
        .into()
    } else {
        // §8 has always called for an inert strip here, and there was none:
        // an empty Space left a gap where the bar belongs, so an idle
        // transport looked unfinished rather than idle. The rail without a
        // handle says there is a bar, and nothing to move along it.
        let rail = theme.seek_rail_style();
        container(
            container(
                Space::new()
                    .width(Fill)
                    .height(Length::Fixed(Theme::RAIL_THICKNESS)),
            )
            .style(move |_t: &Theme| rail),
        )
        .width(Fill)
        .height(Length::Fixed(SLIDER_HEIGHT))
        .align_y(Alignment::Center)
        .into()
    };

    let time = move |value: String| {
        container(text(value).size(12).style(move |_t: &Theme| muted))
            .width(Length::Fixed(44.0))
            .align_x(Alignment::Center)
    };

    let bar = row![
        artwork,
        symbol(Icon::Previous, "Previous", Event::Previous, false),
        symbol(
            match transport.state {
                PlaybackState::Playing => Icon::Pause,
                PlaybackState::Paused | PlaybackState::Stopped => Icon::Play,
            },
            "Play or pause",
            Event::PlayPause,
            false,
        ),
        symbol(Icon::Next, "Next", Event::Next, false),
        now_playing,
        time(format_duration(shown_position)),
        seek,
        time(format_duration(duration)),
        // Shuffle and repeat sit past the seek bar rather than among the
        // transport buttons: they change what happens next, which is a
        // different kind of thing from moving through what is playing now.
        symbol(
            Icon::Shuffle,
            "Shuffle",
            Event::ToggleShuffle,
            transport.shuffled,
        ),
        symbol(
            match transport.repeat {
                RepeatMode::One => Icon::RepeatOne,
                RepeatMode::Off | RepeatMode::All => Icon::Repeat,
            },
            "Repeat",
            Event::CycleRepeat,
            transport.repeat != RepeatMode::Off,
        ),
        // §8: a target, matching the "reveal current item" convention from
        // editors and file managers.
        symbol(
            Icon::JumpToPlaying,
            "Jump to what is playing",
            Event::JumpToPlaying,
            false,
        ),
        // The queue needs a way in that does not require knowing a keybinding.
        // §10 makes the keyboard a complete path, not the only one.
        symbol(
            Icon::Queue,
            "Queue",
            Event::ToggleQueue,
            transport.queue_open
        ),
        // The wheel changes the volume while the pointer is over the bar,
        // which is where a hand already is when it wants to. `mouse_area`
        // hands the event to the slider first and only sees what the slider
        // ignored, so dragging still works exactly as it did.
        mouse_area(
            slider(0.0..=1.0, transport.volume, move |v| on_event(
                Event::SetVolume(v)
            ))
            .step(0.01)
            .height(SLIDER_HEIGHT)
            .width(Length::Fixed(90.0)),
        )
        .on_scroll(move |delta| on_event(Event::SetVolume(scrolled_volume(volume, delta)))),
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

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;
    use iced::mouse::ScrollDelta;

    fn lines(y: f32) -> ScrollDelta {
        ScrollDelta::Lines { x: 0.0, y }
    }

    #[test]
    fn a_slider_is_at_least_as_tall_as_the_handle_drawn_on_it() {
        // The seek bar was not. iced hit-tests a slider against its layout
        // rectangle, so the height is the whole of what answers a click -- and
        // at 6px it was smaller than the 10px handle the theme draws on it.
        // The target you could see was bigger than the one that responded.
        //
        // Covers the volume bar too, because both read one constant and the
        // theme styles every slider the same way.
        //
        // Asked of the theme rather than written down here, so that growing the
        // handle without growing the box it lives in fails rather than quietly
        // reintroducing this.
        for theme in Theme::all() {
            let style = theme.slider_style(iced::widget::slider::Status::Active);
            let iced::widget::slider::HandleShape::Circle { radius } = style.handle.shape else {
                continue;
            };
            assert!(
                SLIDER_HEIGHT >= radius * 2.0,
                "{}: a {}px handle in a {SLIDER_HEIGHT}px box",
                theme.name,
                radius * 2.0
            );
        }
    }

    #[test]
    fn up_is_louder_and_down_is_quieter() {
        assert_eq!(scrolled_volume(0.5, lines(1.0)), 0.55);
        assert_eq!(scrolled_volume(0.5, lines(-1.0)), 0.45);
    }

    #[test]
    fn the_wheel_stops_at_the_ends_rather_than_wrapping() {
        // A wheel that rolled past full and started again at silence would be
        // a very unpleasant surprise, and this is a control people spin.
        assert_eq!(scrolled_volume(1.0, lines(1.0)), 1.0);
        assert_eq!(scrolled_volume(0.0, lines(-1.0)), 0.0);
        assert_eq!(scrolled_volume(0.98, lines(5.0)), 1.0);
        assert_eq!(scrolled_volume(0.02, lines(-5.0)), 0.0);
    }

    #[test]
    fn a_trackpad_moves_it_too() {
        // Trackpads report pixels rather than lines. The conversion is an
        // approximation; what matters is that it moves in the right direction
        // and by a comparable amount.
        let notch = ScrollDelta::Pixels {
            x: 0.0,
            y: PIXELS_PER_NOTCH,
        };
        assert_eq!(scrolled_volume(0.5, notch), 0.55);

        let nudge = ScrollDelta::Pixels { x: 0.0, y: 5.0 };
        let after = scrolled_volume(0.5, nudge);
        assert!(
            after > 0.5 && after < 0.55,
            "a small nudge moved it {after}"
        );
    }

    #[test]
    fn a_sideways_scroll_leaves_the_volume_alone() {
        // Horizontal scrolling is a real thing on trackpads and tilt wheels,
        // and it is not asking for anything here.
        assert_eq!(
            scrolled_volume(0.5, ScrollDelta::Lines { x: 3.0, y: 0.0 }),
            0.5
        );
    }

    #[test]
    fn twenty_notches_cross_the_whole_range() {
        // Enough to land on a particular loudness without spinning, coarse
        // enough that one notch is worth doing.
        let mut volume = 0.0;
        for _ in 0..20 {
            volume = scrolled_volume(volume, lines(1.0));
        }
        assert_eq!(volume, 1.0);
    }
}
