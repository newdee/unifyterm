//! Box-drawing lines and block elements, drawn as rectangles and not taken
//! from a font: a font's glyphs rarely meet the cell's edges exactly, which
//! leaves gaps in frames and stripes in pictures made of half blocks.
//!
//! Everything here is in whole device pixels, so two neighbors' edges are
//! the same number and not two numbers that round apart.

/// A rectangle inside one cell, in device pixels from the cell's top left:
/// `[left, top, right, bottom]`.
pub(crate) type Rect = [u32; 4];

/// The cell's width over this is a light line's thickness.
const LIGHT_DIVISOR: u32 = 8;
/// A cell's side is cut in this many parts for block elements (`▁` is 1 of 8).
const EIGHTHS: u32 = 8;

const LINES_START: u32 = 0x2500;
const BLOCKS_START: u32 = 0x2580;
const BLOCKS_END: u32 = 0x259F;
/// Where the half lines (`╴` to `╿`) start, as an offset from `LINES_START`.
const HALF_LINES: usize = 0x74;

/// Each line character's four arms, in the order up, right, down, left:
/// `.` none, `l` light, `h` heavy. From U+2500; dashed lines are drawn solid.
#[rustfmt::skip]
const ARMS: [&str; 80] = [
    ".l.l", ".h.h", "l.l.", "h.h.", ".l.l", ".h.h", "l.l.", "h.h.", ".l.l", ".h.h", "l.l.", "h.h.",
    ".ll.", ".hl.", ".lh.", ".hh.", "..ll", "..lh", "..hl", "..hh",
    "ll..", "lh..", "hl..", "hh..", "l..l", "l..h", "h..l", "h..h",
    "lll.", "lhl.", "hll.", "llh.", "hlh.", "hhl.", "lhh.", "hhh.",
    "l.ll", "l.lh", "h.ll", "l.hl", "h.hl", "h.lh", "l.hh", "h.hh",
    ".lll", ".llh", ".hll", ".hlh", ".lhl", ".lhh", ".hhl", ".hhh",
    "ll.l", "ll.h", "lh.l", "lh.h", "hl.l", "hl.h", "hh.l", "hh.h",
    "llll", "lllh", "lhll", "lhlh", "hlll", "llhl", "hlhl", "hllh",
    "hhll", "llhh", "lhhl", "hhlh", "lhhh", "hlhh", "hhhl", "hhhh",
    ".l.l", ".h.h", "l.l.", "h.h.",
];
/// The same for `╴ ╵ ╶ ╷ ╸ ╹ ╺ ╻ ╼ ╽ ╾ ╿` (from U+2574).
const HALF_ARMS: [&str; 12] =
    ["...l", "l...", ".l..", "..l.", "...h", "h...", ".h..", "..h.", ".h.l", "l.h.", ".l.h", "h.l."];

fn arms(c: char) -> Option<&'static str> {
    let offset = usize::try_from(u32::from(c).checked_sub(LINES_START)?).ok()?;
    ARMS.get(offset).or_else(|| HALF_ARMS.get(offset.checked_sub(HALF_LINES)?)).copied()
}

/// Whether [`rects`] draws `c`.
pub(crate) fn draws(c: char) -> bool {
    arms(c).is_some() || (BLOCKS_START..=BLOCKS_END).contains(&u32::from(c))
}

/// The rectangles that make up `c` in a cell `w` by `h` device pixels, and
/// how opaque they are (less than 1 for the shades `░▒▓`). `None` for a
/// character this module does not draw.
pub(crate) fn rects(c: char, w: u32, h: u32) -> Option<(Vec<Rect>, f32)> {
    if let Some(arms) = arms(c) {
        return Some((line_rects(arms, w, h), 1.0));
    }
    block(c).map(|(parts, opacity)| {
        // A part's edges are in eighths of the cell, rounded to whole pixels.
        let pixels = |eighths: u32, side: u32| (eighths * side + EIGHTHS / 2) / EIGHTHS;
        let scaled = parts.iter().map(|p| [pixels(p[0], w), pixels(p[1], h), pixels(p[2], w), pixels(p[3], h)]);
        (scaled.collect(), opacity)
    })
}

fn line_rects(arms: &str, w: u32, h: u32) -> Vec<Rect> {
    // A light line is an eighth of the cell's width, rounded, and at least a pixel.
    let light = ((w + LIGHT_DIVISOR / 2) / LIGHT_DIVISOR).max(1);
    let thickness = |arm: u8| match arm {
        b'l' => light,
        b'h' => light * 2,
        _ => 0,
    };
    let [up, right, down, left] = [0, 1, 2, 3].map(|i| thickness(arms.as_bytes()[i]));
    // An arm runs from its edge to the far side of the thickest line it
    // crosses at the center, so corners have no notch.
    let (across, along) = (up.max(down), left.max(right));
    // Where a line of thickness `t` starts so that it is centered on a
    // side of length `side` (an odd pixel left over goes before the line).
    let centered = |side: u32, t: u32| side.saturating_sub(t).div_ceil(2);
    let (column, row) = (|t: u32| centered(w, t), |t: u32| centered(h, t));
    let mut rects = Vec::new();
    if up > 0 {
        rects.push([column(up), 0, column(up) + up, row(along) + along]);
    }
    if down > 0 {
        rects.push([column(down), row(along), column(down) + down, h]);
    }
    if left > 0 {
        rects.push([0, row(left), column(across) + across, row(left) + left]);
    }
    if right > 0 {
        rects.push([column(across), row(right), w, row(right) + right]);
    }
    rects
}

/// A block element as rectangles in eighths of the cell, and its opacity.
fn block(c: char) -> Option<(Vec<Rect>, f32)> {
    const HALF: u32 = EIGHTHS / 2;
    const FULL: u32 = EIGHTHS;
    let (ul, ur) = ([0, 0, HALF, HALF], [HALF, 0, FULL, HALF]);
    let (ll, lr) = ([0, HALF, HALF, FULL], [HALF, HALF, FULL, FULL]);
    let whole = [0, 0, FULL, FULL];
    let code = u32::from(c);
    let solid = |parts: Vec<Rect>| Some((parts, 1.0));
    match code {
        0x2580 => solid(vec![[0, 0, FULL, HALF]]),
        // ▁ to █: the lower 1/8 to 8/8.
        0x2581..=0x2588 => solid(vec![[0, FULL - (code - 0x2580), FULL, FULL]]),
        // ▉ to ▏: the left 7/8 to 1/8.
        0x2589..=0x258F => solid(vec![[0, 0, 0x2590 - code, FULL]]),
        0x2590 => solid(vec![[HALF, 0, FULL, FULL]]),
        0x2591 => Some((vec![whole], 0.25)),
        0x2592 => Some((vec![whole], 0.5)),
        0x2593 => Some((vec![whole], 0.75)),
        0x2594 => solid(vec![[0, 0, FULL, 1]]),
        0x2595 => solid(vec![[FULL - 1, 0, FULL, FULL]]),
        0x2596 => solid(vec![ll]),
        0x2597 => solid(vec![lr]),
        0x2598 => solid(vec![ul]),
        0x2599 => solid(vec![ul, ll, lr]),
        0x259A => solid(vec![ul, lr]),
        0x259B => solid(vec![ul, ur, ll]),
        0x259C => solid(vec![ul, ur, lr]),
        0x259D => solid(vec![ur]),
        0x259E => solid(vec![ur, ll]),
        0x259F => solid(vec![ur, ll, lr]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 8;
    const H: u32 = 19;

    fn solid(c: char) -> Vec<Rect> {
        let (rects, opacity) = rects(c, W, H).unwrap();
        assert!((opacity - 1.0).abs() < f32::EPSILON, "{c} is drawn solid, not at {opacity}");
        rects
    }

    #[test]
    fn letters_are_left_to_the_font() {
        assert!(!draws('a') && !draws('中') && !draws('╭') && !draws('═'));
        assert_eq!(rects('a', W, H), None);
        assert!(draws('─') && draws('╿') && draws('▀') && draws('▟'));
    }

    #[test]
    fn a_horizontal_line_spans_the_cell() {
        assert_eq!(solid('─'), [[0, 9, 4, 10], [4, 9, 8, 10]]);
        // Heavy: twice as thick, still edge to edge.
        assert_eq!(solid('━'), [[0, 9, 4, 11], [4, 9, 8, 11]]);
    }

    #[test]
    fn a_vertical_line_spans_the_cell() {
        let rects = solid('│');
        assert_eq!((rects[0][1], rects[1][3]), (0, H), "from the top edge to the bottom edge");
        assert!(rects.iter().all(|r| (r[0], r[2]) == (4, 5)), "one pixel wide, centered: {rects:?}");
        assert_eq!((rects[0][3], rects[1][1]), (10, 10), "the halves meet");
    }

    #[test]
    fn a_corner_has_no_notch() {
        // ┌: down and right. The down arm starts at the line's top, the
        // right arm at the down arm's left: together they cover the corner.
        assert_eq!(solid('┌'), [[4, 9, 5, 19], [4, 9, 8, 10]]);
    }

    #[test]
    fn every_line_character_touches_exactly_its_edges() {
        for offset in (0..80).chain(0x74..0x80) {
            let c = char::from_u32(0x2500 + offset).unwrap();
            let spec = arms(c).unwrap().as_bytes();
            let rects = solid(c);
            let tall = |r: &Rect| r[3] - r[1] > r[2] - r[0];
            let touches = [
                rects.iter().any(|r| r[1] == 0 && tall(r)),
                rects.iter().any(|r| r[2] == W && !tall(r)),
                rects.iter().any(|r| r[3] == H && tall(r)),
                rects.iter().any(|r| r[0] == 0 && !tall(r)),
            ];
            for (arm, touch) in spec.iter().zip(touches) {
                assert_eq!(*arm != b'.', touch, "{c} ({})", std::str::from_utf8(spec).unwrap());
            }
            let inside = |r: &Rect| r[2] <= W && r[3] <= H && r[0] < r[2] && r[1] < r[3];
            assert!(rects.iter().all(inside), "{c}: {rects:?}");
        }
    }

    #[test]
    fn half_blocks_meet_in_the_middle() {
        let (upper, lower) = (solid('▀'), solid('▄'));
        assert_eq!(upper, [[0, 0, 8, 10]]);
        assert_eq!(lower, [[0, 10, 8, 19]], "starts where the upper half ends");
        assert_eq!(solid('█'), [[0, 0, 8, 19]]);
        assert_eq!(solid('▌'), [[0, 0, 4, 19]]);
        assert_eq!(solid('▐'), [[4, 0, 8, 19]]);
    }

    #[test]
    fn eighths_grow_from_the_bottom_and_the_left() {
        assert_eq!(solid('▁'), [[0, 17, 8, 19]]);
        assert_eq!(solid('▇'), [[0, 2, 8, 19]]);
        assert_eq!(solid('▏'), [[0, 0, 1, 19]]);
        assert_eq!(solid('▉'), [[0, 0, 7, 19]]);
    }

    #[test]
    fn quadrants_and_shades() {
        assert_eq!(solid('▖'), [[0, 10, 4, 19]]);
        assert_eq!(solid('▚').len(), 2);
        assert_eq!(solid('▟').len(), 3);
        let (rects, opacity) = rects('▒', W, H).unwrap();
        assert_eq!(rects, [[0, 0, 8, 19]]);
        assert!((opacity - 0.5).abs() < f32::EPSILON, "{opacity}");
    }

    #[test]
    fn a_cell_of_no_size_gives_no_inverted_rectangle() {
        for c in ['─', '┼', '█', '▚'] {
            let (rects, _) = rects(c, 0, 0).unwrap();
            assert!(rects.iter().all(|r| r[0] <= r[2] && r[1] <= r[3]), "{c}: {rects:?}");
        }
    }
}
