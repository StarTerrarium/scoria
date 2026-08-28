//! Making our theme the theme iced uses.
//!
//! iced styles a widget by asking its theme for a `Style`, through a per-widget
//! `Catalog`. Implementing those here is what lets [`Theme`] be the application's
//! actual theme rather than something carried alongside iced's and applied by
//! hand at every call site.
//!
//! Each follows the same shape iced's own theme uses: the class is a boxed
//! function, so a widget can either take the default for its kind or be handed
//! a closure. The point is that the closure receives *our* theme, so view code
//! asks for `theme.row_style(...)` and never names a colour.

use super::Theme;
use iced::widget::{button, container, rule, scrollable, slider, text, text_input};

/// Implements a `Catalog` whose class is a boxed style function.
macro_rules! catalog {
    ($module:ident, $default:expr) => {
        impl $module::Catalog for Theme {
            type Class<'a> = Box<dyn Fn(&Self) -> $module::Style + 'a>;

            fn default<'a>() -> Self::Class<'a> {
                Box::new($default)
            }

            fn style(&self, class: &Self::Class<'_>) -> $module::Style {
                class(self)
            }
        }
    };
}

/// The same, for widgets whose style also depends on interaction state.
macro_rules! catalog_with_status {
    ($module:ident, $status:ty, $default:expr) => {
        impl $module::Catalog for Theme {
            type Class<'a> = Box<dyn Fn(&Self, $status) -> $module::Style + 'a>;

            fn default<'a>() -> Self::Class<'a> {
                Box::new($default)
            }

            fn style(&self, class: &Self::Class<'_>, status: $status) -> $module::Style {
                class(self, status)
            }
        }
    };
}

catalog!(container, |_theme| container::Style::default());
catalog!(rule, Theme::rule_style);

catalog_with_status!(button, button::Status, |theme: &Theme, status| {
    theme.transport_button(status, false)
});
catalog_with_status!(scrollable, scrollable::Status, Theme::scrollbar_style);
catalog_with_status!(text_input, text_input::Status, Theme::search_style);
catalog_with_status!(slider, slider::Status, Theme::slider_style);

/// Text is the exception: its class carries no status, and the default has to
/// inherit the surrounding container's colour rather than forcing one, or every
/// label inside a selected row would fight the row's own text colour.
impl text::Catalog for Theme {
    type Class<'a> = Box<dyn Fn(&Self) -> text::Style + 'a>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(|_theme| text::Style { color: None })
    }

    fn style(&self, class: &Self::Class<'_>) -> text::Style {
        class(self)
    }
}
