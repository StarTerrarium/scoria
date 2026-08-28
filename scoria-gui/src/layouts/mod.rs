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
use iced::{Element, Fill};
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

/// Arranges the pieces.
///
/// `panes` is `None` until M8; `Off` is the only placement 1.0 opens with, and
/// the other two are arrangement changes rather than new components.
#[must_use]
pub fn arrange<'a, Message: 'a>(
    placement: BrowserPlacement,
    table: Element<'a, Message, Theme>,
    transport: Element<'a, Message, Theme>,
    search: Element<'a, Message, Theme>,
    panes: Option<Element<'a, Message, Theme>>,
    theme: &Theme,
) -> Element<'a, Message, Theme> {
    let body: Element<'a, Message, Theme> = match (placement, panes) {
        (BrowserPlacement::Off, _) | (_, None) => table,
        (BrowserPlacement::Top, Some(panes)) => {
            column![panes, table].width(Fill).height(Fill).into()
        }
        (BrowserPlacement::Left, Some(panes)) => iced::widget::row![panes, table]
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
