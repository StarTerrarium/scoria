//! The transport's symbols, drawn rather than typed.
//!
//! ## Why these are geometry and not characters
//!
//! They were characters, and the application had no say in how they looked.
//! `Noto Sans` -- what iced resolves `SansSerif` to on a typical desktop --
//! carries none of the media symbols, so every one of them fell back, and
//! which font caught it depended on what else happened to cover that
//! codepoint. `⏮ ⏸ ⏭` are covered by little except a colour emoji font, so
//! they arrived orange with their colours inside the glyph, past the theme
//! entirely. `▶ ⇄ ⟳ ≡` are in ordinary text fonts and came out monochrome and
//! themed. The result was a row of controls in two visual languages, decided
//! by nobody, different on every machine -- and a play button that did not
//! match the pause button it replaces.
//!
//! `DESIGN.md` §8 had already recorded one instance of this, when U+2928
//! rendered as a cross, and drew the lesson "use characters a system font
//! has". That treats the symptom. These are shapes we own, so they are the
//! same everywhere, they take the theme's colour because they are filled with
//! it, and no font on the machine can change them.
//!
//! ## Drawing
//!
//! Every symbol is built from triangles, bars, an arc and straight strokes,
//! inside a square box. A `canvas::Program` gets the theme *and* the cursor,
//! so an icon can ask for its own colour and notice its own hover without the
//! button around it having to tell it anything.

use crate::theme::{Emphasis, IconPen, IconState, Theme};
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path};
use iced::{Element, Length, Point, Rectangle, Renderer, Size, mouse};

/// One transport symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Previous,
    Play,
    Pause,
    Next,
    Shuffle,
    Repeat,
    /// Repeat one, which is the loop with a bar through it -- a shape, so it
    /// reads without depending on the colour telling you the mode changed.
    RepeatOne,
    JumpToPlaying,
    Queue,
}

impl Icon {
    /// Every icon, so the set can be checked for gaps.
    #[must_use]
    pub const fn all() -> [Self; 9] {
        [
            Self::Previous,
            Self::Play,
            Self::Pause,
            Self::Next,
            Self::Shuffle,
            Self::Repeat,
            Self::RepeatOne,
            Self::JumpToPlaying,
            Self::Queue,
        ]
    }

    /// How much of the eye it is meant to take.
    #[must_use]
    pub const fn emphasis(self) -> Emphasis {
        match self {
            Self::Previous | Self::Play | Self::Pause | Self::Next => Emphasis::Primary,
            Self::Shuffle | Self::Repeat | Self::RepeatOne | Self::JumpToPlaying | Self::Queue => {
                Emphasis::Secondary
            }
        }
    }

    /// The side of the box it is drawn in.
    ///
    /// One size for everything but play and pause, which are a little larger
    /// as the anchor. The secondary icons are *not* drawn smaller: a row of
    /// controls at two footprints reads as two rows, and the tier is already
    /// carried by [`Self::stroke`] and by the colour the theme picks from
    /// [`Emphasis`]. Same size, less weight.
    #[must_use]
    pub const fn size(self) -> f32 {
        match self {
            Self::Play | Self::Pause => 20.0,
            _ => 18.0,
        }
    }

    /// How thick this icon's strokes are, as a fraction of its box.
    ///
    /// What separates the tiers now that they are the same size. The primary
    /// controls are mostly filled shapes and read as solid; the secondary ones
    /// are line art, and a thinner line is the difference between "the same
    /// icon, quieter" and "a different icon".
    #[must_use]
    pub const fn stroke(self) -> f32 {
        match self.emphasis() {
            Emphasis::Primary => 0.1,
            Emphasis::Secondary => 0.075,
        }
    }
}

/// How much of the box the symbol itself occupies.
///
/// The rest is breathing room, and the strip at the bottom where an active
/// toggle puts its mark.
const GLYPH: f32 = 0.66;

/// How much smaller an arrowhead is than the shape it would be on its own.
///
/// An arrow that terminates a line should stay line art. Left at full size the
/// heads dominate, and shuffle turns into a solid blob rather than two routes.
const HEAD: f32 = 0.8;

/// Builds an icon, sized and coloured for where it sits.
#[must_use]
pub fn view<'a, Message: 'a>(icon: Icon, active: bool) -> Element<'a, Message, Theme> {
    let side = icon.size();
    Canvas::new(Symbol { icon, active })
        .width(Length::Fixed(side))
        .height(Length::Fixed(side))
        .into()
}

struct Symbol {
    icon: Icon,
    active: bool,
}

impl<Message> canvas::Program<Message, Theme, Renderer> for Symbol {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let pen = theme.icon_pen(IconState {
            emphasis: self.icon.emphasis(),
            active: self.active,
            hovered: cursor.is_over(bounds),
        });

        let mut frame = Frame::new(renderer, bounds.size());
        let side = bounds.width.min(bounds.height);
        let glyph = side * GLYPH;
        // Centred horizontally, and sitting a little high so the mark below
        // has somewhere to go without the symbol looking off-centre.
        let origin = Point::new((side - glyph) / 2.0, (side - glyph) / 2.0 - side * 0.06);

        draw(self.icon, &mut frame, origin, glyph, pen);

        if self.active {
            // §11: the accent is not allowed to be the only thing saying a
            // toggle is on. A bar under the icon says it in a second channel,
            // the way the playing row carries a glyph as well as a tint.
            let width = glyph * 0.6;
            let mark = Path::rectangle(
                Point::new((side - width) / 2.0, side - side * 0.12),
                Size::new(width, (side * 0.09).max(1.5)),
            );
            frame.fill(&mark, pen.fill());
        }

        vec![frame.into_geometry()]
    }
}

/// Draws `icon` into a `glyph`-sided square whose top left is `origin`.
fn draw(icon: Icon, frame: &mut Frame, origin: Point, glyph: f32, pen: IconPen) {
    let at = |x: f32, y: f32| Point::new(origin.x + x * glyph, origin.y + y * glyph);
    // Never below a pixel: a half-pixel stroke is a smudge rather than a
    // lighter line, which is the opposite of what thinning it is for.
    let stroke = (glyph * icon.stroke()).max(1.0);

    // A filled triangle pointing right, from three fractional corners. `scale`
    // shrinks it toward its own centre, which is how an arrowhead is kept
    // lighter than a shape that stands alone.
    let wedge = |frame: &mut Frame,
                 left: f32,
                 right: f32,
                 top: f32,
                 bottom: f32,
                 flip: bool,
                 scale: f32| {
        let (cx, cy) = ((left + right) / 2.0, (top + bottom) / 2.0);
        let (left, right) = (cx + (left - cx) * scale, cx + (right - cx) * scale);
        let (top, bottom) = (cy + (top - cy) * scale, cy + (bottom - cy) * scale);
        let (a, b) = if flip { (right, left) } else { (left, right) };
        let path = Path::new(|p| {
            p.move_to(at(a, top));
            p.line_to(at(b, (top + bottom) / 2.0));
            p.line_to(at(a, bottom));
            p.close();
        });
        frame.fill(&path, pen.fill());
    };
    let triangle = |frame: &mut Frame, left: f32, right: f32, top: f32, bottom: f32, flip: bool| {
        wedge(frame, left, right, top, bottom, flip, 1.0);
    };
    let arrowhead =
        |frame: &mut Frame, left: f32, right: f32, top: f32, bottom: f32, flip: bool| {
            wedge(frame, left, right, top, bottom, flip, HEAD);
        };

    // `weight` is a multiple of the stroke. Pause and the skip bars carry more
    // than a line does, so they hold their own beside a filled triangle.
    let bar = |frame: &mut Frame, x: f32, top: f32, bottom: f32, weight: f32| {
        let path = Path::rectangle(
            at(x, top),
            Size::new(stroke * weight, (bottom - top).mul_add(glyph, 0.0)),
        );
        frame.fill(&path, pen.fill());
    };

    let line = |frame: &mut Frame, from: (f32, f32), to: (f32, f32)| {
        let path = Path::line(at(from.0, from.1), at(to.0, to.1));
        frame.stroke(&path, pen.stroke(stroke));
    };

    match icon {
        // A triangle, filled, pointing the way playback goes.
        Icon::Play => triangle(frame, 0.15, 0.9, 0.05, 0.95, false),
        // Two bars, weighted to match the mass of the play triangle they stand
        // in for, so the button does not appear to change size with its state.
        Icon::Pause => {
            bar(frame, 0.22, 0.06, 0.94, 1.6);
            bar(frame, 0.62, 0.06, 0.94, 1.6);
        }
        // A triangle into a bar: skip, and stop at the edge.
        Icon::Previous => {
            triangle(frame, 0.25, 1.0, 0.1, 0.9, true);
            bar(frame, 0.08, 0.1, 0.9, 1.4);
        }
        Icon::Next => {
            triangle(frame, 0.0, 0.75, 0.1, 0.9, false);
            bar(frame, 0.78, 0.1, 0.9, 1.4);
        }
        // Two paths crossing, each ending in an arrowhead: the order changes.
        // The lines stop short of the edge so the heads are the rightmost
        // thing, which is what makes it read as two routes rather than an X.
        Icon::Shuffle => {
            line(frame, (0.04, 0.26), (0.70, 0.74));
            line(frame, (0.04, 0.74), (0.70, 0.26));
            arrowhead(frame, 0.70, 1.0, 0.60, 0.88, false);
            arrowhead(frame, 0.70, 1.0, 0.12, 0.40, false);
        }
        // A loop, with the gaps and arrowheads that say it goes round.
        Icon::Repeat | Icon::RepeatOne => {
            line(frame, (0.14, 0.26), (0.78, 0.26));
            line(frame, (0.9, 0.36), (0.9, 0.64));
            line(frame, (0.86, 0.74), (0.22, 0.74));
            line(frame, (0.1, 0.64), (0.1, 0.36));
            arrowhead(frame, 0.66, 0.94, 0.06, 0.46, false);
            arrowhead(frame, 0.06, 0.34, 0.54, 0.94, true);
            if icon == Icon::RepeatOne {
                // A bar through the middle: one. Heavier than a line and
                // spanning most of the loop's inside, because at fourteen
                // pixels anything smaller is not a difference at all.
                bar(frame, 0.45, 0.34, 0.66, 1.8);
            }
        }
        // A target: reveal the current item, as editors and file managers do.
        Icon::JumpToPlaying => {
            let centre = at(0.5, 0.5);
            let ring = Path::circle(centre, glyph * 0.3);
            frame.stroke(&ring, pen.stroke(stroke));
            frame.fill(&Path::circle(centre, glyph * 0.1), pen.fill());
            line(frame, (0.5, 0.0), (0.5, 0.14));
            line(frame, (0.5, 0.86), (0.5, 1.0));
            line(frame, (0.0, 0.5), (0.14, 0.5));
            line(frame, (0.86, 0.5), (1.0, 0.5));
        }
        // Three lines: a list of what is coming.
        Icon::Queue => {
            for y in [0.15_f32, 0.5, 0.85] {
                let path = Path::rectangle(at(0.0, y), Size::new(glyph, stroke));
                frame.fill(&path, pen.fill());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_transport_reads_as_two_tiers() {
        // §8's hierarchy, pinned. The left cluster moves through what is
        // playing and is reached for constantly; the right cluster changes
        // what happens next and is set once. Flattening this by accident
        // should fail rather than just look wrong.
        for icon in Icon::all() {
            let expected = match icon {
                Icon::Previous | Icon::Play | Icon::Pause | Icon::Next => Emphasis::Primary,
                _ => Emphasis::Secondary,
            };
            assert_eq!(icon.emphasis(), expected, "{icon:?}");
        }
    }

    #[test]
    fn the_tiers_are_carried_by_weight_rather_than_by_size() {
        // Deliberate, and the second attempt: drawing the right cluster
        // smaller made the transport read as two rows of controls rather than
        // one. They share a footprint now, and the tier comes from a thinner
        // stroke and the colour the theme picks from the emphasis.
        let footprint = Icon::Previous.size();
        for icon in Icon::all() {
            if matches!(icon, Icon::Play | Icon::Pause) {
                // The anchor, and the only thing allowed to be bigger.
                assert!(icon.size() >= footprint, "{icon:?}");
            } else {
                assert!(
                    (icon.size() - footprint).abs() < f32::EPSILON,
                    "{icon:?} is {} where the row is {footprint}",
                    icon.size()
                );
            }
        }

        assert!(
            Icon::Queue.stroke() < Icon::Previous.stroke(),
            "the secondary icons are no lighter than the primary ones"
        );
    }

    #[test]
    fn play_and_pause_are_the_same_size() {
        // They are one button in two states. They were drawn by two different
        // fonts in two different colours, which is what prompted all of this;
        // the least they can do now is not change size.
        assert!((Icon::Play.size() - Icon::Pause.size()).abs() < f32::EPSILON);
    }

    #[test]
    fn every_icon_has_a_box_to_draw_in() {
        for icon in Icon::all() {
            assert!(icon.size() > 0.0, "{icon:?} has no size");
        }
    }
}
