//! Turning core types into terminal output.
//!
//! Display-width aware, because the library this is aimed at is full of CJK
//! titles and a column laid out by `char` count is visibly ragged.

use scoria_core::model::Track;
use std::time::Duration;

/// Approximate rendered width of a string in terminal cells.
///
/// CJK ideographs, kana and full-width forms occupy two cells; combining marks
/// occupy none. This is not a full implementation of the Unicode width tables,
/// but it covers what a music library actually contains and is the difference
/// between aligned columns and a mess.
#[must_use]
pub fn width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

fn char_width(c: char) -> usize {
    match u32::from(c) {
        // Combining marks.
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F => 0,
        // Hangul, CJK, kana, full-width forms.
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

/// Truncates to `cells` display columns, adding an ellipsis if it had to cut.
#[must_use]
pub fn fit(s: &str, cells: usize) -> String {
    if width(s) <= cells {
        return s.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = char_width(c);
        if used + w > cells.saturating_sub(1) {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Pads to `cells` display columns.
#[must_use]
pub fn pad(s: &str, cells: usize) -> String {
    let fitted = fit(s, cells);
    let padding = cells.saturating_sub(width(&fitted));
    format!("{fitted}{}", " ".repeat(padding))
}

/// `3:47`, or `1:02:11` for something long.
#[must_use]
pub fn duration(d: Duration) -> String {
    let total = d.as_secs();
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// One row of the track table, in the same column order §7.3 gives as default.
#[must_use]
pub fn track_row(track: &Track) -> String {
    let number = track
        .track_number
        .map_or_else(|| "  ".to_owned(), |n| format!("{n:2}"));
    let year = track
        .date
        .map_or_else(|| "    ".to_owned(), |d| format!("{:4}", d.year()));

    // §7.4: unavailable tracks are shown struck through, never dropped. The
    // terminal equivalent of the strikethrough the table will use.
    let marker = if track.available { " " } else { "!" };

    format!(
        "{marker} {number}  {}  {}  {}  {}  {year}",
        pad(&track.title, 34),
        pad(&track.album_artist, 20),
        pad(&track.album, 24),
        pad(&duration(track.duration), 7),
    )
}

/// The header the rows line up under.
#[must_use]
pub fn track_header() -> String {
    format!(
        "  {}  {}  {}  {}  {}  {}",
        "  ",
        pad("TITLE", 34),
        pad("ALBUM ARTIST", 20),
        pad("ALBUM", 24),
        pad("TIME", 7),
        "YEAR"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_one_cell_per_character() {
        assert_eq!(width("Metallica"), 9);
    }

    #[test]
    fn cjk_is_two_cells_per_character() {
        // Without this, every row containing a Japanese title would be ragged.
        assert_eq!(width("花冷え"), 6);
        assert_eq!(width("乙女改革"), 8);
    }

    #[test]
    fn padding_lines_up_across_scripts() {
        assert_eq!(width(&pad("花冷え。", 10)), 10);
        assert_eq!(width(&pad("Ghost", 10)), 10);
    }

    #[test]
    fn fitting_never_exceeds_the_budget() {
        for s in ["Metallica", "花冷え。乙女改革", "a"] {
            for cells in 1..12 {
                assert!(width(&fit(s, cells)) <= cells, "{s:?} at {cells}");
            }
        }
    }

    #[test]
    fn durations_read_the_way_a_transport_shows_them() {
        assert_eq!(duration(Duration::from_secs(227)), "3:47");
        assert_eq!(duration(Duration::from_secs(3731)), "1:02:11");
        assert_eq!(duration(Duration::from_secs(5)), "0:05");
    }
}
