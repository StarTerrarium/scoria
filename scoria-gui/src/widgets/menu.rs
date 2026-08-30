//! The right-click menu on a track.
//!
//! ## Why it is not one of §9's overlays
//!
//! §9's overlays are a family: drawers that rise from the bottom, animate,
//! resize by dragging, and never cover the transport. One `Option` holds the
//! open one, and one set of rules governs all of them. A menu at the pointer
//! shares none of that, and forcing it into the same enum would mean the rules
//! stated for the family stopped describing the family. It is its own thing
//! with its own state, and it shares the one rule that matters: Escape closes
//! it, and closes it first.
//!
//! ## What it is for
//!
//! A mouse had no way to reach a track's metadata; `Ctrl+I` was the only path
//! to §9.3's properties. It also turned out to be the missing door for two
//! features that were built and could not be reached: §6.4's "play just this
//! album", whose two stated routes -- a group header and the album pane -- do
//! not exist, and `Command::Play` for a single track.
//!
//! This module knows what the items are and where the panel goes. It does not
//! know what any of them mean; the application decides that.

use crate::keymap::Keymap;
use crate::keymap::action::Action;
use crate::theme::Theme;
use iced::widget::{
    Space, button, column, container, mouse_area, pin, responsive, row, stack, text,
};
use iced::{Element, Fill, Length, Point, Size};
use scoria_core::model::Track;

/// One row of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub event: Event,
    pub label: String,
    /// The keystroke that does the same thing, where there is one.
    ///
    /// Read from the keymap rather than written down, as §8's cheatsheet is:
    /// a binding that changes changes this, and one added later appears
    /// without anyone editing the menu.
    pub accelerator: Option<String>,
    /// Whether a rule is drawn above this item.
    pub separated: bool,
}

/// What the menu can ask for. The application decides what each means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Snapshot just this album (§6.4).
    PlayAlbum,
    /// §9.1, and invariant 6's one exception to artwork following playback.
    ShowArtwork,
    /// §9.3's metadata dump.
    Properties,
    FilterArtist,
    FilterAlbum,
    Dismiss,
}

/// How wide the panel is.
///
/// Fixed rather than fitted to the longest label: an artist name can be very
/// long, and a menu that changes width with the row it was opened on is a menu
/// whose items move under the pointer.
pub const WIDTH: f32 = 240.0;

const ROW_HEIGHT: f32 = 26.0;
const SEPARATOR: f32 = 7.0;
const PADDING: f32 = 5.0;

/// What the menu offers for `track`.
#[must_use]
pub fn items(track: &Track, keymap: &Keymap) -> Vec<Item> {
    let accelerator = |action: Action| {
        keymap
            .bindings_for(action)
            .first()
            .map(|binding| binding.label())
    };

    vec![
        Item {
            event: Event::PlayAlbum,
            label: "Play album".to_owned(),
            accelerator: None,
            separated: false,
        },
        Item {
            event: Event::ShowArtwork,
            label: "Show album art".to_owned(),
            accelerator: None,
            separated: true,
        },
        Item {
            event: Event::Properties,
            label: "Track metadata".to_owned(),
            accelerator: accelerator(Action::ShowProperties),
            separated: false,
        },
        Item {
            event: Event::FilterArtist,
            // The structural value (§4.1), because that is what the filter
            // keys on -- so the menu names what it will actually do.
            label: format!("Filter to {}", track.album_artist),
            accelerator: None,
            separated: true,
        },
        Item {
            event: Event::FilterAlbum,
            label: format!("Filter to {}", track.album),
            accelerator: None,
            separated: false,
        },
    ]
}

/// How tall a menu of these items is.
#[must_use]
pub fn height(items: &[Item]) -> f32 {
    let rows = items.len() as f32 * ROW_HEIGHT;
    let rules = items.iter().filter(|item| item.separated).count() as f32 * SEPARATOR;
    rows + rules + PADDING * 2.0
}

/// Where to put the panel so that it stays on screen.
///
/// Opens down and to the right of the pointer, the way a menu does, and flips
/// back across it when there is not room -- a right-click near the bottom edge
/// has to open upwards rather than off the window. Where there is no room on
/// either side, which happens when the window is smaller than the menu, it
/// clamps rather than choosing the worse overflow.
#[must_use]
pub fn position(at: Point, menu: Size, available: Size) -> Point {
    let place = |pointer: f32, size: f32, limit: f32| {
        if pointer + size <= limit {
            pointer
        } else {
            // Flip to the other side of the pointer, then clamp, so a menu
            // taller than the window starts at the edge rather than above it.
            (pointer - size).clamp(0.0, (limit - size).max(0.0))
        }
    };
    Point::new(
        place(at.x, menu.width, available.width),
        place(at.y, menu.height, available.height),
    )
}

/// Builds the menu layer: the panel, and a sheet behind it that closes on a
/// click anywhere else.
#[must_use]
pub fn view<'a, Message: Clone + 'a>(
    items: Vec<Item>,
    at: Point,
    theme: &Theme,
    on_event: impl Fn(Event) -> Message + Copy + 'a,
) -> Element<'a, Message, Theme> {
    let panel_style = theme.menu_style();
    let muted = theme.text_style(theme.muted_colour());
    let rule_style = theme.rule_style();
    let size = Size::new(WIDTH, height(&items));

    responsive(move |available| {
        let origin = position(at, size, available);

        let mut list = column![].width(Fill);
        for item in &items {
            if item.separated {
                list = list.push(
                    container(
                        iced::widget::rule::horizontal(1).style(move |_t: &Theme| rule_style),
                    )
                    .padding([3, 0]),
                );
            }
            let accelerator = item.accelerator.clone().unwrap_or_default();
            // A button rather than a `mouse_area`, because a menu row has to
            // light up under the pointer and a `mouse_area` cannot say whether
            // it is hovered. Without it there is no telling which row a click
            // will land on.
            list = list.push(
                button(
                    row![
                        text(item.label.clone()).size(13).width(Fill),
                        text(accelerator).size(11).style(move |_t: &Theme| muted),
                    ]
                    .spacing(10)
                    .align_y(iced::Alignment::Center),
                )
                .padding([0, 8])
                .height(Length::Fixed(ROW_HEIGHT))
                .width(Fill)
                .on_press(on_event(item.event))
                .style(|t: &Theme, status| t.menu_item(status)),
            );
        }

        let panel = container(list)
            .width(Length::Fixed(size.width))
            .padding(PADDING)
            .style(move |_t: &Theme| panel_style);

        stack![
            // A click anywhere else closes it. The same sheet the table uses
            // to keep a column drag alive once the pointer leaves the divider.
            mouse_area(Space::new().width(Fill).height(Fill)).on_press(on_event(Event::Dismiss)),
            pin(panel).x(origin.x).y(origin.y),
        ]
        .width(Fill)
        .height(Fill)
        .into()
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use scoria_core::date::Date;
    use scoria_core::model::{AudioProperties, ReplayGain, Timestamp};
    use scoria_core::path::RelativePath;
    use scoria_core::{AlbumId, TrackId};
    use std::time::Duration;

    fn track() -> Track {
        Track {
            id: TrackId(1),
            album_id: AlbumId(7),
            path: RelativePath::new("a/01.flac").unwrap_or_else(|_| unreachable_path()),
            title: "Wonderland".to_owned(),
            artist: "BAND-MAID".to_owned(),
            album_artist: "BAND-MAID".to_owned(),
            album_artist_sort: None,
            album: "CONQUEROR".to_owned(),
            date: Some(Date::from_year(2019)),
            disc: Some(1),
            track_number: Some(7),
            duration: Duration::from_secs(235),
            replay_gain: ReplayGain::default(),
            added_at: Timestamp(0),
            play_count: 0,
            genre: None,
            composer: None,
            audio: AudioProperties::default(),
            available: true,
        }
    }

    /// A path that is always relative, without an `unwrap` in a helper.
    fn unreachable_path() -> RelativePath {
        RelativePath::new("x").unwrap_or_else(|_| unreachable_path())
    }

    // --------------------------------------------------------------- items --

    #[test]
    fn the_menu_offers_what_a_mouse_could_not_otherwise_reach() {
        let menu_items = items(&track(), &Keymap::standard());
        let events: Vec<Event> = menu_items.iter().map(|item| item.event).collect();
        assert_eq!(
            events,
            vec![
                Event::PlayAlbum,
                Event::ShowArtwork,
                Event::Properties,
                Event::FilterArtist,
                Event::FilterAlbum,
            ]
        );
    }

    #[test]
    fn showing_the_album_art_survives_being_moved_into_a_menu() {
        // Invariant 6: the right-click action on a specific track is the one
        // documented exception to artwork following what is playing. Dropping
        // it while rearranging this menu would quietly break that.
        let menu_items = items(&track(), &Keymap::standard());
        assert!(
            menu_items
                .iter()
                .any(|item| item.event == Event::ShowArtwork)
        );
    }

    #[test]
    fn the_filters_name_what_they_will_actually_do() {
        // §4.1: filtering keys on the album artist, not the display artist.
        // A track whose two differ must name the one the filter will use, or
        // the menu promises something else.
        let mut track = track();
        track.artist = "BAND-MAID feat. Someone".to_owned();
        track.album_artist = "BAND-MAID".to_owned();

        let menu_items = items(&track, &Keymap::standard());
        let artist = menu_items
            .iter()
            .find(|item| item.event == Event::FilterArtist)
            .map(|item| item.label.clone())
            .unwrap_or_default();
        assert_eq!(artist, "Filter to BAND-MAID");
    }

    #[test]
    fn the_accelerator_comes_from_the_keymap_rather_than_the_menu() {
        // §8's principle: a binding that changes changes this. Written down
        // here it would be wrong the first time anyone edited the keymap.
        let menu_items = items(&track(), &Keymap::standard());
        let properties = menu_items
            .iter()
            .find(|item| item.event == Event::Properties)
            .and_then(|item| item.accelerator.clone());
        assert_eq!(properties.as_deref(), Some("Ctrl+I"));

        // And an action with no binding shows nothing rather than a blank key.
        let unbound = Keymap {
            name: "empty".to_owned(),
            bindings: Vec::new(),
        };
        let unbound_items = items(&track(), &unbound);
        assert!(
            unbound_items.iter().all(|item| item.accelerator.is_none()),
            "an unbound action invented a keystroke"
        );
    }

    // ------------------------------------------------------------ position --

    const MENU: Size = Size {
        width: 240.0,
        height: 150.0,
    };
    const WINDOW: Size = Size {
        width: 1200.0,
        height: 760.0,
    };

    #[test]
    fn a_menu_with_room_opens_down_and_to_the_right() {
        // Where a menu goes when nothing is in the way, which is most of the
        // time and the thing everything else is a departure from.
        assert_eq!(
            position(Point::new(300.0, 200.0), MENU, WINDOW),
            Point::new(300.0, 200.0)
        );
    }

    #[test]
    fn a_menu_near_the_bottom_opens_upwards() {
        // The case that prompted clamping at all: right-clicking the last row
        // of the table is right-clicking near the bottom of the window.
        let at = Point::new(300.0, 700.0);
        let placed = position(at, MENU, WINDOW);
        assert!(placed.y < at.y, "it opened downwards off the window");
        assert!(placed.y + MENU.height <= WINDOW.height);
    }

    #[test]
    fn a_menu_near_the_right_edge_opens_leftwards() {
        let at = Point::new(1100.0, 200.0);
        let placed = position(at, MENU, WINDOW);
        assert!(placed.x < at.x);
        assert!(placed.x + MENU.width <= WINDOW.width);
    }

    #[test]
    fn a_menu_in_the_corner_flips_both_ways() {
        let placed = position(Point::new(1190.0, 750.0), MENU, WINDOW);
        assert!(placed.x + MENU.width <= WINDOW.width);
        assert!(placed.y + MENU.height <= WINDOW.height);
    }

    #[test]
    fn a_menu_larger_than_the_window_starts_at_the_edge() {
        // Flipping does not help when there is no room either side, and a
        // negative origin would put the first item off the top where it
        // cannot be clicked. Clamped, the menu is cut off at the far end
        // instead, which is the survivable half.
        let tiny = Size {
            width: 100.0,
            height: 80.0,
        };
        let placed = position(Point::new(50.0, 40.0), MENU, tiny);
        assert!(
            placed.x >= 0.0 && placed.y >= 0.0,
            "{placed:?} is off-window"
        );
    }

    #[test]
    fn the_panel_is_as_tall_as_what_is_in_it() {
        let menu_items = items(&track(), &Keymap::standard());
        let height = height(&menu_items);
        assert!(
            height > ROW_HEIGHT * menu_items.len() as f32,
            "the rules take no space"
        );
        assert!(
            height < ROW_HEIGHT * (menu_items.len() + 3) as f32,
            "it is mostly padding"
        );
    }
}
