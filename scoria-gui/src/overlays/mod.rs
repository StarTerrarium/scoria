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
use iced::widget::{column, container, mouse_area, opaque, row, text};
use iced::{Element, Fill, Length};
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
    /// How tall the drawer should be, in pixels.
    ///
    /// Capped rather than proportional. A panel that grows with the window ends
    /// up covering it, and something covering the screen is not read as an
    /// overlay whatever it is called.
    #[must_use]
    pub const fn max_height(self) -> f32 {
        match self {
            Self::Queue => 560.0,
            // Album art is square and exists to be looked at: the one place
            // §2's "dense over pretty" gives way.
            Self::Artwork(_) => 640.0,
            Self::Properties(_) => 560.0,
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

/// An overlay on screen, and where it is in its transition.
///
/// Closing is a state rather than an absence, because an overlay that vanished
/// the instant it was dismissed would be lopsided against one that rises: the
/// same motion has to run backwards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Presenting {
    pub overlay: Overlay,
    /// When the current transition started.
    pub since: Instant,
    pub closing: bool,
}

impl Presenting {
    #[must_use]
    pub fn opening(overlay: Overlay, now: Instant) -> Self {
        Self {
            overlay,
            since: now,
            closing: false,
        }
    }

    /// Starts retracting, from wherever the opening transition had reached.
    ///
    /// Rewinding rather than restarting: dismissing a drawer that is still
    /// rising should retract from its current height, not jump to full and fall.
    #[must_use]
    pub fn closing(self, now: Instant) -> Self {
        if self.closing {
            return self;
        }
        // The rewind has to happen in *linear* time, not in eased output.
        // Subtracting an eased fraction from the clock lands somewhere else on
        // the curve, and the drawer jumps at the moment of dismissal.
        //
        // Solving `1 - ease(p) = shown` for the cubic ease-out gives
        // `p = 1 - cbrt(shown)`.
        let shown = progress(self.since, now).clamp(0.0, 1.0);
        let already_retracted = OPEN_DURATION.mul_f32(1.0 - shown.cbrt());
        Self {
            overlay: self.overlay,
            since: now.checked_sub(already_retracted).unwrap_or(now),
            closing: true,
        }
    }

    /// How much of the drawer is showing, 0 to 1.
    #[must_use]
    pub fn shown(self, now: Instant) -> f32 {
        let p = progress(self.since, now);
        if self.closing { 1.0 - p } else { p }
    }

    /// Whether the transition has finished and the overlay can be dropped.
    #[must_use]
    pub fn is_finished(self, now: Instant) -> bool {
        self.closing && progress(self.since, now) >= 1.0
    }

    /// What the rest of the application should consider open.
    ///
    /// A closing overlay is already gone as far as anything but drawing is
    /// concerned: pressing its key again should reopen it rather than close
    /// something that is on its way out.
    #[must_use]
    pub const fn effective(self) -> Option<Overlay> {
        if self.closing {
            None
        } else {
            Some(self.overlay)
        }
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

/// Builds the drawer and the dimming behind it, as one layer.
///
/// Returns the layer rather than a finished view: where it goes is the layout's
/// decision, and the layout puts it over the explorer only.
///
/// **The transport bar is never covered.** §9.2's modality is about not seeing
/// the browse list and the play order at once, and the transport is neither --
/// it is the thing the queue is *about*, so hiding it would remove the ability
/// to pause or skip while reading the queue and gain nothing.
///
/// The drawer rises from the bottom edge, which is where the transport is, so
/// it reads as belonging to it.
#[must_use]
pub fn present<'a, Message: Clone + 'a>(
    overlay: Overlay,
    content: Element<'a, Message, Theme>,
    theme: &Theme,
    progress: f32,
    on_close: Message,
) -> Element<'a, Message, Theme> {
    let progress = progress.clamp(0.0, 1.0);
    let frame_style = theme.overlay_style();
    let muted = theme.text_style(theme.muted_colour());

    // Deliberately generous. A heading small enough to be efficient would not
    // announce that anything had opened, which is the whole problem this is
    // solving.
    let header = row![
        text(overlay.title()).size(24),
        container(text("Esc to close").size(11).style(move |_t: &Theme| muted))
            .width(Fill)
            .align_x(iced::Right),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center);

    // Height carries the animation: the drawer rises rather than fading in.
    let drawer = container(column![header, content].spacing(12))
        .padding([14, 18])
        .width(Fill)
        .height(Length::Fixed(overlay.max_height() * progress))
        .style(move |_t: &Theme| frame_style);

    // Clicks inside the drawer belong to it, not to the scrim behind.
    let drawer = opaque(mouse_area(drawer));

    let scrim_style = theme.scrim_style(progress);
    mouse_area(
        container(drawer)
            .width(Fill)
            .height(Fill)
            .align_y(iced::Bottom)
            .style(move |_t: &Theme| scrim_style),
    )
    .on_press(on_close)
    .into()
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
    fn every_drawer_leaves_the_library_visible_above_it() {
        // The complaint that produced this: a panel filling the screen does not
        // read as an overlay whatever it is called.
        for overlay in [
            Overlay::Queue,
            Overlay::Artwork(None),
            Overlay::Properties(TrackId(1)),
        ] {
            assert!(
                overlay.max_height() <= 660.0,
                "{overlay:?} is {} tall",
                overlay.max_height()
            );
        }
    }

    #[test]
    fn closing_runs_the_same_motion_backwards() {
        let start = Instant::now();
        let open = Presenting::opening(Overlay::Queue, start);
        assert!(open.shown(start) < 0.01);
        assert!(open.shown(start + OPEN_DURATION) > 0.99);

        let shut = open.closing(start + OPEN_DURATION);
        assert!(
            shut.shown(start + OPEN_DURATION) > 0.99,
            "should start from full"
        );
        assert!(
            shut.shown(start + OPEN_DURATION * 2) < 0.01,
            "should end at nothing"
        );
    }

    #[test]
    fn dismissing_mid_rise_retracts_from_where_it_reached() {
        // Restarting the clock would make a half-open drawer jump to full and
        // then fall, which is worse than not animating at all.
        let start = Instant::now();
        let halfway = start + OPEN_DURATION / 3;

        let open = Presenting::opening(Overlay::Queue, start);
        let shown = open.shown(halfway);
        let shut = open.closing(halfway);

        assert!(
            (shut.shown(halfway) - shown).abs() < 0.02,
            "jumped from {shown} to {}",
            shut.shown(halfway)
        );
    }

    #[test]
    fn a_closing_overlay_is_already_gone_to_everything_but_drawing() {
        // Pressing its key again should reopen it, not close something that is
        // already on its way out.
        let now = Instant::now();
        let shut = Presenting::opening(Overlay::Queue, now).closing(now);
        assert_eq!(shut.effective(), None);
        assert_eq!(
            toggle(shut.effective(), Overlay::Queue),
            Some(Overlay::Queue)
        );
    }

    #[test]
    fn a_finished_close_can_be_dropped() {
        let start = Instant::now();
        let shown = start + OPEN_DURATION;
        let shut = Presenting::opening(Overlay::Queue, start).closing(shown);

        assert!(!shut.is_finished(shown), "should still be retracting");
        assert!(shut.is_finished(shown + OPEN_DURATION), "should be done");
    }

    #[test]
    fn dismissing_one_that_never_appeared_is_immediate() {
        // Nothing to retract, so nothing to watch retract.
        let now = Instant::now();
        let shut = Presenting::opening(Overlay::Queue, now).closing(now);
        assert!(shut.is_finished(now));
    }

    #[test]
    fn an_opening_overlay_is_never_finished() {
        // Dropping one that is still rising would make it vanish mid-animation.
        let now = Instant::now();
        let open = Presenting::opening(Overlay::Queue, now);
        assert!(!open.is_finished(now));
        assert!(!open.is_finished(now + OPEN_DURATION * 10));
    }

    #[test]
    fn closing_twice_does_not_restart_the_retraction() {
        let now = Instant::now();
        let shut = Presenting::opening(Overlay::Queue, now).closing(now);
        let again = shut.closing(now + OPEN_DURATION / 2);
        assert_eq!(shut.since, again.since);
    }

    #[test]
    fn opening_from_nothing_opens() {
        assert_eq!(toggle(None, Overlay::Queue), Some(Overlay::Queue));
    }
}
