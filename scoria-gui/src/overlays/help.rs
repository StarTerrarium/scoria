//! The `?` cheatsheet.
//!
//! `DESIGN.md` §8 and §10. **Generated from the keymap rather than written
//! out**, which is the only version that cannot drift: a binding that changes
//! changes this, and a binding that is removed disappears from it. A
//! hand-written list of shortcuts is wrong the first time anyone edits the
//! keymap and stays wrong until someone notices.
//!
//! Keyboard control is the power-user path -- everything here can be done with
//! the mouse, and §8 requires it -- so this is a shortcut to something already
//! reachable, never the only way in. The note at the top says so, because a
//! screen full of key names implies the opposite.

use crate::keymap::action::Group;
use crate::keymap::{Action, Keymap};
use crate::theme::Theme;
use iced::widget::{column, container, row, scrollable, text};
use iced::{Alignment, Element, Fill, Length};

pub struct HelpView<'a> {
    pub keymap: &'a Keymap,
    pub theme: &'a Theme,
}

#[must_use]
pub fn view<'a, Message: 'a>(help: HelpView<'a>) -> Element<'a, Message, Theme> {
    let theme = help.theme;
    let muted = theme.text_style(theme.muted_colour());
    let mut body = column![].spacing(2).width(Fill);

    body = body.push(
        container(
            text(
                "Every one of these can also be done with the mouse. The keyboard is the \
                 quicker way, not the only way.",
            )
            .size(11)
            .style(move |_t: &Theme| muted),
        )
        .padding([2, 8])
        .width(Fill),
    );

    for group in Group::ALL {
        let mut rows = column![].spacing(1).width(Fill);
        let mut any = false;

        for action in Action::all() {
            if action.group() != *group {
                continue;
            }
            // An action with no binding in this keymap is not a gap to
            // apologise for -- it is reachable another way, and §10 expects a
            // second keymap to bind a different subset.
            let keys = help.keymap.bindings_for(action);
            if keys.is_empty() {
                continue;
            }
            any = true;

            let labels = keys
                .iter()
                .map(|binding| binding.label())
                .collect::<Vec<_>>()
                .join("  or  ");

            rows = rows.push(
                container(
                    row![
                        container(text(action.describe()).size(13)).width(Fill),
                        text(labels).size(12).style(move |_t: &Theme| muted),
                    ]
                    .spacing(8)
                    .padding([0, 8])
                    .align_y(Alignment::Center)
                    .height(Length::Fixed(22.0)),
                )
                .width(Fill),
            );
        }

        if any {
            body = body.push(gap());
            body = body.push(heading(group.heading(), theme));
            body = body.push(rows);
        }
    }

    scrollable(body)
        .spacing(crate::widgets::SCROLLBAR_SPACING)
        .width(Fill)
        .height(Fill)
        .into()
}

fn gap<'a, Message: 'a>() -> Element<'a, Message, Theme> {
    iced::widget::Space::new()
        .height(Length::Fixed(12.0))
        .into()
}

fn heading<'a, Message: 'a>(label: &'static str, theme: &Theme) -> Element<'a, Message, Theme> {
    let muted = theme.text_style(theme.muted_colour());
    container(text(label).size(11).style(move |_t: &Theme| muted))
        .padding([4, 8])
        .width(Fill)
        .into()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn every_bound_action_appears_somewhere() {
        // The cheatsheet is only worth having if it is complete. Walking the
        // groups must reach every action the standard keymap binds.
        let keymap = Keymap::standard();
        for action in Action::all() {
            if keymap.bindings_for(action).is_empty() {
                continue;
            }
            assert!(
                Group::ALL.contains(&action.group()),
                "{action:?} is bound but its group is not shown"
            );
        }
    }

    #[test]
    fn every_group_has_something_in_it() {
        // An empty heading is a hole in the list, and means an action moved
        // group without the heading following it.
        let keymap = Keymap::standard();
        for group in Group::ALL {
            let populated = Action::all()
                .into_iter()
                .filter(|a| a.group() == *group)
                .any(|a| !keymap.bindings_for(a).is_empty());
            assert!(populated, "{group:?} has no bound actions under it");
        }
    }

    #[test]
    fn a_binding_reads_the_way_the_key_is_printed() {
        use crate::keymap::{Mods, Trigger};

        let plain = crate::keymap::Binding::new(Trigger::Char('?'), Mods::NONE, Action::ToggleHelp);
        assert_eq!(plain.label(), "?");

        let ctrl = crate::keymap::Binding::new(Trigger::Char('f'), Mods::CTRL, Action::FocusSearch);
        assert_eq!(ctrl.label(), "Ctrl+F");

        let arrow = crate::keymap::Binding::new(Trigger::Up, Mods::SHIFT, Action::ExtendUp);
        assert_eq!(arrow.label(), "Shift+↑");
    }
}
