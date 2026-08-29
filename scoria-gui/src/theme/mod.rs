//! The semantic role table, and its conversion to iced.
//!
//! `DESIGN.md` §11. iced ships a base `Palette` of background, text, primary,
//! success, danger and warning -- built for buttons and forms. A dense table
//! needs roles it does not name: a selected row distinct from a focused one, a
//! now-playing row distinct from a selected one (a track can be both, either or
//! neither), an unavailable row, a group header, an alternating tint, and a
//! column header distinct from the body.
//!
//! So the roles are ours. They are plain data, serialisable, and 1.0 compiles
//! them in -- "hardcode the value, not the shape" (§2). User themes later
//! become "also read from a directory" rather than a refactor.
//!
//! **This module is the only place `iced::Color` appears.**
//! `scripts/check-boundaries.sh` enforces that. Everything above asks the theme
//! for a style; nothing above names a colour.

pub mod catalog;
pub mod colour;
pub mod roles;

pub use colour::Colour;
pub use roles::Roles;

use iced::widget::{button, container, rule, scrollable, slider, text, text_input};
use iced::{Background, Border, Color, border};
use serde::{Deserialize, Serialize};

/// A complete theme: a name and the role table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub roles: Roles,
}

impl Theme {
    /// The themes 1.0 ships, in the order a picker would offer them.
    #[must_use]
    pub fn all() -> Vec<Self> {
        vec![Self::dark(), Self::midnight(), Self::grey()]
    }

    /// The default. Dark, because §11 says dark-first and deliberately does not
    /// follow the system preference -- a dense music player that flips to light
    /// because the sun went down is startling rather than helpful.
    #[must_use]
    pub fn dark() -> Self {
        Self {
            name: "Scoria Dark".to_owned(),
            roles: Roles::derive(
                Colour::rgb(0x16, 0x16, 0x1b),
                Colour::rgb(0xc8, 0xcc, 0xd4),
                Colour::rgb(0x7a, 0xa2, 0xf7),
            ),
        }
    }

    #[must_use]
    pub fn midnight() -> Self {
        Self {
            name: "Midnight".to_owned(),
            roles: Roles::derive(
                Colour::rgb(0x0d, 0x11, 0x17),
                Colour::rgb(0xb6, 0xc2, 0xcf),
                Colour::rgb(0x4d, 0x9d, 0x74),
            ),
        }
    }

    #[must_use]
    pub fn grey() -> Self {
        Self {
            name: "Ash".to_owned(),
            roles: Roles::derive(
                Colour::rgb(0x1c, 0x1c, 0x1c),
                Colour::rgb(0xd0, 0xd0, 0xd0),
                Colour::rgb(0xb0, 0x8a, 0x5a),
            ),
        }
    }

    /// Builds a theme from one of iced's built-in palettes.
    ///
    /// §11: we get the built-in themes as starting points without being limited
    /// to what they name, because the roles they do not have are derived.
    #[must_use]
    pub fn from_iced(theme: &iced::Theme) -> Self {
        let palette = theme.palette();
        Self {
            name: theme.to_string(),
            roles: Roles::derive(
                from_iced_color(palette.background),
                from_iced_color(palette.text),
                from_iced_color(palette.primary),
            ),
        }
    }

    /// Whether this theme is dark, for deciding derived contrast.
    #[must_use]
    pub fn is_dark(&self) -> bool {
        self.roles.background.luma() < 0.5
    }

    // ------------------------------------------------- styles for widgets --
    //
    // Everything above this module asks for one of these rather than naming a
    // colour. That is what keeps §11's boundary real.

    /// The window background.
    #[must_use]
    pub fn window_style(&self) -> container::Style {
        container::Style {
            background: Some(Background::Color(colour(self.roles.background))),
            text_color: Some(colour(self.roles.text)),
            ..container::Style::default()
        }
    }

    /// A panel that sits above the background: the transport bar, a pane.
    #[must_use]
    pub fn surface_style(&self) -> container::Style {
        container::Style {
            background: Some(Background::Color(colour(self.roles.surface))),
            text_color: Some(colour(self.roles.text)),
            border: Border {
                color: colour(self.roles.border),
                width: 1.0,
                radius: border::radius(0),
            },
            ..container::Style::default()
        }
    }

    /// The track table's column header.
    #[must_use]
    pub fn header_style(&self) -> container::Style {
        container::Style {
            background: Some(Background::Color(colour(self.roles.header_background))),
            text_color: Some(colour(self.roles.header_text)),
            ..container::Style::default()
        }
    }

    /// A group header row (§7.3).
    #[must_use]
    pub fn group_header_style(&self) -> container::Style {
        container::Style {
            background: Some(Background::Color(colour(
                self.roles.group_header_background,
            ))),
            text_color: Some(colour(self.roles.group_header_text)),
            ..container::Style::default()
        }
    }

    /// One row of the table.
    ///
    /// A track can be selected, playing, both, either or neither -- which is
    /// exactly the case iced's palette has no vocabulary for.
    #[must_use]
    pub fn row_style(&self, row: RowState) -> container::Style {
        let background = match (row.selected, row.playing) {
            (true, _) => Some(self.roles.selected_row),
            (false, true) => Some(self.roles.now_playing_row),
            (false, false) if row.alternate => Some(self.roles.row_alternate),
            (false, false) => None,
        };

        container::Style {
            background: background.map(|c| Background::Color(colour(c))),
            text_color: Some(colour(self.row_text(row))),
            ..container::Style::default()
        }
    }

    /// The text colour a row's contents should use.
    #[must_use]
    pub fn row_text(&self, row: RowState) -> Colour {
        if !row.available {
            // §7.4: unavailable tracks are shown, dimmed and struck through,
            // never dropped.
            self.roles.unavailable_text
        } else if row.selected {
            self.roles.selected_row_text
        } else if row.playing {
            self.roles.now_playing_text
        } else {
            self.roles.text
        }
    }

    #[must_use]
    pub fn text_colour(&self) -> Colour {
        self.roles.text
    }

    #[must_use]
    pub fn muted_colour(&self) -> Colour {
        self.roles.text_muted
    }

    #[must_use]
    pub fn accent_colour(&self) -> Colour {
        self.roles.accent
    }

    /// For `text(...).style(...)`.
    #[must_use]
    pub fn text_style(&self, c: Colour) -> text::Style {
        text::Style {
            color: Some(colour(c)),
        }
    }

    /// A symbol-only transport button (§8).
    #[must_use]
    pub fn transport_button(&self, status: button::Status, active: bool) -> button::Style {
        let base = if active {
            self.roles.accent
        } else {
            self.roles.text_muted
        };
        let text_colour = match status {
            button::Status::Hovered | button::Status::Pressed => self.roles.text,
            button::Status::Active | button::Status::Disabled => base,
        };
        button::Style {
            background: match status {
                button::Status::Hovered => {
                    Some(Background::Color(colour(self.roles.row_alternate)))
                }
                _ => None,
            },
            text_color: colour(text_colour),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: border::radius(3),
            },
            ..button::Style::default()
        }
    }

    /// A row of the table, as a button so it can be clicked.
    #[must_use]
    pub fn row_button(&self, status: button::Status, row: RowState) -> button::Style {
        let mut style = button::Style {
            background: self.row_style(row).background,
            text_color: colour(self.row_text(row)),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: border::radius(0),
            },
            ..button::Style::default()
        };
        if status == button::Status::Hovered && !row.selected {
            style.background = Some(Background::Color(colour(
                self.roles.background.mix(self.roles.text, 0.08),
            )));
        }
        style
    }

    /// The volume slider (§8).
    #[must_use]
    pub fn slider_style(&self, status: slider::Status) -> slider::Style {
        let handle = match status {
            slider::Status::Active => self.roles.text_muted,
            slider::Status::Hovered | slider::Status::Dragged => self.roles.accent,
        };
        slider::Style {
            rail: slider::Rail {
                // Filled portion first, then the remainder.
                backgrounds: (
                    Background::Color(colour(self.roles.accent)),
                    Background::Color(colour(self.roles.border)),
                ),
                width: 3.0,
                border: Border {
                    radius: border::radius(2),
                    ..Border::default()
                },
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Circle { radius: 5.0 },
                background: Background::Color(colour(handle)),
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
            },
        }
    }

    /// The frame an overlay sits in (§9).
    ///
    /// Lifted rather than merely bordered. A panel the same shade as the
    /// surface behind it, on a dark theme, reads as the screen having changed
    /// rather than as something sitting above it -- so this is lighter than
    /// anything else in the window and casts a shadow, which is the strongest
    /// cue a flat interface has for depth.
    #[must_use]
    pub fn overlay_style(&self) -> container::Style {
        container::Style {
            background: Some(Background::Color(colour(
                self.roles.background.mix(self.roles.text, 0.12),
            ))),
            text_color: Some(colour(self.roles.text)),
            border: Border {
                color: colour(self.roles.background.mix(self.roles.text, 0.28)),
                width: 1.0,
                // Rounded at the top only: it rises from the transport bar and
                // is joined to it rather than floating free.
                radius: border::top(10),
            },
            shadow: iced::Shadow {
                color: Color {
                    a: 0.55,
                    ..Color::BLACK
                },
                // Cast upward: the drawer is in front of the library, not
                // resting on it.
                offset: iced::Vector::new(0.0, -6.0),
                blur_radius: 24.0,
            },
            ..container::Style::default()
        }
    }

    /// The dimming behind an overlay.
    ///
    /// Deliberately not opaque. The library has to stay *visible* underneath,
    /// or there is nothing for the overlay to be on top of and opening one
    /// reads as a navigation rather than a layer. Black rather than the theme
    /// background, because tinting a dark background with more of itself dims
    /// nothing.
    /// `progress` runs 0 to 1 as the overlay opens, so the dimming fades in
    /// rather than appearing between one frame and the next.
    #[must_use]
    pub fn scrim_style(&self, progress: f32) -> container::Style {
        container::Style {
            background: Some(Background::Color(Color {
                a: 0.45 * progress.clamp(0.0, 1.0),
                ..Color::BLACK
            })),
            ..container::Style::default()
        }
    }

    /// Where a dragged column will land.
    ///
    /// The accent colour and a solid bar: this has to be legible against the
    /// header for the length of a drag, and it is the only feedback saying the
    /// drag is doing anything at all.
    #[must_use]
    pub fn insertion_style(&self) -> container::Style {
        container::Style {
            background: Some(colour(self.roles.accent).into()),
            ..container::Style::default()
        }
    }

    /// The grabbable divider between two column headings.
    #[must_use]
    pub fn column_divider_style(&self) -> container::Style {
        container::Style {
            background: Some(Background::Color(colour(
                self.roles.background.mix(self.roles.text, 0.22),
            ))),
            ..container::Style::default()
        }
    }

    /// The drag handle at the top of a drawer.
    #[must_use]
    pub fn grip_style(&self) -> container::Style {
        container::Style {
            background: Some(Background::Color(colour(
                self.roles.background.mix(self.roles.text, 0.35),
            ))),
            border: Border {
                radius: border::radius(2),
                ..Border::default()
            },
            ..container::Style::default()
        }
    }

    /// The dismissible filter chip in the header.
    ///
    /// Reads as a thing you can act on rather than a label: bordered in the
    /// accent, and brightening on hover.
    #[must_use]
    pub fn chip_button(&self, status: button::Status) -> button::Style {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let tint = if hovered { 0.30 } else { 0.18 };
        button::Style {
            background: Some(Background::Color(colour(
                self.roles.background.mix(self.roles.accent, tint),
            ))),
            text_color: colour(self.roles.text),
            border: Border {
                color: colour(self.roles.accent),
                width: 1.0,
                radius: border::radius(3),
            },
            ..button::Style::default()
        }
    }

    #[must_use]
    pub fn rule_style(&self) -> rule::Style {
        rule::Style {
            color: colour(self.roles.border),
            radius: border::radius(0),
            fill_mode: rule::FillMode::Full,
            snap: true,
        }
    }

    #[must_use]
    pub fn search_style(&self, status: text_input::Status) -> text_input::Style {
        let border_colour = match status {
            text_input::Status::Focused { .. } => self.roles.accent,
            _ => self.roles.border,
        };
        text_input::Style {
            background: Background::Color(colour(self.roles.surface)),
            border: Border {
                color: colour(border_colour),
                width: 1.0,
                radius: border::radius(3),
            },
            icon: colour(self.roles.text_muted),
            placeholder: colour(self.roles.text_muted),
            value: colour(self.roles.text),
            selection: colour(self.roles.accent),
        }
    }

    #[must_use]
    pub fn scrollbar_style(&self, status: scrollable::Status) -> scrollable::Style {
        let hovered = matches!(
            status,
            scrollable::Status::Hovered { .. } | scrollable::Status::Dragged { .. }
        );
        let thumb = if hovered {
            self.roles.text_muted
        } else {
            self.roles.border
        };
        let rail = scrollable::Rail {
            background: Some(Background::Color(colour(self.roles.background))),
            border: Border::default(),
            scroller: scrollable::Scroller {
                background: Background::Color(colour(thumb)),
                border: Border {
                    radius: border::radius(3),
                    ..Border::default()
                },
            },
        };
        scrollable::Style {
            container: container::Style::default(),
            vertical_rail: rail,
            horizontal_rail: rail,
            gap: None,
            // The middle-click auto-scroll overlay. Rarely seen, but it has
            // to come from the theme like everything else.
            auto_scroll: scrollable::AutoScroll {
                background: Background::Color(colour(self.roles.surface)),
                border: Border {
                    color: colour(self.roles.border),
                    width: 1.0,
                    radius: border::radius(12),
                },
                shadow: iced::Shadow::default(),
                icon: colour(self.roles.text_muted),
            },
        }
    }
}

/// What a table row is, for choosing its colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RowState {
    pub selected: bool,
    pub playing: bool,
    pub alternate: bool,
    pub available: bool,
}

/// Our colour into iced's. The boundary, in one function.
#[must_use]
pub fn colour(c: Colour) -> Color {
    Color::from_rgb8(c.r, c.g, c.b)
}

fn from_iced_color(c: Color) -> Colour {
    let to_byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Colour::rgb(to_byte(c.r), to_byte(c.g), to_byte(c.b))
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl std::fmt::Display for Theme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// Lets iced use our theme directly, rather than us shadowing one of its own.
impl iced::theme::Base for Theme {
    fn default(_preference: iced::theme::Mode) -> Self {
        // §11: deliberately ignores the system preference. A dense music player
        // that flips to light because the sun went down is startling.
        Self::dark()
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn mode(&self) -> iced::theme::Mode {
        if self.is_dark() {
            iced::theme::Mode::Dark
        } else {
            iced::theme::Mode::Light
        }
    }

    fn base(&self) -> iced::theme::Style {
        iced::theme::Style {
            background_color: colour(self.roles.background),
            text_color: colour(self.roles.text),
        }
    }

    fn palette(&self) -> Option<iced::theme::Palette> {
        Some(iced::theme::Palette {
            background: colour(self.roles.background),
            text: colour(self.roles.text),
            primary: colour(self.roles.accent),
            success: colour(self.roles.accent),
            warning: colour(self.roles.warning),
            danger: colour(self.roles.danger),
        })
    }
}
