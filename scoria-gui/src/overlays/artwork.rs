//! Full-size album art.
//!
//! `DESIGN.md` §9.1. **Artwork always follows what is playing.** No mode, no
//! toggle -- this is a global rule, applied everywhere artwork appears, and the
//! only exception is the right-click action that opens this overlay for a
//! *specific* album without disturbing the ambient rule.
//!
//! That exception is why [`ArtworkView::album`] exists at all. It is not a
//! follow-selection mode in disguise: it is set once, by an explicit action, on
//! an overlay the user opened deliberately.

use crate::theme::Theme;
use iced::widget::{column, container, text};
use iced::{ContentFit, Element, Fill};
use std::path::PathBuf;

pub struct ArtworkView<'a> {
    /// The cached image, if there is one.
    pub image: Option<PathBuf>,
    /// What the art belongs to, for the caption.
    pub caption: Option<String>,
    pub theme: &'a Theme,
}

#[must_use]
pub fn view<'a, Message: 'a>(artwork: ArtworkView<'a>) -> Element<'a, Message, Theme> {
    let theme = artwork.theme;
    let muted = theme.text_style(theme.muted_colour());

    let picture: Element<'a, Message, Theme> = match artwork.image {
        Some(path) => iced::widget::image(path)
            .width(Fill)
            .height(Fill)
            .content_fit(ContentFit::Contain)
            .into(),
        None => container(text("No artwork").size(13).style(move |_t: &Theme| muted))
            .width(Fill)
            .height(Fill)
            .align_x(iced::Alignment::Center)
            .align_y(iced::Alignment::Center)
            .into(),
    };

    let mut content = column![picture].spacing(8).width(Fill).height(Fill);
    if let Some(caption) = artwork.caption {
        content = content.push(
            container(text(caption).size(12).style(move |_t: &Theme| muted))
                .width(Fill)
                .align_x(iced::Alignment::Center),
        );
    }
    content.into()
}
