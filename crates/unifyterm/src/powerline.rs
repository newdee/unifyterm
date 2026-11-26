//! Powerline's separators (U+E0B0 to U+E0BF), drawn as shapes and not
//! taken from a font: the prompt UnifyTerm writes uses them, and few fonts
//! have them (Cascadia Code, Consolas and Menlo do not). Drawn, they also
//! meet the cell's edges exactly, so the color before and after a separator
//! join without a seam.
//!
//! A shape is polygons in device pixels from the cell's top left; a round
//! side is many short lines.

/// Which way a shape points: its tip, or its round side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Toward {
    Left,
    Right,
}

/// The corner a right triangle fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Corner {
    UpperLeft,
    UpperRight,
    LowerLeft,
    LowerRight,
}

/// Which way a diagonal line goes: `/` or `\`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slope {
    Rising,
    Falling,
}

/// One separator's shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// A filled triangle, its tip at the middle of one side (U+E0B0, U+E0B2).
    Triangle(Toward),
    /// That triangle's two slanted sides, as lines (U+E0B1, U+E0B3).
    Chevron(Toward),
    /// A filled half of an ellipse as tall as the cell (U+E0B4, U+E0B6).
    HalfDisc(Toward),
    /// That half ellipse's round side, as a line (U+E0B5, U+E0B7).
    Arc(Toward),
    /// A filled right triangle in a corner (U+E0B8, U+E0BA, U+E0BC, U+E0BE).
    Wedge(Corner),
    /// The cell's diagonal, as a line (U+E0B9, U+E0BB, U+E0BD, U+E0BF).
    Diagonal(Slope),
}

/// The shape of `c`; `None` for a character this module does not draw.
pub(crate) const fn shape(c: char) -> Option<Shape> {
    Some(match c {
        '\u{E0B0}' => Shape::Triangle(Toward::Right),
        '\u{E0B1}' => Shape::Chevron(Toward::Right),
        '\u{E0B2}' => Shape::Triangle(Toward::Left),
        '\u{E0B3}' => Shape::Chevron(Toward::Left),
        '\u{E0B4}' => Shape::HalfDisc(Toward::Right),
        '\u{E0B5}' => Shape::Arc(Toward::Right),
        '\u{E0B6}' => Shape::HalfDisc(Toward::Left),
        '\u{E0B7}' => Shape::Arc(Toward::Left),
        '\u{E0B8}' => Shape::Wedge(Corner::LowerLeft),
        '\u{E0BA}' => Shape::Wedge(Corner::LowerRight),
        '\u{E0BC}' => Shape::Wedge(Corner::UpperLeft),
        '\u{E0BE}' => Shape::Wedge(Corner::UpperRight),
        // The wedges' slanted sides as lines: two of each slope.
        '\u{E0B9}' | '\u{E0BF}' => Shape::Diagonal(Slope::Falling),
        '\u{E0BB}' | '\u{E0BD}' => Shape::Diagonal(Slope::Rising),
        _ => return None,
    })
}

/// Whether this module draws `c`.
pub(crate) const fn draws(c: char) -> bool {
    shape(c).is_some()
}

/// A point in device pixels from the cell's top left.
pub(crate) type Point = [f32; 2];

/// How many straight lines make a half ellipse's round side.
const ROUND_STEPS: u16 = 24;
/// A line is the cell's width over this thick, as a box-drawing light line is, and at least a pixel.
const LINE_DIVISOR: f32 = 8.0;

/// A line's thickness in a cell `w` device pixels wide.
fn thickness(w: f32) -> f32 {
    (w / LINE_DIVISOR).round().max(1.0)
}

/// The points of a half ellipse's round side, from top to bottom: as wide as
/// `across` from the flat side at `x`, as tall as the cell (`h`), the round
/// side `toward` one way. `inset` makes a smaller one inside it.
fn round_side(x: f32, across: f32, h: f32, toward: Toward, inset: f32) -> Vec<Point> {
    let (radius_x, radius_y) = ((across - inset).max(0.0), (h / 2.0 - inset).max(0.0));
    let sign = if toward == Toward::Right { 1.0 } else { -1.0 };
    (0..=ROUND_STEPS)
        .map(|step| {
            // From straight up (-90°) round to straight down (90°).
            let angle = std::f32::consts::PI * (f32::from(step) / f32::from(ROUND_STEPS) - 0.5);
            [x + sign * radius_x * angle.cos(), h / 2.0 + radius_y * angle.sin()]
        })
        .collect()
}

/// `shape` in a cell `w` by `h` device pixels, as polygons to fill.
pub(crate) fn polygons(shape: Shape, w: f32, h: f32) -> Vec<Vec<Point>> {
    let t = thickness(w);
    let middle = h / 2.0;
    match shape {
        Shape::Triangle(Toward::Right) => vec![vec![[0.0, 0.0], [w, middle], [0.0, h]]],
        Shape::Triangle(Toward::Left) => vec![vec![[w, 0.0], [0.0, middle], [w, h]]],
        // The two slanted sides, each `t` thick across: one polygon round both.
        Shape::Chevron(Toward::Right) => {
            vec![vec![[0.0, 0.0], [t, 0.0], [w, middle], [t, h], [0.0, h], [w - t, middle]]]
        }
        Shape::Chevron(Toward::Left) => {
            vec![vec![[w, 0.0], [w - t, 0.0], [0.0, middle], [w - t, h], [w, h], [t, middle]]]
        }
        Shape::HalfDisc(Toward::Right) => vec![round_side(0.0, w, h, Toward::Right, 0.0)],
        Shape::HalfDisc(Toward::Left) => vec![round_side(w, w, h, Toward::Left, 0.0)],
        // The round side between the half ellipse and one `t` smaller inside it.
        Shape::Arc(toward) => {
            let x = if toward == Toward::Right { 0.0 } else { w };
            let mut ring = round_side(x, w, h, toward, 0.0);
            ring.extend(round_side(x, w, h, toward, t).into_iter().rev());
            vec![ring]
        }
        Shape::Wedge(Corner::UpperLeft) => vec![vec![[0.0, 0.0], [w, 0.0], [0.0, h]]],
        Shape::Wedge(Corner::UpperRight) => vec![vec![[0.0, 0.0], [w, 0.0], [w, h]]],
        Shape::Wedge(Corner::LowerLeft) => vec![vec![[0.0, 0.0], [w, h], [0.0, h]]],
        Shape::Wedge(Corner::LowerRight) => vec![vec![[w, 0.0], [w, h], [0.0, h]]],
        Shape::Diagonal(Slope::Rising) => vec![vec![[0.0, h], [t, h], [w, 0.0], [w - t, 0.0]]],
        Shape::Diagonal(Slope::Falling) => vec![vec![[0.0, 0.0], [t, 0.0], [w, h], [w - t, h]]],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 9.0;
    const H: f32 = 19.0;

    /// Whether `point` is inside `polygon` (even-odd rule).
    fn inside(polygon: &[Point], point: Point) -> bool {
        let mut inside = false;
        let mut previous = polygon[polygon.len() - 1];
        for &current in polygon {
            let crosses = (current[1] > point[1]) != (previous[1] > point[1]);
            if crosses {
                let x =
                    previous[0] + (point[1] - previous[1]) / (current[1] - previous[1]) * (current[0] - previous[0]);
                if point[0] < x {
                    inside = !inside;
                }
            }
            previous = current;
        }
        inside
    }

    fn filled(c: char, point: Point) -> bool {
        polygons(shape(c).expect("drawn"), W, H).iter().any(|polygon| inside(polygon, point))
    }

    #[test]
    fn the_sixteen_separators_are_drawn_and_nothing_else() {
        let drawn: Vec<char> = (0xE0A0..=0xE0C0).filter_map(char::from_u32).filter(|c| draws(*c)).collect();
        assert_eq!(drawn.len(), 16);
        assert_eq!((drawn[0], drawn[15]), ('\u{E0B0}', '\u{E0BF}'));
        assert!(!draws('a') && !draws('─') && !draws('\u{E0A0}'), "letters, box lines and the branch sign are not");
    }

    #[test]
    fn every_shape_stays_in_its_cell() {
        for c in (0xE0B0..=0xE0BF).filter_map(char::from_u32) {
            for polygon in polygons(shape(c).expect("drawn"), W, H) {
                assert!(polygon.len() >= 3, "{c:?}");
                for [x, y] in polygon {
                    assert!((-0.001..=W + 0.001).contains(&x) && (-0.001..=H + 0.001).contains(&y), "{c:?}: {x}, {y}");
                }
            }
        }
    }

    #[test]
    fn a_triangle_fills_from_its_flat_side_to_its_tip() {
        // U+E0B0: the whole left edge, the tip at the right edge's middle, nothing at the right corners.
        assert!(filled('\u{E0B0}', [0.5, 1.0]) && filled('\u{E0B0}', [0.5, H - 1.0]));
        assert!(filled('\u{E0B0}', [W - 0.5, H / 2.0]), "the tip");
        assert!(!filled('\u{E0B0}', [W - 0.5, 1.0]) && !filled('\u{E0B0}', [W - 0.5, H - 1.0]));
        // U+E0B2: the mirror.
        assert!(filled('\u{E0B2}', [W - 0.5, 1.0]) && filled('\u{E0B2}', [0.5, H / 2.0]));
        assert!(!filled('\u{E0B2}', [0.5, 1.0]));
    }

    #[test]
    fn a_half_disc_is_round_where_a_triangle_is_pointed() {
        // Between the flat side and the round one, a half disc covers more than the triangle does.
        let point = [W * 0.7, H * 0.25];
        assert!(filled('\u{E0B4}', point) && !filled('\u{E0B0}', point));
        assert!(filled('\u{E0B4}', [0.5, 0.5]), "the flat side's corner");
        assert!(!filled('\u{E0B4}', [W - 0.5, 0.5]), "not the far corner");
        assert!(filled('\u{E0B6}', [W * 0.3, H * 0.25]) && !filled('\u{E0B6}', [0.5, 0.5]), "the mirror");
    }

    #[test]
    fn lines_are_lines_and_wedges_fill_their_corner() {
        assert!(filled('\u{E0B1}', [0.5, 0.5]) && !filled('\u{E0B1}', [0.5, H / 2.0]), "a chevron's middle is empty");
        assert!(
            filled('\u{E0B5}', [W - 0.5, H / 2.0]) && !filled('\u{E0B5}', [1.0, H / 2.0]),
            "an arc's inside is empty"
        );
        assert!(filled('\u{E0BC}', [0.5, 0.5]) && !filled('\u{E0BC}', [W - 0.5, H - 0.5]), "upper left");
        assert!(filled('\u{E0BA}', [W - 0.5, H - 0.5]) && !filled('\u{E0BA}', [0.5, 0.5]), "lower right");
        assert!(filled('\u{E0B8}', [0.5, H - 0.5]) && filled('\u{E0BE}', [W - 0.5, 0.5]));
        assert!(filled('\u{E0BB}', [0.5, H - 0.5]) && filled('\u{E0BB}', [W - 0.5, 0.5]), "/ from corner to corner");
        assert!(!filled('\u{E0BB}', [0.5, 0.5]));
        assert!(filled('\u{E0B9}', [0.5, 0.5]) && filled('\u{E0B9}', [W - 0.5, H - 0.5]), "\\ from corner to corner");
    }

    #[test]
    fn a_line_is_an_eighth_of_the_cell_and_at_least_a_pixel() {
        assert!((thickness(9.0) - 1.0).abs() < f32::EPSILON);
        assert!((thickness(16.0) - 2.0).abs() < f32::EPSILON);
        assert!((thickness(3.0) - 1.0).abs() < f32::EPSILON, "never thinner than a pixel");
    }
}
