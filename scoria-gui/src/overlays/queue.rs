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
use scoria_core::model::Track;

/// What the queue needs in order to draw itself.
pub struct QueueView<'a> {
    /// The same columns the library table uses. §7.3's configuration applies
    /// everywhere the table appears, because there is only one table.
    pub columns: &'a crate::widgets::columns::ColumnLayout,
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
    /// The row the user has clicked or arrowed onto.
    ///
    /// Separate from `position`: §11's roles exist because a track can be
    /// selected, playing, both, either or neither, and the queue is where all
    /// four happen. An earlier version made a click do nothing at all, on the
    /// grounds that selection in a play order *is* the position -- which is
    /// tidy reasoning and reads as a broken window.
    pub cursor: Option<usize>,
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
        selected: queue.cursor,
        // Marked by index rather than by id: a snapshot may hold the same track
        // twice, and only one of them is the one playing.
        playing_row: queue.position,
        playing: None,
        scroll_offset: queue.scroll_offset,
        theme,
        id: queue.id,
        columns: queue.columns,
        // The queue is a play order, not a view to sort or rearrange: its
        // headings show no sort marker and its edges do not drag.
        sort: None,
        resizing: false,
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
