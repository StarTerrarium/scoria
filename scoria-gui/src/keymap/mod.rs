//! Key bindings as data.
//!
//! `DESIGN.md` §10: **the keymap is a data structure, not match arms in view
//! code.** Two keymaps resolving to one shared action enum -- the same enum the
//! TUI will drive. Deferring the vim *keymap* is cheap; deferring this *design*
//! would not be, because match arms scattered through views cannot be swapped
//! for a second scheme without touching every one of them.
//!
//! §10 also records the one collision worth resolving up front: type-to-jump
//! wants bare letters, and vim normal mode wants bare letters. They cannot both
//! own unmodified keypresses, so in vim mode searching goes through `/`. That
//! is a property of a keymap, which is why keymaps are values.

pub mod action;

pub use action::Action;

use iced::keyboard::{Key, Modifiers, key::Named};
use serde::{Deserialize, Serialize};

/// A key, in a form that can be written down and compared.
///
/// Deliberately not `iced::keyboard::Key`: a keymap is meant to be
/// serialisable, and it has to mean the same thing to the TUI, which has never
/// heard of iced.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Trigger {
    /// A printable character, matched case-insensitively.
    Char(char),
    Space,
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Function(u8),
}

impl Trigger {
    /// Translates an iced key press. `None` for keys no binding can name.
    #[must_use]
    pub fn from_iced(key: &Key) -> Option<Self> {
        match key {
            Key::Character(c) => {
                let ch = c.chars().next()?;
                Some(Self::Char(ch.to_ascii_lowercase()))
            }
            Key::Named(named) => Some(match named {
                Named::Space => Self::Space,
                Named::Enter => Self::Enter,
                Named::Escape => Self::Escape,
                Named::Tab => Self::Tab,
                Named::Backspace => Self::Backspace,
                Named::Delete => Self::Delete,
                Named::ArrowUp => Self::Up,
                Named::ArrowDown => Self::Down,
                Named::ArrowLeft => Self::Left,
                Named::ArrowRight => Self::Right,
                Named::Home => Self::Home,
                Named::End => Self::End,
                Named::PageUp => Self::PageUp,
                Named::PageDown => Self::PageDown,
                Named::F1 => Self::Function(1),
                Named::F2 => Self::Function(2),
                Named::F3 => Self::Function(3),
                Named::F5 => Self::Function(5),
                _ => return None,
            }),
            Key::Unidentified => None,
        }
    }
}

/// Which modifiers a binding requires.
///
/// A plain struct rather than iced's bitflags, for the same reason [`Trigger`]
/// is ours: this has to survive being written to a file and read by a frontend
/// that does not use iced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Mods {
    pub const NONE: Self = Self {
        ctrl: false,
        shift: false,
        alt: false,
    };
    pub const CTRL: Self = Self {
        ctrl: true,
        shift: false,
        alt: false,
    };
    pub const SHIFT: Self = Self {
        ctrl: false,
        shift: true,
        alt: false,
    };

    #[must_use]
    pub fn from_iced(modifiers: Modifiers) -> Self {
        Self {
            ctrl: modifiers.control(),
            shift: modifiers.shift(),
            alt: modifiers.alt(),
        }
    }
}

/// One binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub trigger: Trigger,
    #[serde(default)]
    pub mods: Mods,
    pub action: Action,
}

impl Binding {
    #[must_use]
    pub const fn new(trigger: Trigger, mods: Mods, action: Action) -> Self {
        Self {
            trigger,
            mods,
            action,
        }
    }
}

/// An ordered list of bindings.
///
/// Ordered because the first match wins, which is what lets a more specific
/// binding shadow a general one without the lookup needing to know about
/// specificity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keymap {
    pub name: String,
    pub bindings: Vec<Binding>,
}

impl Keymap {
    /// Resolves a key press.
    #[must_use]
    pub fn resolve(&self, trigger: &Trigger, mods: Mods) -> Option<Action> {
        self.bindings
            .iter()
            .find(|b| b.trigger == *trigger && b.mods == mods)
            .map(|b| b.action)
    }

    /// Resolves straight from an iced key press.
    #[must_use]
    pub fn resolve_iced(&self, key: &Key, modifiers: Modifiers) -> Option<Action> {
        self.resolve(&Trigger::from_iced(key)?, Mods::from_iced(modifiers))
    }

    /// The bindings for one action, for the `?` cheatsheet (§8).
    #[must_use]
    pub fn bindings_for(&self, action: Action) -> Vec<&Binding> {
        self.bindings
            .iter()
            .filter(|b| b.action == action)
            .collect()
    }

    /// Arrows and standard shortcuts. What 1.0 ships (§10).
    #[must_use]
    pub fn standard() -> Self {
        use Action as A;
        use Trigger as T;

        let bindings = vec![
            // Movement.
            Binding::new(T::Up, Mods::NONE, A::MoveUp),
            Binding::new(T::Down, Mods::NONE, A::MoveDown),
            Binding::new(T::PageUp, Mods::NONE, A::PageUp),
            Binding::new(T::PageDown, Mods::NONE, A::PageDown),
            Binding::new(T::Home, Mods::NONE, A::MoveToStart),
            Binding::new(T::End, Mods::NONE, A::MoveToEnd),
            Binding::new(T::Up, Mods::SHIFT, A::ExtendUp),
            Binding::new(T::Down, Mods::SHIFT, A::ExtendDown),
            // Doing something with the selection.
            Binding::new(T::Enter, Mods::NONE, A::PlaySelection),
            // Transport. Space is the one binding everyone already knows.
            Binding::new(T::Space, Mods::NONE, A::PlayPause),
            Binding::new(T::Right, Mods::CTRL, A::Next),
            Binding::new(T::Left, Mods::CTRL, A::Previous),
            Binding::new(T::Char('s'), Mods::CTRL, A::ToggleShuffle),
            Binding::new(T::Char('r'), Mods::CTRL, A::CycleRepeat),
            // Getting around.
            Binding::new(T::Char('f'), Mods::CTRL, A::FocusSearch),
            Binding::new(T::Char('l'), Mods::CTRL, A::CycleLayout),
            Binding::new(T::Char('j'), Mods::CTRL, A::JumpToPlaying),
            Binding::new(T::Char('q'), Mods::CTRL, A::ToggleQueue),
            // Overlays. §9: escape closes, and the key that opened it toggles.
            Binding::new(T::Escape, Mods::NONE, A::Dismiss),
            Binding::new(T::Char('?'), Mods::NONE, A::ToggleHelp),
            Binding::new(T::Char('/'), Mods::NONE, A::FocusSearch),
            Binding::new(T::Function(1), Mods::NONE, A::ToggleHelp),
            // F5, because that is where every application puts refresh.
            Binding::new(T::Function(5), Mods::NONE, A::Rescan),
        ];

        Self {
            name: "Standard".to_owned(),
            bindings,
        }
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Self::standard()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn resolves_a_plain_key() {
        let keymap = Keymap::standard();
        assert_eq!(
            keymap.resolve(&Trigger::Space, Mods::NONE),
            Some(Action::PlayPause)
        );
        assert_eq!(
            keymap.resolve(&Trigger::Down, Mods::NONE),
            Some(Action::MoveDown)
        );
    }

    #[test]
    fn modifiers_select_different_actions_for_one_key() {
        let keymap = Keymap::standard();
        assert_eq!(
            keymap.resolve(&Trigger::Down, Mods::NONE),
            Some(Action::MoveDown)
        );
        assert_eq!(
            keymap.resolve(&Trigger::Down, Mods::SHIFT),
            Some(Action::ExtendDown)
        );
        assert_eq!(keymap.resolve(&Trigger::Down, Mods::CTRL), None);
    }

    #[test]
    fn an_unbound_key_resolves_to_nothing() {
        assert_eq!(
            Keymap::standard().resolve(&Trigger::Char('z'), Mods::NONE),
            None
        );
    }

    #[test]
    fn characters_are_matched_case_insensitively() {
        // Shift+F and Ctrl+F should not be different bindings by accident.
        let upper = Trigger::from_iced(&Key::Character("F".into())).unwrap();
        assert_eq!(upper, Trigger::Char('f'));
    }

    #[test]
    fn unmapped_keys_are_ignored_rather_than_guessed() {
        assert_eq!(Trigger::from_iced(&Key::Named(Named::PrintScreen)), None);
        assert_eq!(Trigger::from_iced(&Key::Unidentified), None);
    }

    #[test]
    fn the_first_matching_binding_wins() {
        // Ordering is the whole conflict-resolution rule, so a later binding
        // must not silently take over from an earlier one.
        let keymap = Keymap {
            name: "test".to_owned(),
            bindings: vec![
                Binding::new(Trigger::Char('x'), Mods::NONE, Action::Next),
                Binding::new(Trigger::Char('x'), Mods::NONE, Action::Previous),
            ],
        };
        assert_eq!(
            keymap.resolve(&Trigger::Char('x'), Mods::NONE),
            Some(Action::Next)
        );
    }

    #[test]
    fn no_two_standard_bindings_collide() {
        // A shadowed binding is a binding that silently does nothing.
        let keymap = Keymap::standard();
        let mut seen = std::collections::HashSet::new();
        for binding in &keymap.bindings {
            assert!(
                seen.insert((binding.trigger.clone(), binding.mods)),
                "{:?}+{:?} is bound twice",
                binding.mods,
                binding.trigger
            );
        }
    }

    #[test]
    fn every_action_the_ui_needs_has_a_binding() {
        // An action with no way to reach it is dead weight; a keyboard-driven
        // application should not have any.
        let keymap = Keymap::standard();
        for action in Action::all() {
            assert!(
                !keymap.bindings_for(action).is_empty(),
                "{action:?} cannot be reached from the keyboard"
            );
        }
    }

    #[test]
    fn there_is_a_way_out_of_a_filter_from_the_keyboard() {
        // Filters survive a layout change and `Off` shows no panes, so without
        // this there is a state that can be entered and not left.
        assert_eq!(
            Keymap::standard().resolve(&Trigger::Escape, Mods::NONE),
            Some(Action::Dismiss)
        );
    }

    #[test]
    fn a_keymap_survives_a_serde_round_trip() {
        // §10: the keymap is data, so a vim keymap later is a file rather than
        // a rewrite.
        let keymap = Keymap::standard();
        let text = toml::to_string_pretty(&keymap).unwrap();
        assert_eq!(toml::from_str::<Keymap>(&text).unwrap(), keymap);
    }

    #[test]
    fn a_second_keymap_needs_no_code_changes() {
        // The point of §10's design, demonstrated: a vim-style map is a value.
        // The bare-letter collision it describes is resolved by binding `/` to
        // search, exactly as the design says.
        let vim = Keymap {
            name: "Vim".to_owned(),
            bindings: vec![
                Binding::new(Trigger::Char('j'), Mods::NONE, Action::MoveDown),
                Binding::new(Trigger::Char('k'), Mods::NONE, Action::MoveUp),
                Binding::new(Trigger::Char('/'), Mods::NONE, Action::FocusSearch),
            ],
        };
        assert_eq!(
            vim.resolve(&Trigger::Char('j'), Mods::NONE),
            Some(Action::MoveDown)
        );
        assert_eq!(
            vim.resolve(&Trigger::Char('/'), Mods::NONE),
            Some(Action::FocusSearch)
        );
    }
}
