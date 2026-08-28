//! Overlays.
//!
//! `DESIGN.md` §9: one family, one set of rules. Four overlays with three
//! dismissal behaviours would be its own configuration burden.
//!
//! - **One at a time, never stacked.** Enforced by the state being one
//!   `Option`, not a collection: there is no way to express two.
//! - **Escape closes. The key that opened it toggles it.**
//! - Anything wanting to be a second simultaneous overlay is a pane, and panes
//!   are banned.
//!
//! That last rule is why this module holds no layout of its own beyond a scrim
//! and a frame. An overlay that needed to sit *beside* something would be a
//! pane wearing a different name.

pub mod artwork;
pub mod properties;
pub mod queue;

use crate::theme::Theme;
use iced::widget::{center, column, container, mouse_area, opaque, row, stack, text};
use iced::{Element, Fill};
use scoria_core::id::{AlbumId, TrackId};
use std::time::{Duration, Instant};

/// Which overlay is open.
///
/// One value, not a stack of them. §9's first rule is structural here rather
/// than something the code has to remember.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    /// The authoritative answer to "what plays next" (§9.2).
    Queue,
    /// Full-size album art. Normally the playing album, but the right-click
    /// action can name another (§9.1).
    Artwork(Option<AlbumId>),
    /// A metadata dump for one track (§9.3).
    Properties(TrackId),
}

impl Overlay {
    /// How big the frame should be, in pixels.
    ///
    /// Capped rather than proportional. A dialog that grows with the window
    /// ends up covering nearly all of it on a large screen, and something
    /// covering the screen is not read as an overlay whatever it is called.
    #[must_use]
    pub const fn max_size(self) -> (f32, f32) {
        match self {
            // A play order is a tall narrow thing. Wider would be empty space.
            Self::Queue => (760.0, 620.0),
            // Album art is square, and this is the one place §2's "dense over
            // pretty" gives way -- it exists to be looked at.
            Self::Artwork(_) => (720.0, 760.0),
            Self::Properties(_) => (680.0, 620.0),
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Queue => "Queue",
            Self::Artwork(_) => "Album art",
            Self::Properties(_) => "Properties",
        }
    }
}

/// Opens, closes or replaces, honouring §9's rules.
///
/// §9 says "the key that opened it toggles it", and the useful reading of that
/// is that the *same request* toggles. Toggling by kind instead would mean
/// asking for a second track's properties closed the dialog rather than showing
/// them: you asked to see something and got nothing.
///
/// So an identical request closes and anything else replaces. It never stacks;
/// there is one `Option`, so two cannot be expressed.
///
/// A function over values rather than a method, so the rule is testable without
/// a window.
#[must_use]
pub fn toggle(current: Option<Overlay>, requested: Overlay) -> Option<Overlay> {
    match current {
        Some(open) if open == requested => None,
        _ => Some(requested),
    }
}

/// How long an overlay takes to appear.
///
/// Short enough not to be a wait, long enough to be seen. The complaint it
/// answers is that opening one was easy to miss entirely: a thing that is
/// simply *there* on the next frame does not register as having arrived.
pub const OPEN_DURATION: Duration = Duration::from_millis(140);

/// Eased progress through the opening transition, 0 to 1.
///
/// Cubic ease-out: fast at the start so it feels immediate, settling at the end
/// so it does not look like it stopped abruptly.
#[must_use]
pub fn progress(since: Instant, now: Instant) -> f32 {
    let elapsed = now.saturating_duration_since(since).as_secs_f32();
    let linear = (elapsed / OPEN_DURATION.as_secs_f32()).clamp(0.0, 1.0);
    1.0 - (1.0 - linear).powi(3)
}

/// Puts `content` over `base` as a modal.
///
/// The scrim is `opaque`, which is what makes it modal: input stops there
/// instead of reaching the table underneath. Clicking it closes, which is the
/// behaviour every modal has and §9 does not need to state.
#[must_use]
pub fn present<'a, Message: Clone + 'a>(
    base: Element<'a, Message, Theme>,
    overlay: Overlay,
    content: Element<'a, Message, Theme>,
    theme: &Theme,
    progress: f32,
    on_close: Message,
) -> Element<'a, Message, Theme> {
    let frame_style = theme.overlay_style();
    let muted = theme.text_style(theme.muted_colour());

    let header = row![
        text(overlay.title()).size(16),
        container(text("Esc to close").size(11).style(move |_t: &Theme| muted))
            .width(Fill)
            .align_x(iced::Right),
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);

    // Grows the last few percent as it appears. Small on purpose: enough to
    // read as arriving, not so much that it looks like it is bouncing.
    let scale = 0.96 + 0.04 * progress.clamp(0.0, 1.0);
    let (max_width, max_height) = overlay.max_size();
    let (max_width, max_height) = (max_width * scale, max_height * scale);
    let frame = container(column![header, content].spacing(10))
        .padding(16)
        // Fill *up to* the cap, so a small window still gets a usable dialog
        // and a large one does not get a full-screen panel.
        .width(Fill)
        .height(Fill)
        .max_width(max_width)
        .max_height(max_height)
        .style(move |_t: &Theme| frame_style);

    // The frame swallows its own clicks, so clicking inside does not dismiss.
    let dialog = mouse_area(frame);

    let scrim_style = theme.scrim_style(progress);
    let scrim = mouse_area(
        container(center(opaque(dialog)))
            .width(Fill)
            .height(Fill)
            .style(move |_t: &Theme| scrim_style),
    )
    .on_press(on_close);

    stack![base, opaque(scrim)].width(Fill).height(Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_that_opened_it_closes_it() {
        assert_eq!(toggle(Some(Overlay::Queue), Overlay::Queue), None);
    }

    #[test]
    fn opening_another_replaces_rather_than_stacks() {
        // §9's first rule. There is no way to express two, which is the point.
        let open = toggle(Some(Overlay::Queue), Overlay::Artwork(None));
        assert_eq!(open, Some(Overlay::Artwork(None)));
    }

    #[test]
    fn properties_for_another_track_replaces_the_one_showing() {
        // Toggling is by kind, not by value: asking for a different track's
        // properties should swap the contents, not close the dialog.
        let open = toggle(
            Some(Overlay::Properties(TrackId(1))),
            Overlay::Properties(TrackId(2)),
        );
        assert_eq!(open, Some(Overlay::Properties(TrackId(2))));
    }

    #[test]
    fn the_same_request_closes_whatever_it_names() {
        for overlay in [
            Overlay::Queue,
            Overlay::Artwork(None),
            Overlay::Artwork(Some(AlbumId(3))),
            Overlay::Properties(TrackId(7)),
        ] {
            assert_eq!(
                toggle(Some(overlay), overlay),
                None,
                "{overlay:?} should have closed"
            );
        }
    }

    #[test]
    fn asking_for_the_playing_art_while_another_album_shows_switches_to_it() {
        // §9.1's escape hatch put a specific album on screen. Clicking the
        // transport thumbnail asks for what is *playing*, and answering that by
        // closing would ignore the request.
        let open = toggle(
            Some(Overlay::Artwork(Some(AlbumId(3)))),
            Overlay::Artwork(None),
        );
        assert_eq!(open, Some(Overlay::Artwork(None)));
    }

    #[test]
    fn the_transition_starts_hidden_and_finishes_shown() {
        let start = Instant::now();
        assert!(progress(start, start) < 0.01, "should start invisible");
        assert!(
            (progress(start, start + OPEN_DURATION) - 1.0).abs() < 0.01,
            "should finish"
        );
    }

    #[test]
    fn the_transition_never_leaves_its_range() {
        // The values scale a colour's alpha and the frame's size, so anything
        // outside 0..=1 would be a visual glitch rather than a slow animation.
        let start = Instant::now();
        for offset in [0, 10, 70, 139, 140, 500, 100_000] {
            let p = progress(start, start + Duration::from_millis(offset));
            assert!((0.0..=1.0).contains(&p), "{offset}ms gave {p}");
        }
    }

    #[test]
    fn the_transition_is_fast_at_the_start() {
        // Ease-out: past halfway by the time a third of the duration has gone,
        // so it feels immediate rather than sluggish.
        let start = Instant::now();
        assert!(progress(start, start + OPEN_DURATION / 3) > 0.5);
    }

    #[test]
    fn a_clock_that_goes_backwards_does_not_panic() {
        // saturating_duration_since rather than a subtraction: this runs on
        // every frame an overlay is open, and CLAUDE.md allows no known panics.
        let start = Instant::now();
        assert!(progress(start + Duration::from_secs(1), start) >= 0.0);
    }

    #[test]
    fn every_overlay_is_capped_below_a_typical_window() {
        // The complaint that produced this: a dialog filling the screen does
        // not read as an overlay, whatever it is called.
        for overlay in [
            Overlay::Queue,
            Overlay::Artwork(None),
            Overlay::Properties(TrackId(1)),
        ] {
            let (width, height) = overlay.max_size();
            assert!(width <= 900.0, "{overlay:?} is {width} wide");
            assert!(height <= 800.0, "{overlay:?} is {height} tall");
        }
    }

    #[test]
    fn opening_from_nothing_opens() {
        assert_eq!(toggle(None, Overlay::Queue), Some(Overlay::Queue));
    }
}
