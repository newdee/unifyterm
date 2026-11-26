//! What to paint for a screen: a snapshot and a theme in, lists of things
//! to draw out, all in cells. No window and no font is needed, so tests
//! check the result directly; the view turns cells into pixels.

use crate::graphics;
use crate::session::{Cell, Color, CursorShape, Focus, Snapshot, attr};
use crate::shine::Glow;
use crate::theme::{Rgb, Theme};

/// How far a dim color is moved toward its background.
const DIM_AMOUNT: f32 = 0.4;
/// Under this contrast (WCAG's ratio, 1 to 21) text is lost on what is
/// behind it. The light themes' white on their background is 1.1 to 1.3;
/// their dimmest color that is meant to be read is over 4.
const LEAST_SEEN: f64 = 2.0;

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

/// A character the window draws itself and not a font: a box-drawing or block character (`boxdraw`), a powerline separator (`powerline`).
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

/// Where an image shows: the cell of its top left corner (above or left of
/// the screen when part of it is), and the first and last cells of the
/// screen that show it, as (line, column).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ImageArea {
    pub(crate) id: u32,
    pub(crate) origin: (i64, i64),
    pub(crate) first: (usize, usize),
    pub(crate) last: (usize, usize),
}

#[derive(Clone, PartialEq, Debug, Default)]
pub(crate) struct Frame {
    pub(crate) fills: Vec<Fill>,
    pub(crate) runs: Vec<Run>,
    pub(crate) drawn: Vec<Drawn>,
    pub(crate) strokes: Vec<Stroke>,
    pub(crate) cursor: Option<CursorMark>,
    pub(crate) images: Vec<ImageArea>,
}

/// The image a placeholder's color names: 24 bits in a true color, 8 in one of the 256.
fn image_number(color: Color) -> Option<u32> {
    match color {
        Color::Rgb(Rgb(red, green, blue)) => Some(u32::from_be_bytes([0, red, green, blue])),
        Color::Indexed(index) => Some(u32::from(index)),
        Color::Foreground | Color::Background => None,
    }
}

/// The image, row and column a placeholder shows. What its marks leave out
/// comes from the placeholder left of it (`left`: its image, row, column),
/// as kitty's protocol says: the same row, the next column.
fn placeholder(cell: &Cell, left: Option<(u32, usize, usize)>) -> Option<(u32, usize, usize)> {
    let id = image_number(cell.fg)?;
    let marks: Vec<usize> = cell.combining.as_deref().unwrap_or("").chars().filter_map(graphics::number_of).collect();
    let follows =
        |row: Option<usize>| left.filter(|(was, was_row, _)| *was == id && row.is_none_or(|row| row == *was_row));
    let (row, column) = match (marks.first(), marks.get(1)) {
        (Some(&row), Some(&column)) => (row, column),
        (Some(&row), None) => (row, follows(Some(row)).map_or(0, |(_, _, column)| column + 1)),
        (None, _) => follows(None).map_or((0, 0), |(_, row, column)| (row, column + 1)),
    };
    // A third mark is the number's top byte.
    let id = marks.get(2).and_then(|top| u32::try_from(*top).ok()).map_or(id, |top| id | (top << 24));
    Some((id, row, column))
}

/// Adds a cell that shows (image, row, column) at (line, column) of the screen.
fn add_image_cell(images: &mut Vec<ImageArea>, (id, row, column): (u32, usize, usize), at: (usize, usize)) {
    let wide = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
    let origin = (wide(at.0) - wide(row), wide(at.1) - wide(column));
    match images.iter_mut().find(|area| area.id == id && area.origin == origin) {
        Some(area) => {
            area.first = (area.first.0.min(at.0), area.first.1.min(at.1));
            area.last = (area.last.0.max(at.0), area.last.1.max(at.1));
        }
        None => images.push(ImageArea { id, origin, first: at, last: at }),
    }
}

fn resolve(color: Color, theme: &Theme) -> Rgb {
    match color {
        Color::Foreground => theme.foreground,
        Color::Background => theme.background,
        Color::Indexed(index) => theme.indexed(index),
        Color::Rgb(rgb) => rgb,
    }
}

/// Whether `color` is white or bright white of the 16 colors.
const fn is_white(color: Color) -> bool {
    matches!(color, Color::Indexed(7 | 15))
}

/// A cell's text color and background as painted.
fn colors(cell: &Cell, theme: &Theme) -> (Rgb, Rgb) {
    let (mut fg, mut bg) = (resolve(cell.fg, theme), resolve(cell.bg, theme));
    if cell.has(attr::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }
    // Programs that take the screen for a dark one write plain words in white (PowerShell does,
    // for what is typed). A light theme's white is as light as its background, and cannot be made
    // dark: white on a color must stay light. So white on the theme's own background (or, inverse,
    // that background's color on white), where the two cannot be told apart, and only there, is
    // drawn in the theme's own text color. A dark theme's white shows on its background: nothing
    // changes there, and a cell with any other color behind it is left as the program asked.
    if is_white(cell.fg) && matches!(cell.bg, Color::Background) && fg.contrast(bg) < LEAST_SEEN {
        let (plain, behind) = (theme.foreground, theme.background);
        fg = if plain.contrast(bg) >= behind.contrast(bg) { plain } else { behind };
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
#[cfg(test)]
pub(crate) fn layout(snapshot: &Snapshot, theme: &Theme, focus: Focus) -> Frame {
    layout_lit(snapshot, theme, focus, &Glow::default())
}

/// How far toward white a cell's colors go at the brightest of the light (`prompt.shine`).
const LIGHT_LIFT: f32 = 0.55;
const WHITE: Rgb = Rgb(255, 255, 255);

/// The frame for `snapshot`, with the cells `glow` lights made brighter.
/// Without the keyboard's focus the cursor is a hollow box.
pub(crate) fn layout_lit(snapshot: &Snapshot, theme: &Theme, focus: Focus, glow: &Glow) -> Frame {
    let mut frame = Frame::default();
    for line in 0..snapshot.lines {
        // The text being collected, and the spaces seen since its last character.
        let mut open: Option<(Run, usize)> = None;
        // The placeholder just left, for the marks the next one leaves out.
        let mut left: Option<(u32, usize, usize)> = None;
        for (column, cell) in snapshot.row(line).iter().enumerate() {
            if cell.has(attr::SPACER) {
                continue;
            }
            if cell.c == graphics::PLACEHOLDER {
                left = placeholder(cell, left);
                if let Some(shown) = left {
                    add_image_cell(&mut frame.images, shown, (line, column));
                }
                if let Some((_, spaces)) = open.as_mut() {
                    *spaces += 1;
                }
                continue;
            }
            left = None;
            let (mut fg, mut bg) = colors(cell, theme);
            let light = glow.at(line, column);
            if light > 0.0 {
                fg = fg.toward(WHITE, light * LIGHT_LIFT);
                // The screen's own color behind a lit character stays: only a band's color is lit.
                if bg != theme.background {
                    bg = bg.toward(WHITE, light * LIGHT_LIFT);
                }
            }
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
            if crate::terminal::drawn_here(cell.c) {
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

    #[test]
    fn placeholders_make_one_image_area_and_draw_no_text() {
        let mut screen = crate::session::tests::quiet(10, 4);
        // Image 5: rows 1 and 2 (row 0 scrolled away above), columns 0-2; the second row's
        // marks leave the column out after the first cell: it follows from the left.
        let (one, two, zero) = ('\u{030D}', '\u{030E}', '\u{0305}');
        let row1 = format!("\x1b[38;2;0;0;5m\u{10EEEE}{one}{zero}\u{10EEEE}{one}{one}\u{10EEEE}{one}{two}\x1b[39m");
        let row2 = format!("\x1b[38;2;0;0;5m\u{10EEEE}{two}{zero}\u{10EEEE}{two}\u{10EEEE}\x1b[39mx");
        screen.feed(format!("{row1}\r\n{row2}").as_bytes());
        let frame = layout(&screen.snapshot(), &Theme::default(), Focus::Focused);
        assert_eq!(frame.images, [ImageArea { id: 5, origin: (-1, 0), first: (0, 0), last: (1, 2) }]);
        assert_eq!(
            frame.runs.iter().map(|run| run.text.as_str()).collect::<Vec<_>>(),
            ["x"],
            "no glyph for a placeholder"
        );
    }
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
    fn white_that_is_lost_on_what_is_behind_it_is_drawn_in_the_themes_text_color() {
        // The light theme "day": its white and bright white are as light as its background.
        let mut light = Theme::default();
        (light.foreground, light.background) = (Rgb(0x14, 0x25, 0x3a), Rgb(0xf4, 0xf9, 0xfe));
        (light.ansi[4], light.ansi[7], light.ansi[15]) =
            (Rgb(0, 0x4d, 0x9f), Rgb(0xd9, 0xe3, 0xec), Rgb(0xf4, 0xf9, 0xfe));
        let drawn = |bytes: &str, theme: &Theme| {
            let mut session = quiet(20, 3);
            session.feed(b"\x1b[H\x1b[2J");
            session.feed(bytes.as_bytes());
            let frame = layout(&session.snapshot(), theme, Focus::Focused);
            frame.runs.iter().map(|run| (run.text.clone(), run.color)).collect::<Vec<_>>()
        };
        // White and bright white on the background; white on blue; blue; white behind the background's color.
        let bytes = "\x1b[37ma\x1b[97mb\x1b[37;44mc\x1b[0;34md\x1b[0;7;37me";
        assert_eq!(
            drawn(bytes, &light),
            [
                ("ab".to_owned(), light.foreground),
                ("c".to_owned(), light.ansi[7]),
                ("d".to_owned(), light.ansi[4]),
                ("e".to_owned(), light.foreground),
            ],
            "white lost on the light background is the text's color; white on blue stays white"
        );
        let dark = Theme::default();
        assert_eq!(
            drawn(bytes, &dark),
            [
                ("a".to_owned(), dark.ansi[7]),
                ("b".to_owned(), dark.ansi[15]),
                ("c".to_owned(), dark.ansi[7]),
                ("d".to_owned(), dark.ansi[4]),
                ("e".to_owned(), dark.background),
            ],
            "on a dark theme white can be seen: nothing changes"
        );
        // Black on a dark background is not white: left as programs have always found it.
        assert_eq!(drawn("\x1b[30mx", &dark), [("x".to_owned(), dark.ansi[0])]);
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
