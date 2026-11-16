//! Box-drawing lines and block elements, drawn as rectangles and not taken
//! from a font: a font's glyphs rarely meet the cell's edges exactly, which
//! leaves gaps in frames and stripes in pictures made of half blocks.

/// A rectangle inside one cell, in device pixels from the cell's top left:
/// `[left, top, right, bottom]`.
pub type Rect = [f32; 4];

/// The cell's width over this is a light line's thickness.
const LIGHT_DIVISOR: f32 = 8.0;

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
    let offset = (c as u32).checked_sub(LINES_START)? as usize;
    ARMS.get(offset).or_else(|| HALF_ARMS.get(offset.checked_sub(HALF_LINES)?)).copied()
}

/// Whether [`rects`] draws `c`.
pub fn draws(c: char) -> bool {
    arms(c).is_some() || (BLOCKS_START..=BLOCKS_END).contains(&(c as u32))
}

/// The rectangles that make up `c` in a cell `w` by `h` device pixels, and
/// how opaque they are (less than 1 for the shades `░▒▓`). `None` for a
/// character this module does not draw.
pub fn rects(c: char, w: f32, h: f32) -> Option<(Vec<Rect>, f32)> {
    if let Some(arms) = arms(c) {
        return Some((line_rects(arms, w, h), 1.0));
    }
    block(c).map(|(parts, opacity)| {
        let scaled = parts.iter().map(|p| [(p[0] * w).round(), (p[1] * h).round(), (p[2] * w).round(), (p[3] * h).round()]);
        (scaled.collect(), opacity)
    })
}

fn line_rects(arms: &str, w: f32, h: f32) -> Vec<Rect> {
    let light = (w / LIGHT_DIVISOR).round().max(1.0);
    let thickness = |arm: u8| match arm {
        b'l' => light,
        b'h' => light * 2.0,
        _ => 0.0,
    };
    let [up, right, down, left] = [0, 1, 2, 3].map(|i| thickness(arms.as_bytes()[i]));
    // An arm runs from its edge to the far side of the thickest line it
    // crosses at the center, so corners have no notch.
    let (across, along) = (up.max(down), left.max(right));
    let column = |t: f32| ((w - t) / 2.0).round();
    let row = |t: f32| ((h - t) / 2.0).round();
    let mut rects = Vec::new();
    if up > 0.0 {
        rects.push([column(up), 0.0, column(up) + up, row(along) + along]);
    }
    if down > 0.0 {
        rects.push([column(down), row(along), column(down) + down, h]);
    }
    if left > 0.0 {
        rects.push([0.0, row(left), column(across) + across, row(left) + left]);
    }
    if right > 0.0 {
        rects.push([column(across), row(right), w, row(right) + right]);
    }
    rects
}

/// A block element as rectangles in fractions of the cell, and its opacity.
fn block(c: char) -> Option<(Vec<Rect>, f32)> {
    const EIGHTH: f32 = 0.125;
    let (ul, ur, ll, lr) = ([0.0, 0.0, 0.5, 0.5], [0.5, 0.0, 1.0, 0.5], [0.0, 0.5, 0.5, 1.0], [0.5, 0.5, 1.0, 1.0]);
    let full = [0.0, 0.0, 1.0, 1.0];
    let code = c as u32;
    let solid = |parts: Vec<Rect>| Some((parts, 1.0));
    match code {
        0x2580 => solid(vec![[0.0, 0.0, 1.0, 0.5]]),
        // ▁ to █: the lower 1/8 to 8/8.
        0x2581..=0x2588 => solid(vec![[0.0, 1.0 - EIGHTH * (code - 0x2580) as f32, 1.0, 1.0]]),
        // ▉ to ▏: the left 7/8 to 1/8.
        0x2589..=0x258F => solid(vec![[0.0, 0.0, EIGHTH * (0x2590 - code) as f32, 1.0]]),
        0x2590 => solid(vec![[0.5, 0.0, 1.0, 1.0]]),
        0x2591 => Some((vec![full], 0.25)),
        0x2592 => Some((vec![full], 0.5)),
        0x2593 => Some((vec![full], 0.75)),
        0x2594 => solid(vec![[0.0, 0.0, 1.0, EIGHTH]]),
        0x2595 => solid(vec![[1.0 - EIGHTH, 0.0, 1.0, 1.0]]),
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

    const W: f32 = 8.0;
    const H: f32 = 19.0;

    fn solid(c: char) -> Vec<Rect> {
        let (rects, opacity) = rects(c, W, H).unwrap();
        assert_eq!(opacity, 1.0);
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
        assert_eq!(solid('─'), [[0.0, 9.0, 4.0, 10.0], [4.0, 9.0, 8.0, 10.0]]);
        // Heavy: twice as thick, still edge to edge.
        assert_eq!(solid('━'), [[0.0, 9.0, 4.0, 11.0], [4.0, 9.0, 8.0, 11.0]]);
    }

    #[test]
    fn a_vertical_line_spans_the_cell() {
        let rects = solid('│');
        assert_eq!((rects[0][1], rects[1][3]), (0.0, H), "from the top edge to the bottom edge");
        assert!(rects.iter().all(|r| r[0] == 4.0 && r[2] == 5.0), "one pixel wide, centered");
        assert_eq!(rects[0][3], 10.0, "the halves meet");
        assert_eq!(rects[1][1], 10.0);
    }

    #[test]
    fn a_corner_has_no_notch() {
        // ┌: down and right. The down arm starts at the line's top, the
        // right arm at the down arm's left: together they cover the corner.
        assert_eq!(solid('┌'), [[4.0, 9.0, 5.0, 19.0], [4.0, 9.0, 8.0, 10.0]]);
    }

    #[test]
    fn every_line_character_touches_exactly_its_edges() {
        for offset in (0..80).chain(0x74..0x80) {
            let c = char::from_u32(0x2500 + offset).unwrap();
            let spec = arms(c).unwrap().as_bytes();
            let rects = solid(c);
            let touches = [
                rects.iter().any(|r| r[1] == 0.0 && r[3] - r[1] > r[2] - r[0]),
                rects.iter().any(|r| r[2] == W && r[2] - r[0] > r[3] - r[1]),
                rects.iter().any(|r| r[3] == H && r[3] - r[1] > r[2] - r[0]),
                rects.iter().any(|r| r[0] == 0.0 && r[2] - r[0] > r[3] - r[1]),
            ];
            for (arm, touch) in spec.iter().zip(touches) {
                assert_eq!(*arm != b'.', touch, "{c} ({})", std::str::from_utf8(spec).unwrap());
            }
            assert!(rects.iter().all(|r| r[0] >= 0.0 && r[1] >= 0.0 && r[2] <= W && r[3] <= H && r[0] < r[2] && r[1] < r[3]));
        }
    }

    #[test]
    fn half_blocks_meet_in_the_middle() {
        let (upper, lower) = (solid('▀'), solid('▄'));
        assert_eq!(upper, [[0.0, 0.0, 8.0, 10.0]]);
        assert_eq!(lower, [[0.0, 10.0, 8.0, 19.0]], "starts where the upper half ends");
        assert_eq!(solid('█'), [[0.0, 0.0, 8.0, 19.0]]);
        assert_eq!(solid('▌'), [[0.0, 0.0, 4.0, 19.0]]);
        assert_eq!(solid('▐'), [[4.0, 0.0, 8.0, 19.0]]);
    }

    #[test]
    fn eighths_grow_from_the_bottom_and_the_left() {
        assert_eq!(solid('▁'), [[0.0, 17.0, 8.0, 19.0]]);
        assert_eq!(solid('▇'), [[0.0, 2.0, 8.0, 19.0]]);
        assert_eq!(solid('▏'), [[0.0, 0.0, 1.0, 19.0]]);
        assert_eq!(solid('▉'), [[0.0, 0.0, 7.0, 19.0]]);
    }

    #[test]
    fn quadrants_and_shades() {
        assert_eq!(solid('▖'), [[0.0, 10.0, 4.0, 19.0]]);
        assert_eq!(solid('▚').len(), 2);
        assert_eq!(solid('▟').len(), 3);
        assert_eq!(rects('▒', W, H), Some((vec![[0.0, 0.0, 8.0, 19.0]], 0.5)));
    }
}
