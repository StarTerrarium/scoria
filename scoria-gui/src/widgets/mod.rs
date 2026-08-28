//! Shared widgets.
//!
//! `table` is the one track table (invariant 3). `transport` is the bar §8
//! describes. Layouts arrange these; they never define their own.

pub mod table;
pub mod transport;

use std::time::Duration;

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
