//! Colours, in our vocabulary rather than iced's.
//!
//! `DESIGN.md` §11: themes are expressed in our own terms and the iced layer
//! converts at the boundary. Scattering `iced::Color` through view code would
//! force a parallel theme system the moment the TUI arrives, and it would drift
//! immediately.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An opaque sRGB colour.
///
/// Serialised as `#rrggbb`, because a theme file is meant to be edited by hand
/// and `{ r = 0.086, g = 0.086, b = 0.106 }` is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colour {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Colour {
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Parses `#rrggbb` or `rrggbb`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix('#').unwrap_or(text);
        if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let component = |range: std::ops::Range<usize>| -> Option<u8> {
            u8::from_str_radix(hex.get(range)?, 16).ok()
        };
        Some(Self {
            r: component(0..2)?,
            g: component(2..4)?,
            b: component(4..6)?,
        })
    }

    #[must_use]
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Blends towards `other`. `amount` is clamped to 0..=1.
    ///
    /// Used to derive roles the iced palette does not name -- an alternating row
    /// tint is the background nudged a few percent towards the text colour, not
    /// a colour anyone should have to choose.
    #[must_use]
    pub fn mix(self, other: Self, amount: f32) -> Self {
        let t = amount.clamp(0.0, 1.0);
        let blend = |a: u8, b: u8| {
            let mixed = f32::from(a).mul_add(1.0 - t, f32::from(b) * t);
            mixed.round().clamp(0.0, 255.0) as u8
        };
        Self {
            r: blend(self.r, other.r),
            g: blend(self.g, other.g),
            b: blend(self.b, other.b),
        }
    }

    /// Perceived lightness, 0.0 to 1.0.
    ///
    /// Rec. 601 luma. Good enough to decide whether a theme is dark, which is
    /// the only question asked of it.
    #[must_use]
    pub fn luma(self) -> f32 {
        (0.299 * f32::from(self.r) + 0.587 * f32::from(self.g) + 0.114 * f32::from(self.b)) / 255.0
    }
}

impl Serialize for Colour {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Colour {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).ok_or_else(|| {
            serde::de::Error::custom(format!("not a colour: {text:?} (expected #rrggbb)"))
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn round_trips_through_hex() {
        let c = Colour::rgb(0x16, 0x16, 0x1b);
        assert_eq!(c.to_hex(), "#16161b");
        assert_eq!(Colour::parse("#16161b"), Some(c));
        assert_eq!(Colour::parse("16161b"), Some(c));
    }

    #[test]
    fn rejects_nonsense_rather_than_guessing() {
        for bad in ["", "#fff", "#gggggg", "#1234567", "blue"] {
            assert_eq!(Colour::parse(bad), None, "should have rejected {bad:?}");
        }
    }

    #[test]
    fn mixing_reaches_both_ends() {
        let a = Colour::rgb(0, 0, 0);
        let b = Colour::rgb(255, 255, 255);
        assert_eq!(a.mix(b, 0.0), a);
        assert_eq!(a.mix(b, 1.0), b);
        assert_eq!(a.mix(b, 0.5), Colour::rgb(128, 128, 128));
    }

    #[test]
    fn mixing_clamps_rather_than_wrapping() {
        let a = Colour::rgb(10, 10, 10);
        let b = Colour::rgb(200, 200, 200);
        assert_eq!(a.mix(b, -5.0), a);
        assert_eq!(a.mix(b, 5.0), b);
    }

    #[test]
    fn luma_separates_dark_from_light() {
        assert!(Colour::rgb(0x16, 0x16, 0x1b).luma() < 0.5);
        assert!(Colour::rgb(0xf5, 0xf5, 0xf5).luma() > 0.5);
    }

    #[test]
    fn survives_a_serde_round_trip() {
        let c = Colour::rgb(0x7a, 0xa2, 0xf7);
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, "\"#7aa2f7\"");
        assert_eq!(serde_json::from_str::<Colour>(&json).unwrap(), c);
    }
}
