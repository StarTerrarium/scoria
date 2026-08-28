//! The queue.
//!
//! `DESIGN.md` §9.2. Modal, on top of the explorer, and modality is the point:
//! being able to see both at once would recreate the ambiguity the separation
//! exists to remove.
//!
//! It uses the track table pointed at `tracks` -- the same widget the library
//! view uses, which is invariant 3 doing its job. If the queue needed its own
//! table, the table would be wrong.
//!
//! The overlay exists because `tracks` and `origin` can diverge (§6.2). Without
//! it there is no authoritative answer to "what plays next", and a Next button
//! that disagrees with the visible list reads as a bug.

use crate::theme::Theme;
use crate::widgets::table;
use iced::widget::{column, container, text};
use iced::{Element, Fill};
use scoria_core::id::TrackId;
use scoria_core::model::Track;

/// What the queue needs in order to draw itself.
pub struct QueueView<'a> {
    /// The display list, owned by the caller.
    ///
    /// No groups: the snapshot is a play order, and a header wherever the album
    /// changed would suggest the order means something it does not (§6.2).
    pub lines: &'a [table::Line],
    /// The snapshot, in its own order. Already-played tracks included: §9.2
    /// says the whole snapshot, because "what plays next" is only meaningful
    /// against where you are in it.
    pub rows: Vec<&'a Track>,
    /// Index into the snapshot of the track playing now.
    pub position: Option<usize>,
    pub playing: Option<TrackId>,
    pub scroll_offset: f32,
    /// Set when the view's contents no longer match the snapshot.
    pub divergence: Option<String>,
    pub theme: &'a Theme,
    pub id: &'static str,
}

#[must_use]
pub fn view<'a, Message: Clone + 'a>(
    queue: QueueView<'a>,
    on_event: impl Fn(table::Event) -> Message + Copy + 'a,
) -> Element<'a, Message, Theme> {
    if queue.rows.is_empty() {
        let muted = queue.theme.text_style(queue.theme.muted_colour());
        return container(
            text("Nothing queued")
                .size(13)
                .style(move |_t: &Theme| muted),
        )
        .width(Fill)
        .height(Fill)
        .align_x(iced::Alignment::Center)
        .align_y(iced::Alignment::Center)
        .into();
    }

    let theme = queue.theme;

    let table_view = table::TableView {
        lines: queue.lines,
        rows: queue.rows,
        // The current position is shown as the selection, because in the queue
        // that is what selection means: where you are.
        selected: queue.position,
        playing: queue.playing,
        scroll_offset: queue.scroll_offset,
        theme,
        id: queue.id,
    };

    let mut body = column![table::view(table_view, on_event)]
        .height(Fill)
        .width(Fill);

    // §9.2: a one-line explanation *only* when they differ. Always present, it
    // would be clutter; present only when it applies, it answers the question
    // the queue exists to answer.
    if let Some(divergence) = queue.divergence {
        let muted = theme.text_style(theme.muted_colour());
        body = body.push(
            container(text(divergence).size(12).style(move |_t: &Theme| muted))
                .padding([4, 2])
                .width(Fill),
        );
    }

    body.into()
}
