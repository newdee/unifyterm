//! What to paint for a screen: a snapshot and a theme in, lists of things
//! to draw out, all in cells. No window and no font is needed, so tests
//! check the result directly; the view turns cells into pixels.

use crate::boxdraw;
use crate::session::{Cell, Color, CursorShape, Focus, Snapshot, attr};
use crate::theme::{Rgb, Theme};

/// How far a dim color is moved toward its background.
const DIM_AMOUNT: f32 = 0.4;

/// A background behind `cells` cells of one line.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Fill {
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) cells: usize,
    pub(crate) color: Rgb,
}

/// A run of text in one style, starting at a cell.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Run {
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) cells: usize,
    pub(crate) text: String,
    pub(crate) color: Rgb,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    /// One character a cell: each glyph is put at its own cell, whatever
    /// width the font gives it. Not so for a wide character or one with
    /// combining marks, which is a `Run` by itself.
    pub(crate) grid: bool,
}

/// A box-drawing or block character, drawn by `boxdraw` and not a font.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Drawn {
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) c: char,
    pub(crate) color: Rgb,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StrokeKind {
    Underline,
    Strike,
}

/// A line under or through `cells` cells.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Stroke {
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) cells: usize,
    pub(crate) kind: StrokeKind,
    pub(crate) color: Rgb,
}

#[derive(Clone, PartialEq, Debug)]
pub(crate) struct CursorMark {
    pub(crate) line: usize,
    pub(crate) column: usize,
    /// 2 on a wide character.
    pub(crate) cells: usize,
    pub(crate) shape: CursorShape,
    pub(crate) color: Rgb,
    /// For a block: the character under it, to draw again in `over_color`
    /// so it shows on the block.
    pub(crate) over: Option<String>,
    pub(crate) over_color: Rgb,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub(crate) struct Frame {
    pub(crate) fills: Vec<Fill>,
    pub(crate) runs: Vec<Run>,
    pub(crate) drawn: Vec<Drawn>,
    pub(crate) strokes: Vec<Stroke>,
    pub(crate) cursor: Option<CursorMark>,
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

/// The frame for `snapshot`. Without the keyboard's focus the cursor is a
/// hollow box.
pub(crate) fn layout(snapshot: &Snapshot, theme: &Theme, focus: Focus) -> Frame {
    let mut frame = Frame::default();
    for line in 0..snapshot.lines {
        // The text being collected, and the spaces seen since its last character.
        let mut open: Option<(Run, usize)> = None;
        for (column, cell) in snapshot.row(line).iter().enumerate() {
            if cell.has(attr::SPACER) {
                continue;
            }
            let (fg, bg) = colors(cell, theme);
            let cells = if cell.has(attr::WIDE) { 2 } else { 1 };
            if bg != theme.background {
                let fill = Fill { line, column, cells, color: bg };
                let adjoins =
                    |a: &Fill, b: &Fill| a.line == b.line && a.column + a.cells == b.column && a.color == b.color;
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
                frame.runs.extend(open.take().map(|(text, _)| text));
                frame.drawn.push(Drawn { line, column, c: cell.c, color: fg });
            } else if cells == 2 || cell.combining.is_some() {
                frame.runs.extend(open.take().map(|(text, _)| text));
                frame.runs.push(Run { line, column, cells, text: glyphs(cell), color: fg, bold, italic, grid: false });
            } else {
                match open.as_mut() {
                    Some((text, spaces)) if (text.color, text.bold, text.italic) == (fg, bold, italic) => {
                        text.text.extend(std::iter::repeat_n(' ', *spaces));
                        text.text.push(cell.c);
                        text.cells = column + 1 - text.column;
                        *spaces = 0;
                    }
                    _ => {
                        frame.runs.extend(open.take().map(|(text, _)| text));
                        let text =
                            Run { line, column, cells: 1, text: cell.c.into(), color: fg, bold, italic, grid: true };
                        open = Some((text, 0));
                    }
                }
            }
        }
        frame.runs.extend(open.take().map(|(text, _)| text));
    }
    // A cursor outside the screen is not drawn (a snapshot does not have
    // one; painting must not fail on a snapshot that does).
    frame.cursor = snapshot
        .cursor
        .filter(|cursor| cursor.line < snapshot.lines && cursor.column < snapshot.columns)
        .map(|cursor| {
            let row = snapshot.row(cursor.line);
            // On the second cell of a wide character: the cursor covers both.
            let column =
                if row[cursor.column].has(attr::SPACER) { cursor.column.saturating_sub(1) } else { cursor.column };
            let cell = &row[column];
            let shape = if focus == Focus::Focused { cursor.shape } else { CursorShape::Hollow };
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

    fn frame_of(bytes: &str, focus: Focus) -> Frame {
        let mut session = quiet(20, 3);
        session.feed(b"\x1b[H\x1b[2J");
        session.feed(bytes.as_bytes());
        layout(&session.snapshot(), &Theme::default(), focus)
    }

    #[test]
    fn plain_text_is_one_run_a_line() {
        let frame = frame_of("ab cd\r\nxy", Focus::Focused);
        let theme = Theme::default();
        assert_eq!(frame.runs.len(), 2);
        let first = &frame.runs[0];
        assert_eq!((first.line, first.column, first.cells, first.text.as_str()), (0, 0, 5, "ab cd"));
        assert!(first.grid && !first.bold && first.color == theme.foreground);
        assert_eq!((frame.runs[1].line, frame.runs[1].text.as_str()), (1, "xy"));
        assert!(frame.fills.is_empty() && frame.strokes.is_empty() && frame.drawn.is_empty());
    }

    #[test]
    fn a_change_of_style_starts_a_new_run() {
        let frame = frame_of("ab\x1b[1mcd\x1b[0;31mef\x1b[0m  gh", Focus::Focused);
        let runs: Vec<_> = frame.runs.iter().map(|t| (t.column, t.text.as_str(), t.bold)).collect();
        assert_eq!(runs, [(0, "ab", false), (2, "cd", true), (4, "ef", false), (8, "gh", false)]);
        assert_eq!(frame.runs[2].color, Theme::default().ansi[1]);
    }

    #[test]
    fn trailing_spaces_are_not_text() {
        let frame = frame_of("ab   ", Focus::Focused);
        assert_eq!((frame.runs[0].text.as_str(), frame.runs[0].cells), ("ab", 2));
    }

    #[test]
    fn a_wide_character_is_text_by_itself() {
        let frame = frame_of("a中b", Focus::Focused);
        let runs: Vec<_> = frame.runs.iter().map(|t| (t.column, t.cells, t.text.as_str(), t.grid)).collect();
        assert_eq!(runs, [(0, 1, "a", true), (1, 2, "中", false), (3, 1, "b", true)]);
    }

    #[test]
    fn combining_marks_stay_with_their_character() {
        let frame = frame_of("xe\u{301}y", Focus::Focused);
        let runs: Vec<_> = frame.runs.iter().map(|t| (t.column, t.text.as_str(), t.grid)).collect();
        assert_eq!(runs, [(0, "x", true), (1, "e\u{301}", false), (2, "y", true)]);
    }

    #[test]
    fn backgrounds_join_along_a_line() {
        let frame = frame_of("\x1b[41mab\x1b[42mc\x1b[0md\x1b[41me", Focus::Focused);
        let theme = Theme::default();
        let fills: Vec<_> = frame.fills.iter().map(|f| (f.column, f.cells, f.color)).collect();
        assert_eq!(fills, [(0, 2, theme.ansi[1]), (2, 1, theme.ansi[2]), (4, 1, theme.ansi[1])]);
    }

    #[test]
    fn inverse_swaps_and_dim_fades() {
        let theme = Theme::default();
        let frame = frame_of("\x1b[7ma\x1b[0;2mb", Focus::Focused);
        assert_eq!(frame.fills[0].color, theme.foreground);
        assert_eq!(frame.runs[0].color, theme.background);
        assert_eq!(frame.runs[1].color, theme.foreground.toward(theme.background, DIM_AMOUNT));
    }

    #[test]
    fn hidden_text_is_not_drawn() {
        let frame = frame_of("a\x1b[8mbc\x1b[0md", Focus::Focused);
        let runs: Vec<_> = frame.runs.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(runs, ["a  d"], "the hidden cells are spaces in the run");
    }

    #[test]
    fn underline_and_strike_are_strokes() {
        let frame = frame_of("\x1b[4mab \x1b[0;9mc", Focus::Focused);
        let strokes: Vec<_> = frame.strokes.iter().map(|s| (s.column, s.cells, s.kind)).collect();
        assert_eq!(strokes, [(0, 3, StrokeKind::Underline), (3, 1, StrokeKind::Strike)], "an underlined space counts");
    }

    #[test]
    fn box_characters_are_drawn_not_typed() {
        let frame = frame_of("a│b▀", Focus::Focused);
        let drawn: Vec<_> = frame.drawn.iter().map(|d| (d.column, d.c)).collect();
        assert_eq!(drawn, [(1, '│'), (3, '▀')]);
        let runs: Vec<_> = frame.runs.iter().map(|t| (t.column, t.text.as_str())).collect();
        assert_eq!(runs, [(0, "a"), (2, "b")]);
    }

    #[test]
    fn the_cursor_is_a_block_with_its_character_on_top() {
        let theme = Theme::default();
        let frame = frame_of("ab\x1b[1D", Focus::Focused);
        let cursor = frame.cursor.unwrap();
        assert_eq!((cursor.line, cursor.column, cursor.cells, cursor.shape), (0, 1, 1, CursorShape::Block));
        assert_eq!(
            (cursor.over.as_deref(), cursor.over_color, cursor.color),
            (Some("b"), theme.background, theme.cursor)
        );
        let at_end = frame_of("ab", Focus::Focused).cursor.unwrap();
        assert_eq!((at_end.column, at_end.over), (2, None), "nothing under it");
    }

    #[test]
    fn the_cursor_is_hollow_without_focus_and_absent_when_hidden() {
        assert_eq!(frame_of("a", Focus::Unfocused).cursor.unwrap().shape, CursorShape::Hollow);
        assert_eq!(frame_of("a\x1b[?25l", Focus::Focused).cursor, None);
        assert_eq!(frame_of("a\x1b[6 q", Focus::Focused).cursor.unwrap().shape, CursorShape::Beam);
    }

    #[test]
    fn the_cursor_covers_a_whole_wide_character() {
        let cursor = frame_of("中\x1b[1D", Focus::Focused).cursor.unwrap();
        assert_eq!((cursor.column, cursor.cells, cursor.over.as_deref()), (0, 2, Some("中")));
    }

    #[test]
    fn a_cursor_outside_the_screen_is_not_drawn() {
        let mut snapshot = quiet(20, 3).snapshot();
        for (line, column) in [(3, 0), (0, 20), (99, 99)] {
            snapshot.cursor = Some(crate::session::Cursor { line, column, shape: CursorShape::Block });
            assert_eq!(layout(&snapshot, &Theme::default(), Focus::Focused).cursor, None, "({line}, {column})");
        }
        snapshot.cursor = Some(crate::session::Cursor { line: 2, column: 19, shape: CursorShape::Block });
        assert!(layout(&snapshot, &Theme::default(), Focus::Focused).cursor.is_some(), "the last cell is inside");
    }

    #[test]
    fn selected_cells_get_the_selection_color() {
        let mut session = quiet(20, 3);
        session.feed(b"\x1b[H\x1b[2Jhello");
        session.select(0, 1, crate::session::Half::Left, crate::session::SelectBy::Cell);
        session.select_to(0, 3, crate::session::Half::Right);
        let theme = Theme::default();
        let frame = layout(&session.snapshot(), &theme, Focus::Focused);
        assert_eq!(frame.fills, [Fill { line: 0, column: 1, cells: 3, color: theme.selection }]);
        assert_eq!(frame.runs.len(), 1, "the text's color does not change, so it stays one run");
    }
}
