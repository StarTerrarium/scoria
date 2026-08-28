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
use iced::{Element, Fill, Length};
use scoria_core::id::{AlbumId, TrackId};

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
    on_close: Message,
) -> Element<'a, Message, Theme> {
    let frame_style = theme.overlay_style();
    let muted = theme.text_style(theme.muted_colour());

    let header = row![
        text(overlay.title()).size(15),
        container(text("Esc to close").size(11).style(move |_t: &Theme| muted))
            .width(Fill)
            .align_x(iced::Right),
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);

    let frame = container(column![header, content].spacing(10))
        .padding(14)
        .width(Length::FillPortion(3))
        .height(Length::FillPortion(3))
        .style(move |_t: &Theme| frame_style);

    // The frame swallows its own clicks, so clicking inside does not dismiss.
    let dialog = mouse_area(frame);

    let scrim_style = theme.scrim_style();
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
    fn opening_from_nothing_opens() {
        assert_eq!(toggle(None, Overlay::Queue), Some(Overlay::Queue));
    }
}
