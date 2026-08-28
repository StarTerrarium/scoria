//! The roles a dense table needs.
//!
//! `DESIGN.md` §11. Each is stated rather than derived at the point of use, so
//! a theme file can override any single one where the derivation looks wrong --
//! which is the whole reason for having a role table instead of a palette.

use super::colour::Colour;
use serde::{Deserialize, Serialize};

/// Every colour the application can ask for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Roles {
    /// The window behind everything.
    pub background: Colour,
    /// Panels that sit above it: the transport bar, the filter panes.
    pub surface: Colour,
    pub border: Colour,

    pub text: Colour,
    /// Secondary text: `artist — album` under a title, counts in a pane.
    pub text_muted: Colour,

    /// A selected row, which is not the same as a focused one and not the same
    /// as a playing one.
    pub selected_row: Colour,
    pub selected_row_text: Colour,

    /// The row that is playing. A track can be selected, playing, both, either
    /// or neither, so these cannot be the same colour.
    pub now_playing_row: Colour,
    pub now_playing_text: Colour,

    /// A track whose file no longer resolves (§7.4). Shown, dimmed, struck
    /// through -- never dropped.
    pub unavailable_text: Colour,

    pub group_header_background: Colour,
    pub group_header_text: Colour,

    /// The faint tint on every other row that makes a wide table readable.
    pub row_alternate: Colour,

    pub header_background: Colour,
    pub header_text: Colour,

    pub accent: Colour,
    pub warning: Colour,
    pub danger: Colour,
}

impl Roles {
    /// Derives the whole table from three colours.
    ///
    /// This is what lets §11's promise hold: any iced palette becomes a usable
    /// starting point, and the roles it has no name for are computed rather
    /// than guessed at by hand. A theme that wants a different answer overrides
    /// the single role rather than redefining everything.
    ///
    /// The derivations are all "nudge the background towards the text", scaled
    /// by how strong the role should be. That keeps contrast sensible whether
    /// the base is dark or light.
    #[must_use]
    pub fn derive(background: Colour, text: Colour, accent: Colour) -> Self {
        let toward_text = |amount: f32| background.mix(text, amount);

        Self {
            background,
            surface: toward_text(0.05),
            border: toward_text(0.14),

            text,
            text_muted: background.mix(text, 0.62),

            // Selection is the accent, muted enough that white text still reads
            // on it. It is stronger than the now-playing tint because selection
            // is what the user is doing right now.
            selected_row: background.mix(accent, 0.55),
            selected_row_text: Colour::rgb(0xff, 0xff, 0xff),

            // Now-playing is quieter and ambient: it marks a fact rather than
            // an action, and it has to remain distinguishable underneath a
            // selection highlight.
            now_playing_row: background.mix(accent, 0.16),
            now_playing_text: accent,

            unavailable_text: background.mix(text, 0.38),

            group_header_background: toward_text(0.09),
            group_header_text: accent,

            // Barely there on purpose. An alternating tint that announces
            // itself is worse than none.
            row_alternate: toward_text(0.04),

            header_background: toward_text(0.07),
            header_text: background.mix(text, 0.70),

            accent,
            warning: Colour::rgb(0xd8, 0xa6, 0x57),
            danger: Colour::rgb(0xd6, 0x5d, 0x5d),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::theme::Theme;

    fn dark() -> Roles {
        Roles::derive(
            Colour::rgb(0x16, 0x16, 0x1b),
            Colour::rgb(0xc8, 0xcc, 0xd4),
            Colour::rgb(0x7a, 0xa2, 0xf7),
        )
    }

    #[test]
    fn selected_and_playing_are_distinguishable() {
        // §11's central point: a track can be both, either or neither, so these
        // three states must all look different from each other.
        let r = dark();
        assert_ne!(r.selected_row, r.now_playing_row);
        assert_ne!(r.selected_row, r.background);
        assert_ne!(r.now_playing_row, r.background);
    }

    #[test]
    fn selection_reads_more_strongly_than_now_playing() {
        // Selection is what the user is doing; now-playing is ambient. If the
        // ambient one shouted louder, the table would be hard to work in.
        let r = dark();
        let distance = |c: Colour| (c.luma() - r.background.luma()).abs();
        assert!(distance(r.selected_row) > distance(r.now_playing_row));
    }

    #[test]
    fn unavailable_text_is_dimmer_than_ordinary_text() {
        let r = dark();
        assert!(
            (r.unavailable_text.luma() - r.background.luma()).abs()
                < (r.text.luma() - r.background.luma()).abs()
        );
    }

    #[test]
    fn the_alternating_tint_is_subtle() {
        // Visible enough to guide the eye across a wide row, quiet enough not
        // to look like a second selection colour.
        let r = dark();
        let delta = (r.row_alternate.luma() - r.background.luma()).abs();
        assert!(delta > 0.0, "an invisible tint is pointless");
        assert!(delta < 0.08, "a loud tint reads as a highlight: {delta}");
    }

    #[test]
    fn derivation_works_from_a_light_base_too() {
        // The 22 built-in iced themes include light ones, and §11 says they are
        // usable starting points.
        let light = Roles::derive(
            Colour::rgb(0xfa, 0xfa, 0xfa),
            Colour::rgb(0x20, 0x20, 0x20),
            Colour::rgb(0x30, 0x60, 0xc0),
        );
        assert_ne!(light.surface, light.background);
        assert_ne!(light.text_muted, light.text);
        assert!(
            light.row_alternate.luma() < light.background.luma(),
            "should darken, not lighten"
        );
    }

    #[test]
    fn every_shipped_theme_is_dark() {
        // §11: 1.0 ships a curated handful, dark-first, defaulting to dark.
        for theme in Theme::all() {
            assert!(theme.is_dark(), "{} should be dark", theme.name);
        }
    }

    #[test]
    fn the_default_theme_ignores_the_system_preference() {
        // A dense player that flips to light because the sun went down is
        // startling rather than helpful.
        use iced::theme::{Base, Mode};
        assert!(<Theme as Base>::default(Mode::Light).is_dark());
        assert!(<Theme as Base>::default(Mode::Dark).is_dark());
    }

    #[test]
    fn a_theme_survives_a_serde_round_trip() {
        // §11: themes are serialisable data even while compiled in, so user
        // themes later are "also read from a directory" rather than a refactor.
        let theme = Theme::dark();
        let text = toml::to_string_pretty(&theme).unwrap();
        assert!(
            text.contains("#16161b"),
            "colours should be hex in the file:\n{text}"
        );
        assert_eq!(toml::from_str::<Theme>(&text).unwrap(), theme);
    }

    #[test]
    fn a_built_in_iced_palette_becomes_a_usable_theme() {
        let theme = Theme::from_iced(&iced::Theme::Nord);
        assert_ne!(theme.roles.selected_row, theme.roles.now_playing_row);
        assert_ne!(theme.roles.surface, theme.roles.background);
    }
}
