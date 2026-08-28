//! Arrangement, and nothing else.
//!
//! `DESIGN.md` §7.1 and `CLAUDE.md` invariant 3: the three layouts differ on
//! exactly one variable -- where the browser sits. The table, columns,
//! transport bar, search field, selection model and keybinds are identical
//! across all three, and a keystroke cycles the value.
//!
//! **Nothing in this module defines a widget.** If a layout needed its own
//! table it would not be a layout, and adding one would be the violation
//! `scripts/check-boundaries.sh` checks for.

use crate::theme::Theme;
use iced::widget::{column, container};
use iced::{Element, Fill, Length};
use serde::{Deserialize, Serialize};

/// Where the filter panes sit (§7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BrowserPlacement {
    /// A single continuous table with album group headers, navigated by search
    /// and type-to-jump.
    #[default]
    Off,
    /// Panes in a horizontal strip with now-playing artwork. Full table width,
    /// about ten artist rows.
    Top,
    /// Panes full height. Many artists visible, at the cost of roughly 250px of
    /// table width.
    Left,
}

impl BrowserPlacement {
    /// The next placement, for the keystroke that cycles them.
    #[must_use]
    pub const fn cycle(self) -> Self {
        match self {
            Self::Off => Self::Left,
            Self::Left => Self::Top,
            Self::Top => Self::Off,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Browser off",
            Self::Top => "Browser on top",
            Self::Left => "Browser on the left",
        }
    }
}

/// The browser: two filter panes and the now-playing artwork.
///
/// Passed as pieces rather than as a finished element, because arranging them
/// is exactly what differs between placements -- stacked in a column on the
/// left, side by side in a strip on top. Handing over an assembled browser
/// would move that decision out of this module, which is the only place it
/// belongs.
pub struct Browser<'a, Message> {
    pub artists: Element<'a, Message, Theme>,
    pub albums: Element<'a, Message, Theme>,
    pub artwork: Element<'a, Message, Theme>,
}

/// How much table width the `Left` placement costs (§7.1).
const LEFT_WIDTH: f32 = 250.0;
/// Tall enough for the artwork, which is what raises the strip from four
/// artist rows to about ten and makes `Top` viable at all (§7.1).
const TOP_HEIGHT: f32 = 220.0;
/// Square, and the reason the strip is as tall as it is.
const ARTWORK_SIZE: f32 = TOP_HEIGHT - 12.0;

// §7.1: the artwork is not decoration filling leftover space -- it justifies a
// strip height the panes need anyway. Checked at build time rather than in a
// test, because both values are constants and a test could only ever agree with
// the compiler.
const _: () = assert!(
    ARTWORK_SIZE <= TOP_HEIGHT,
    "the artwork cannot exceed the strip"
);
const _: () = assert!(
    ARTWORK_SIZE > TOP_HEIGHT * 0.9,
    "the strip is no longer sized around the artwork"
);

/// Arranges the pieces.
///
/// The three placements differ here and nowhere else. Every component is the
/// same object in all three.
#[must_use]
pub fn arrange<'a, Message: 'a>(
    placement: BrowserPlacement,
    table: Element<'a, Message, Theme>,
    transport: Element<'a, Message, Theme>,
    search: Element<'a, Message, Theme>,
    browser: Option<Browser<'a, Message>>,
    theme: &Theme,
) -> Element<'a, Message, Theme> {
    let body: Element<'a, Message, Theme> = match (placement, browser) {
        (BrowserPlacement::Off, _) | (_, None) => table,

        // §7.1: panes full height, many artists visible, at the cost of about
        // 250px of table width.
        (BrowserPlacement::Left, Some(browser)) => iced::widget::row![
            column![browser.artists, rule(theme), browser.albums]
                .width(Length::Fixed(LEFT_WIDTH))
                .height(Fill),
            vertical_rule(theme),
            table,
        ]
        .width(Fill)
        .height(Fill)
        .into(),

        // §7.1: a horizontal strip with the now-playing artwork. The artwork is
        // not decoration filling leftover space -- it justifies a strip height
        // the panes need anyway, raising them from four visible artist rows to
        // about ten.
        (BrowserPlacement::Top, Some(browser)) => column![
            iced::widget::row![
                container(browser.artwork)
                    .width(Length::Fixed(ARTWORK_SIZE))
                    .height(Length::Fixed(ARTWORK_SIZE)),
                vertical_rule(theme),
                browser.artists,
                vertical_rule(theme),
                browser.albums,
            ]
            .height(Length::Fixed(TOP_HEIGHT))
            .width(Fill),
            rule(theme),
            table,
        ]
        .width(Fill)
        .height(Fill)
        .into(),
    };

    let style = theme.window_style();
    container(column![search, body, transport].width(Fill).height(Fill))
        .width(Fill)
        .height(Fill)
        .style(move |_t: &Theme| style)
        .into()
}

fn rule<'a, Message: 'a>(theme: &Theme) -> Element<'a, Message, Theme> {
    let style = theme.rule_style();
    iced::widget::rule::horizontal(1)
        .style(move |_t: &Theme| style)
        .into()
}

fn vertical_rule<'a, Message: 'a>(theme: &Theme) -> Element<'a, Message, Theme> {
    let style = theme.rule_style();
    iced::widget::rule::vertical(1)
        .style(move |_t: &Theme| style)
        .into()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn cycling_visits_every_placement_and_returns() {
        // §7.1: one keystroke cycles the value. Three presses should return to
        // where it started, or the keystroke is a trap.
        let mut placement = BrowserPlacement::Off;
        let mut seen = vec![placement];
        for _ in 0..2 {
            placement = placement.cycle();
            seen.push(placement);
        }
        assert_eq!(placement.cycle(), BrowserPlacement::Off);
        assert_eq!(seen.len(), 3);

        let unique: std::collections::HashSet<_> = seen.into_iter().collect();
        assert_eq!(unique.len(), 3, "cycling skipped a placement");
    }

    #[test]
    fn the_default_is_the_one_1_0_opens_with() {
        assert_eq!(BrowserPlacement::default(), BrowserPlacement::Off);
    }

    #[test]
    fn the_left_placement_costs_the_width_the_design_budgets() {
        // §7.1 puts a number on it: roughly 250px of table width. Worth
        // asserting, because the whole character of the placement is that it
        // trades table width for visible artists.
        assert!((LEFT_WIDTH - 250.0).abs() < f32::EPSILON);
    }

    #[test]
    fn the_top_strip_is_tall_enough_for_about_ten_artist_rows() {
        // §7.1: the claustrophobia of `Top` comes from fewer visible artist
        // rows, not from table width, and sizing the strip to fit the artwork
        // is what raises it from four rows to about ten. If the strip ever
        // shrinks below that, the placement stops being viable.
        let heading_and_all = crate::widgets::table::ROW_HEIGHT * 2.0;
        let rows = (TOP_HEIGHT - heading_and_all) / crate::widgets::table::ROW_HEIGHT;
        assert!(rows >= 8.0, "only {rows} artist rows fit");
    }

    #[test]
    fn a_placement_survives_a_serde_round_trip() {
        for placement in [
            BrowserPlacement::Off,
            BrowserPlacement::Top,
            BrowserPlacement::Left,
        ] {
            let text = serde_json::to_string(&placement).unwrap();
            assert_eq!(
                serde_json::from_str::<BrowserPlacement>(&text).unwrap(),
                placement
            );
        }
    }
}
