//! What the frontend remembers between runs.
//!
//! Kept in the `[ui]` section of the shared settings file. Core owns the
//! library settings in the same file and neither side can name the other's
//! types, so each writes only its own section -- see
//! `scoria_core::config::save_section`.
//!
//! Everything here is a *proportion* or a choice, never a pixel count. A size
//! recorded on one monitor is wrong on the next, which is the same reason the
//! drawer and the columns are weights rather than measurements.

use crate::layouts::BrowserPlacement;
use crate::widgets::columns::ColumnLayout;
use scoria_core::config::Paths;
use scoria_core::error::CoreError;
use scoria_core::query::sort::SortPreset;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The section this lives under.
pub const SECTION: &str = "ui";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Settings {
    pub theme: String,
    pub placement: BrowserPlacement,
    pub columns: ColumnLayout,
    /// The configured sort (§7.3). A header click overrides it transiently and
    /// does not change this.
    pub sort: SortPreset,
    /// Drawer heights by overlay name, as fractions of the space above the
    /// transport.
    pub drawer_heights: BTreeMap<String, f32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: crate::theme::Theme::dark().name,
            placement: BrowserPlacement::default(),
            columns: ColumnLayout::default(),
            sort: SortPreset::default(),
            drawer_heights: BTreeMap::new(),
        }
    }
}

impl Settings {
    /// Reads the `[ui]` section, repairing anything a stale or hand-edited file
    /// left in a state the application cannot use.
    ///
    /// # Errors
    ///
    /// [`CoreError::Config`] if the file exists but cannot be read or parsed.
    pub fn load(paths: &Paths) -> Result<Self, CoreError> {
        let mut settings: Self = scoria_core::config::load_section(&paths.config_file(), SECTION)?;
        settings.columns = settings.columns.repaired();
        Ok(settings)
    }

    /// Writes the `[ui]` section, leaving the rest of the file alone.
    ///
    /// # Errors
    ///
    /// [`CoreError::Config`] if the file cannot be written.
    pub fn save(&self, paths: &Paths) -> Result<(), CoreError> {
        scoria_core::config::save_section(&paths.config_file(), SECTION, self)
    }

    /// The theme named in the settings, or the default if it is not one we
    /// ship.
    #[must_use]
    pub fn theme(&self) -> crate::theme::Theme {
        crate::theme::Theme::all()
            .into_iter()
            .find(|t| t.name == self.theme)
            .unwrap_or_else(crate::theme::Theme::dark)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn defaults_round_trip_through_toml() {
        let settings = Settings::default();
        let text = toml::to_string_pretty(&settings).unwrap();
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), settings);
    }

    #[test]
    fn an_empty_section_yields_defaults() {
        // A file that predates a setting must not stop the application reading
        // the rest of it.
        let settings: Settings = toml::from_str("").unwrap();
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn an_unknown_theme_falls_back_rather_than_failing() {
        let settings = Settings {
            theme: "Solarized Banana".to_owned(),
            ..Settings::default()
        };
        assert_eq!(settings.theme(), crate::theme::Theme::dark());
    }

    #[test]
    fn a_partial_section_keeps_the_defaults_for_what_is_missing() {
        let settings: Settings = toml::from_str("placement = \"left\"").unwrap();
        assert_eq!(settings.placement, BrowserPlacement::Left);
        assert_eq!(settings.columns, ColumnLayout::default());
    }
}
