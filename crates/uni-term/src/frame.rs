//! What to paint for a screen: a snapshot and a theme in, lists of things
//! to draw out, all in cells. No window and no font is needed, so tests
//! check the result directly; the view turns cells into pixels.

use crate::boxdraw;
use crate::session::{Cell, Color, CursorShape, Snapshot, attr};
use crate::theme::{Rgb, Theme};

/// How far a dim color is moved toward its background.
const DIM_AMOUNT: f32 = 0.4;

/// A background behind `cells` cells of one line.
#[derive(Clone, PartialEq, Debug)]
pub struct Fill {
    pub line: usize,
    pub column: usize,
    pub cells: usize,
    pub color: Rgb,
}

/// Text in one style, starting at a cell.
#[derive(Clone, PartialEq, Debug)]
pub struct Text {
    pub line: usize,
    pub column: usize,
    pub cells: usize,
    pub text: String,
    pub color: Rgb,
    pub bold: bool,
    pub italic: bool,
    /// One character a cell: each glyph is put at its own cell, whatever
    /// width the font gives it. Not so for a wide character or one with
    /// combining marks, which is a `Text` by itself.
    pub grid: bool,
}

/// A box-drawing or block character, drawn by `boxdraw` and not a font.
#[derive(Clone, PartialEq, Debug)]
pub struct Drawn {
    pub line: usize,
    pub column: usize,
    pub c: char,
    pub color: Rgb,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StrokeKind {
    Underline,
    Strike,
}

/// A line under or through `cells` cells.
#[derive(Clone, PartialEq, Debug)]
pub struct Stroke {
    pub line: usize,
    pub column: usize,
    pub cells: usize,
    pub kind: StrokeKind,
    pub color: Rgb,
}

#[derive(Clone, PartialEq, Debug)]
pub struct CursorMark {
    pub line: usize,
    pub column: usize,
    /// 2 on a wide character.
    pub cells: usize,
    pub shape: CursorShape,
    pub color: Rgb,
    /// For a block: the character under it, to draw again in `over_color`
    /// so it shows on the block.
    pub over: Option<String>,
    pub over_color: Rgb,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Frame {
    pub fills: Vec<Fill>,
    pub texts: Vec<Text>,
    pub drawn: Vec<Drawn>,
    pub strokes: Vec<Stroke>,
    pub cursor: Option<CursorMark>,
}

fn resolve(color: Color, theme: &Theme) -> Rgb {
    match color {
        Color::Foreground => theme.foreground,
        Color::Background => theme.background,
        Color::Indexed(index) => theme.indexed(index),
        Color::Rgb(rgb) => rgb,
    }
}

/// A cell's text color and background as painted.
fn colors(cell: &Cell, theme: &Theme) -> (Rgb, Rgb) {
    let (mut fg, mut bg) = (resolve(cell.fg, theme), resolve(cell.bg, theme));
    if cell.has(attr::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }
    if cell.has(attr::DIM) {
        fg = fg.toward(bg, DIM_AMOUNT);
    }
    if cell.has(attr::SELECTED) {
        bg = theme.selection;
    }
    if cell.has(attr::HIDDEN) {
        fg = bg;
    }
    (fg, bg)
}

fn glyphs(cell: &Cell) -> String {
    let mut text = String::from(cell.c);
    text.extend(cell.combining.as_deref());
    text
}

/// Nothing of a font's to draw: a space, or text the program hid.
fn blank(cell: &Cell) -> bool {
    cell.has(attr::HIDDEN) || (matches!(cell.c, ' ' | '\0') && cell.combining.is_none())
}

/// Grows the last item when `next` continues it on the same line, else adds `next`.
fn extend<T>(items: &mut Vec<T>, next: T, continues: impl Fn(&T, &T) -> bool, grow: impl Fn(&mut T, &T)) {
    match items.last_mut() {
        Some(last) if continues(last, &next) => grow(last, &next),
        _ => items.push(next),
    }
}

/// The frame for `snapshot`. `focused`: the window has the keyboard; when
/// it does not, the cursor is a hollow box.
pub fn layout(snapshot: &Snapshot, theme: &Theme, focused: bool) -> Frame {
    let mut frame = Frame::default();
    for line in 0..snapshot.lines {
        // The text being collected, and the spaces seen since its last character.
        let mut open: Option<(Text, usize)> = None;
        for (column, cell) in snapshot.row(line).iter().enumerate() {
            if cell.has(attr::SPACER) {
                continue;
            }
            let (fg, bg) = colors(cell, theme);
            let cells = if cell.has(attr::WIDE) { 2 } else { 1 };
            if bg != theme.background {
                let fill = Fill { line, column, cells, color: bg };
                let adjoins = |a: &Fill, b: &Fill| a.line == b.line && a.column + a.cells == b.column && a.color == b.color;
                extend(&mut frame.fills, fill, adjoins, |a, b| a.cells += b.cells);
            }
            for (attribute, kind) in [(attr::UNDERLINE, StrokeKind::Underline), (attr::STRIKE, StrokeKind::Strike)] {
                if cell.has(attribute) {
                    let stroke = Stroke { line, column, cells, kind, color: fg };
                    let adjoins = |a: &Stroke, b: &Stroke| {
                        a.line == b.line && a.column + a.cells == b.column && a.color == b.color && a.kind == b.kind
                    };
                    extend(&mut frame.strokes, stroke, adjoins, |a, b| a.cells += b.cells);
                }
            }
            if blank(cell) {
                if let Some((_, spaces)) = open.as_mut() {
                    *spaces += 1;
                }
                continue;
            }
            let (bold, italic) = (cell.has(attr::BOLD), cell.has(attr::ITALIC));
            if boxdraw::draws(cell.c) {
                frame.texts.extend(open.take().map(|(text, _)| text));
                frame.drawn.push(Drawn { line, column, c: cell.c, color: fg });
            } else if cells == 2 || cell.combining.is_some() {
                frame.texts.extend(open.take().map(|(text, _)| text));
                frame.texts.push(Text { line, column, cells, text: glyphs(cell), color: fg, bold, italic, grid: false });
            } else {
                match open.as_mut() {
                    Some((text, spaces)) if (text.color, text.bold, text.italic) == (fg, bold, italic) => {
                        text.text.extend(std::iter::repeat_n(' ', *spaces));
                        text.text.push(cell.c);
                        text.cells = column + 1 - text.column;
                        *spaces = 0;
                    }
                    _ => {
                        frame.texts.extend(open.take().map(|(text, _)| text));
                        let text = Text { line, column, cells: 1, text: cell.c.into(), color: fg, bold, italic, grid: true };
                        open = Some((text, 0));
                    }
                }
            }
        }
        frame.texts.extend(open.take().map(|(text, _)| text));
    }
    // A cursor outside the screen is not drawn (a snapshot does not have
    // one; painting must not fail on a snapshot that does).
    frame.cursor = snapshot.cursor.filter(|cursor| cursor.line < snapshot.lines && cursor.column < snapshot.columns).map(|cursor| {
        let row = snapshot.row(cursor.line);
        // On the second cell of a wide character: the cursor covers both.
        let column = if row[cursor.column].has(attr::SPACER) { cursor.column.saturating_sub(1) } else { cursor.column };
        let cell = &row[column];
        let shape = if focused { cursor.shape } else { CursorShape::Hollow };
        CursorMark {
            line: cursor.line,
            column,
            cells: if cell.has(attr::WIDE) { 2 } else { 1 },
            shape,
            color: theme.cursor,
            over: (shape == CursorShape::Block && !blank(cell)).then(|| glyphs(cell)),
            over_color: theme.background,
        }
    });
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::tests::quiet;

    fn frame_of(bytes: &str, focused: bool) -> Frame {
        let mut session = quiet(20, 3);
        session.feed(b"\x1b[H\x1b[2J");
        session.feed(bytes.as_bytes());
        layout(&session.snapshot(), &Theme::default(), focused)
    }

    #[test]
    fn plain_text_is_one_run_a_line() {
        let frame = frame_of("ab cd\r\nxy", true);
        let theme = Theme::default();
        assert_eq!(frame.texts.len(), 2);
        let first = &frame.texts[0];
        assert_eq!((first.line, first.column, first.cells, first.text.as_str()), (0, 0, 5, "ab cd"));
        assert!(first.grid && !first.bold && first.color == theme.foreground);
        assert_eq!((frame.texts[1].line, frame.texts[1].text.as_str()), (1, "xy"));
        assert!(frame.fills.is_empty() && frame.strokes.is_empty() && frame.drawn.is_empty());
    }

    #[test]
    fn a_change_of_style_starts_a_new_run() {
        let frame = frame_of("ab\x1b[1mcd\x1b[0;31mef\x1b[0m  gh", true);
        let runs: Vec<_> = frame.texts.iter().map(|t| (t.column, t.text.as_str(), t.bold)).collect();
        assert_eq!(runs, [(0, "ab", false), (2, "cd", true), (4, "ef", false), (8, "gh", false)]);
        assert_eq!(frame.texts[2].color, Theme::default().ansi[1]);
    }

    #[test]
    fn trailing_spaces_are_not_text() {
        let frame = frame_of("ab   ", true);
        assert_eq!((frame.texts[0].text.as_str(), frame.texts[0].cells), ("ab", 2));
    }

    #[test]
    fn a_wide_character_is_text_by_itself() {
        let frame = frame_of("a中b", true);
        let runs: Vec<_> = frame.texts.iter().map(|t| (t.column, t.cells, t.text.as_str(), t.grid)).collect();
        assert_eq!(runs, [(0, 1, "a", true), (1, 2, "中", false), (3, 1, "b", true)]);
    }

    #[test]
    fn combining_marks_stay_with_their_character() {
        let frame = frame_of("xe\u{301}y", true);
        let runs: Vec<_> = frame.texts.iter().map(|t| (t.column, t.text.as_str(), t.grid)).collect();
        assert_eq!(runs, [(0, "x", true), (1, "e\u{301}", false), (2, "y", true)]);
    }

    #[test]
    fn backgrounds_join_along_a_line() {
        let frame = frame_of("\x1b[41mab\x1b[42mc\x1b[0md\x1b[41me", true);
        let theme = Theme::default();
        let fills: Vec<_> = frame.fills.iter().map(|f| (f.column, f.cells, f.color)).collect();
        assert_eq!(fills, [(0, 2, theme.ansi[1]), (2, 1, theme.ansi[2]), (4, 1, theme.ansi[1])]);
    }

    #[test]
    fn inverse_swaps_and_dim_fades() {
        let theme = Theme::default();
        let frame = frame_of("\x1b[7ma\x1b[0;2mb", true);
        assert_eq!(frame.fills[0].color, theme.foreground);
        assert_eq!(frame.texts[0].color, theme.background);
        assert_eq!(frame.texts[1].color, theme.foreground.toward(theme.background, DIM_AMOUNT));
    }

    #[test]
    fn hidden_text_is_not_drawn() {
        let frame = frame_of("a\x1b[8mbc\x1b[0md", true);
        let runs: Vec<_> = frame.texts.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(runs, ["a  d"], "the hidden cells are spaces in the run");
    }

    #[test]
    fn underline_and_strike_are_strokes() {
        let frame = frame_of("\x1b[4mab \x1b[0;9mc", true);
        let strokes: Vec<_> = frame.strokes.iter().map(|s| (s.column, s.cells, s.kind)).collect();
        assert_eq!(strokes, [(0, 3, StrokeKind::Underline), (3, 1, StrokeKind::Strike)], "an underlined space counts");
    }

    #[test]
    fn box_characters_are_drawn_not_typed() {
        let frame = frame_of("a│b▀", true);
        let drawn: Vec<_> = frame.drawn.iter().map(|d| (d.column, d.c)).collect();
        assert_eq!(drawn, [(1, '│'), (3, '▀')]);
        let runs: Vec<_> = frame.texts.iter().map(|t| (t.column, t.text.as_str())).collect();
        assert_eq!(runs, [(0, "a"), (2, "b")]);
    }

    #[test]
    fn the_cursor_is_a_block_with_its_character_on_top() {
        let theme = Theme::default();
        let frame = frame_of("ab\x1b[1D", true);
        let cursor = frame.cursor.unwrap();
        assert_eq!((cursor.line, cursor.column, cursor.cells, cursor.shape), (0, 1, 1, CursorShape::Block));
        assert_eq!((cursor.over.as_deref(), cursor.over_color, cursor.color), (Some("b"), theme.background, theme.cursor));
        let at_end = frame_of("ab", true).cursor.unwrap();
        assert_eq!((at_end.column, at_end.over), (2, None), "nothing under it");
    }

    #[test]
    fn the_cursor_is_hollow_without_focus_and_absent_when_hidden() {
        assert_eq!(frame_of("a", false).cursor.unwrap().shape, CursorShape::Hollow);
        assert_eq!(frame_of("a\x1b[?25l", true).cursor, None);
        assert_eq!(frame_of("a\x1b[6 q", true).cursor.unwrap().shape, CursorShape::Beam);
    }

    #[test]
    fn the_cursor_covers_a_whole_wide_character() {
        let cursor = frame_of("中\x1b[1D", true).cursor.unwrap();
        assert_eq!((cursor.column, cursor.cells, cursor.over.as_deref()), (0, 2, Some("中")));
    }

    #[test]
    fn a_cursor_outside_the_screen_is_not_drawn() {
        let mut snapshot = quiet(20, 3).snapshot();
        for (line, column) in [(3, 0), (0, 20), (99, 99)] {
            snapshot.cursor = Some(crate::session::Cursor { line, column, shape: CursorShape::Block });
            assert_eq!(layout(&snapshot, &Theme::default(), true).cursor, None, "({line}, {column})");
        }
        snapshot.cursor = Some(crate::session::Cursor { line: 2, column: 19, shape: CursorShape::Block });
        assert!(layout(&snapshot, &Theme::default(), true).cursor.is_some(), "the last cell is inside");
    }

    #[test]
    fn selected_cells_get_the_selection_color() {
        let mut session = quiet(20, 3);
        session.feed(b"\x1b[H\x1b[2Jhello");
        session.select(0, 1, false, crate::session::SelectBy::Cell);
        session.select_to(0, 3, true);
        let theme = Theme::default();
        let frame = layout(&session.snapshot(), &theme, true);
        assert_eq!(frame.fills, [Fill { line: 0, column: 1, cells: 3, color: theme.selection }]);
        assert_eq!(frame.texts.len(), 1, "the text's color does not change, so it stays one run");
    }
}
