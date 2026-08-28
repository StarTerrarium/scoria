//! Shared widgets.
//!
//! `table` is the one track table (invariant 3). `transport` is the bar §8
//! describes. Layouts arrange these; they never define their own.

pub mod panes;
pub mod table;
pub mod transport;

use std::time::Duration;

/// Gap between an embedded scrollbar and the content beside it.
///
/// iced floats scrollbars over the content by default, which puts the scrollbar
/// on top of whatever is at the right edge -- in a filter pane, that is the
/// count. Giving the scrollable a spacing embeds it in the layout instead, so
/// the content is narrowed rather than covered.
pub const SCROLLBAR_SPACING: f32 = 2.0;

/// How much width an embedded scrollbar takes: iced's default 10px bar plus
/// [`SCROLLBAR_SPACING`].
///
/// Anything drawn *outside* a scrollable but meant to line up with the rows
/// inside it -- a column header, a pinned `All` row -- has to reserve the same,
/// or its right-hand column sits proud of the ones below it.
pub const SCROLLBAR_ALLOWANCE: f32 = 10.0 + SCROLLBAR_SPACING;

/// `3:47`, or `1:02:11` for something long.
///
/// Here rather than in the table because the transport bar shows the same
/// thing, and two implementations would eventually disagree about a boundary.
#[must_use]
pub fn format_duration(d: Duration) -> String {
    let total = d.as_secs();
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_the_way_a_transport_shows_them() {
        assert_eq!(format_duration(Duration::from_secs(0)), "0:00");
        assert_eq!(format_duration(Duration::from_secs(227)), "3:47");
        assert_eq!(format_duration(Duration::from_secs(3731)), "1:02:11");
    }
}
