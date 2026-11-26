//! The prompt put together: [`Prompt`] (how it looks) and [`Facts`] (what it
//! is about) give [`Rendered`], cells with their colors worked out. No shell
//! is known here: `escape` writes the cells the way one shell takes them.
//!
//! The separators of `classic` and `rainbow` are powerline's characters
//! (U+E0B0 to U+E0BD). UnifyTerm draws them itself; in a terminal that does
//! not, they need a font that has them.

use std::fmt::Write as _;
use std::time::Duration;

use unicode_width::UnicodeWidthChar as _;
use unifyterm_config::prompt::{Gradient, Lines, Paint, Palette, Prompt, Segment, Separator, Style, Worktree};

use super::facts::{Facts, Head};

/// One character of the prompt and its colors; `None`: the terminal's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cell {
    pub(crate) text: char,
    pub(crate) fg: Option<[u8; 3]>,
    pub(crate) bg: Option<[u8; 3]>,
}

impl Cell {
    const fn blank() -> Self {
        Self { text: ' ', fg: None, bg: None }
    }
}

/// The prompt, ready to be written: its lines (the last ends where the input
/// begins), and what goes in the shell's own place at the line's end (zsh's
/// `RPROMPT`, fish's right prompt), empty for the other shells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rendered {
    pub(crate) lines: Vec<Vec<Cell>>,
    pub(crate) right: Vec<Cell>,
}

/// Which side a run of segments is on: the shapes between them point away from the middle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

/// A separator: the full shape (the segment's color into the next), or the thin line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Weight {
    Solid,
    Thin,
}

/// The powerline character for a separator.
const fn glyph(separator: Separator, side: Side, weight: Weight) -> char {
    match (separator, side, weight) {
        (Separator::Sharp, Side::Left, Weight::Solid) => '\u{E0B0}',
        (Separator::Sharp, Side::Left, Weight::Thin) => '\u{E0B1}',
        (Separator::Sharp, Side::Right, Weight::Solid) => '\u{E0B2}',
        (Separator::Sharp, Side::Right, Weight::Thin) => '\u{E0B3}',
        (Separator::Round, Side::Left, Weight::Solid) => '\u{E0B4}',
        (Separator::Round, Side::Left, Weight::Thin) => '\u{E0B5}',
        (Separator::Round, Side::Right, Weight::Solid) => '\u{E0B6}',
        (Separator::Round, Side::Right, Weight::Thin) => '\u{E0B7}',
        // The upper-left triangle after a segment, the lower-right one before it: a "/" either way.
        (Separator::Slant, Side::Left, Weight::Solid) => '\u{E0BC}',
        (Separator::Slant, Side::Left, Weight::Thin) => '\u{E0BD}',
        (Separator::Slant, Side::Right, Weight::Solid) => '\u{E0BA}',
        (Separator::Slant, Side::Right, Weight::Thin) => '\u{E0BB}',
    }
}

/// How far from the bar toward the text the thin lines of `classic` are.
const THIN_LINE_AMOUNT: f32 = 0.45;
/// A command that took less than this has no duration shown.
const SHOWN_FROM: Duration = Duration::from_secs(2);
const SECONDS_IN_MINUTE: u64 = 60;
const SECONDS_IN_HOUR: u64 = 3600;

/// A segment that has something to say, with what it says and its own colors.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Piece {
    text: String,
    paints: Vec<[u8; 3]>,
}

/// How long a command took, as a person reads it: `38s`, `4m05s`, `2h03m`.
fn took(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < SECONDS_IN_MINUTE {
        format!("{seconds}s")
    } else if seconds < SECONDS_IN_HOUR {
        format!("{}m{:02}s", seconds / SECONDS_IN_MINUTE, seconds % SECONDS_IN_MINUTE)
    } else {
        format!("{}h{:02}m", seconds / SECONDS_IN_HOUR, seconds % SECONDS_IN_HOUR / SECONDS_IN_MINUTE)
    }
}

/// The directory as the prompt shows it: the home as `~`, `/` between the parts.
fn directory(facts: &Facts) -> String {
    let slashed = |path: &std::path::Path| path.display().to_string().replace('\\', "/");
    if let Some(home) = &facts.home
        && let Ok(rest) = facts.cwd.strip_prefix(home)
    {
        let rest = slashed(rest);
        return if rest.is_empty() { "~".to_owned() } else { format!("~/{rest}") };
    }
    slashed(&facts.cwd)
}

/// What `segment` says, in `style`; `None` when it has nothing to say (no repository, no failure).
fn text(segment: Segment, facts: &Facts, style: Style) -> Option<String> {
    let plain = style == Style::Plain;
    match segment {
        Segment::Dir => Some(directory(facts)),
        Segment::Git => {
            let repo = facts.repo.as_ref()?;
            let name = match &repo.head {
                Head::Branch(name) => name.clone(),
                Head::Detached(commit) => format!("@{commit}"),
            };
            let changes = repo.changes.unwrap_or_default();
            if plain {
                return Some(format!("({name}{})", if changes.any() { "*" } else { "" }));
            }
            let mut said = name;
            for (sign, count) in [
                ('+', changes.staged),
                ('!', changes.changed),
                ('?', changes.untracked),
                ('↑', changes.ahead),
                ('↓', changes.behind),
            ] {
                if count > 0 {
                    let _ = write!(said, " {sign}{count}");
                }
            }
            Some(said)
        }
        Segment::Status => facts
            .status
            .filter(|code| *code != 0)
            .map(|code| if plain { format!("exit {code}") } else { format!("✘ {code}") }),
        Segment::Duration => facts.took.filter(|duration| *duration >= SHOWN_FROM).map(took),
        Segment::Venv => facts.venv.as_ref().map(|name| if plain { format!("({name})") } else { format!("py {name}") }),
        Segment::Time => {
            let super::facts::Clock(hours, minutes, seconds) = facts.now;
            Some(format!("{hours:02}:{minutes:02}:{seconds:02}"))
        }
    }
}

/// `text` with each control character (a line break in a folder's name, an escape) shown as `?`:
/// written into the prompt as they are, they would break its lines or be taken as sequences.
fn printable(text: &str) -> String {
    text.chars().map(|c| if c.is_control() { '?' } else { c }).collect()
}

/// The segments of one side that have something to say, each with its own colors.
fn pieces(segments: &[Segment], prompt: &Prompt, palette: &Palette, facts: &Facts) -> Vec<Piece> {
    let worktree = match facts.repo.as_ref().and_then(|repo| repo.changes) {
        Some(changes) if changes.any() => Worktree::Changed,
        _ => Worktree::Clean,
    };
    segments
        .iter()
        .filter_map(|segment| {
            let text = printable(&text(*segment, facts, prompt.style)?);
            let paints = prompt.paints.of(*segment, worktree).iter().map(|paint| paint.rgb(&palette.ansi)).collect();
            Some(Piece { text, paints })
        })
        .collect()
}

/// The color `at` (0 to 1) of the way along `stops`.
fn along(stops: &[[u8; 3]], at: f32) -> [u8; 3] {
    match stops {
        [] => [0, 0, 0],
        [only] => *only,
        _ => {
            let last = stops.len() - 1;
            let place = at.clamp(0.0, 1.0) * f32::from(u16::try_from(last).unwrap_or(u16::MAX));
            let from = (0..last).rev().find(|step| f32::from(u16::try_from(*step).unwrap_or(0)) <= place).unwrap_or(0);
            let within = place - f32::from(u16::try_from(from).unwrap_or(0));
            unifyterm_config::toward(stops[from], stops[from + 1], within)
        }
    }
}

/// Where cell `index` of `count` is along its run, from 0 to 1.
fn share(index: usize, count: usize) -> f32 {
    if count <= 1 {
        return 0.0;
    }
    let to_f32 = |number: usize| f32::from(u16::try_from(number).unwrap_or(u16::MAX));
    to_f32(index) / to_f32(count - 1)
}

/// Whether the style draws its segments on a band of color: each piece then has a blank each side.
const fn banded(style: Style) -> bool {
    matches!(style, Style::Classic | Style::Rainbow)
}

/// Each piece's text a cell at a time (padded with a blank each side in a
/// band), and each cell's color by the gradient.
fn painted(pieces: &[Piece], prompt: &Prompt, palette: &Palette) -> Vec<Vec<(char, [u8; 3])>> {
    let banded = banded(prompt.style);
    let texts: Vec<Vec<char>> = pieces
        .iter()
        .map(|piece| if banded { format!(" {} ", piece.text) } else { piece.text.clone() }.chars().collect())
        .collect();
    let total: usize = texts.iter().map(Vec::len).sum();
    let whole: Vec<[u8; 3]> = prompt.colors.iter().map(|paint: &Paint| paint.rgb(&palette.ansi)).collect();
    let mut before = 0;
    texts
        .iter()
        .enumerate()
        .map(|(at, cells)| {
            let piece = &pieces[at];
            let own = piece.paints.first().copied().unwrap_or(palette.foreground);
            // A segment's own two colors are its gradient whatever the setting; else the setting says.
            let to = match (piece.paints.get(1), prompt.gradient, pieces.get(at + 1)) {
                (Some(second), _, _) => *second,
                (None, Gradient::Segments, Some(next)) => next.paints.first().copied().unwrap_or(own),
                _ => own,
            };
            let row = cells
                .iter()
                .enumerate()
                .map(|(index, character)| {
                    let color = if prompt.gradient == Gradient::Whole && piece.paints.len() < 2 {
                        along(&whole, share(before + index, total))
                    } else {
                        unifyterm_config::toward(own, to, share(index, cells.len()))
                    };
                    (*character, color)
                })
                .collect();
            before += cells.len();
            row
        })
        .collect()
}

/// One side's cells in the prompt's style.
fn side(pieces: &[Piece], which: Side, prompt: &Prompt, palette: &Palette) -> Vec<Cell> {
    let banded = banded(prompt.style);
    let rows = painted(pieces, prompt, palette);
    let mut cells = Vec::new();
    if !banded {
        for (at, row) in rows.iter().enumerate() {
            if at > 0 {
                cells.push(Cell::blank());
            }
            cells.extend(row.iter().map(|(text, color)| Cell { text: *text, fg: Some(*color), bg: None }));
        }
        return cells;
    }
    let rainbow = prompt.style == Style::Rainbow;
    // What is behind a cell, and what its text is drawn in.
    let behind = |color: [u8; 3]| if rainbow { color } else { palette.bar };
    let ink = |color: [u8; 3]| if rainbow { palette.background } else { color };
    let first = |row: &[(char, [u8; 3])]| row.first().map_or(palette.bar, |(_, color)| behind(*color));
    let last = |row: &[(char, [u8; 3])]| row.last().map_or(palette.bar, |(_, color)| behind(*color));
    let solid = glyph(prompt.separator, which, Weight::Solid);
    let thin = Cell {
        text: glyph(prompt.separator, which, Weight::Thin),
        fg: Some(unifyterm_config::toward(palette.bar, palette.foreground, THIN_LINE_AMOUNT)),
        bg: Some(palette.bar),
    };
    for (at, row) in rows.iter().enumerate() {
        if which == Side::Right {
            // Into the side from the terminal's background; between two segments of classic, a thin line.
            if at == 0 {
                cells.push(Cell { text: solid, fg: Some(first(row)), bg: None });
            } else if rainbow {
                cells.push(Cell { text: solid, fg: Some(first(row)), bg: Some(last(&rows[at - 1])) });
            } else {
                cells.push(thin);
            }
        }
        cells.extend(row.iter().map(|(text, color)| Cell {
            text: *text,
            fg: Some(ink(*color)),
            bg: Some(behind(*color)),
        }));
        if which == Side::Left {
            match rows.get(at + 1) {
                None => cells.push(Cell { text: solid, fg: Some(last(row)), bg: None }),
                Some(next) if rainbow => cells.push(Cell { text: solid, fg: Some(last(row)), bg: Some(first(next)) }),
                Some(_) => cells.push(thin),
            }
        }
    }
    cells
}

/// How many cells wide `cells` are on the screen.
pub(crate) fn width(cells: &[Cell]) -> usize {
    cells.iter().map(|cell| cell.text.width().unwrap_or(0)).sum()
}

/// The prompt for `facts`, as `prompt` and the theme's `palette` say.
pub(crate) fn render(prompt: &Prompt, palette: &Palette, facts: &Facts) -> Rendered {
    let left = side(&pieces(&prompt.left, prompt, palette, facts), Side::Left, prompt, palette);
    let right = side(&pieces(&prompt.right, prompt, palette, facts), Side::Right, prompt, palette);
    let failed = facts.status.is_some_and(|code| code != 0);
    let mark = Cell {
        text: if prompt.style == Style::Plain { '>' } else { '❯' },
        fg: Some(palette.ansi[if failed { 1 } else { 2 }]),
        bg: None,
    };
    let input = [mark, Cell::blank()];
    match prompt.lines {
        Lines::Two => {
            let mut first = left;
            // The right side at the end of the first line, one cell short of the edge, when it fits.
            if let Some(columns) = facts.width
                && !right.is_empty()
                && let Some(gap) = columns.checked_sub(width(&first) + width(&right) + 1).filter(|gap| *gap > 0)
            {
                first.extend(std::iter::repeat_n(Cell::blank(), gap));
                first.extend(right);
            }
            // No segment has anything to say: no empty line above the input.
            let lines = if first.is_empty() { vec![input.to_vec()] } else { vec![first, input.to_vec()] };
            Rendered { lines, right: Vec::new() }
        }
        Lines::One => {
            let mut line = left;
            if !line.is_empty() {
                line.push(Cell::blank());
            }
            line.extend(input);
            let right = if facts.shell.has_right_prompt() { right } else { Vec::new() };
            Rendered { lines: vec![line], right }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use unifyterm_config::PromptConfig;

    use super::super::facts::{Changes, Clock, Repo, Shell};
    use super::*;

    fn palette() -> Palette {
        Palette::of(&unifyterm_config::themes::find("tokyo-night", None).expect("built in"))
    }

    fn facts() -> Facts {
        Facts {
            shell: Shell::Pwsh,
            cwd: PathBuf::from("/home/me/uni-term"),
            home: Some(PathBuf::from("/home/me")),
            status: None,
            took: None,
            repo: Some(Repo { head: Head::Branch("main".into()), changes: Some(Changes::default()) }),
            venv: None,
            width: Some(60),
            now: Clock(14, 5, 9),
        }
    }

    fn prompt(text: &str) -> Prompt {
        toml::from_str::<PromptConfig>(text).expect("a [prompt]").resolved()
    }

    fn line(cells: &[Cell]) -> String {
        cells.iter().map(|cell| cell.text).collect()
    }

    #[test]
    fn lean_is_colored_words_and_the_mark_on_the_next_line() {
        let rendered = render(&prompt(""), &palette(), &facts());
        assert_eq!(rendered.lines.iter().map(|cells| line(cells)).collect::<Vec<_>>(), ["~/uni-term main", "❯ "]);
        let first = &rendered.lines[0];
        assert_eq!(first[0].fg, Some(palette().ansi[4]), "the directory in the theme's blue");
        assert_eq!(first[11].fg, Some(palette().ansi[2]), "a clean branch in its green");
        assert!(first.iter().all(|cell| cell.bg.is_none()), "nothing behind lean's words");
        assert_eq!(
            rendered.lines[1][0].fg,
            Some(palette().ansi[2]),
            "the mark is green after a command that went well"
        );
        assert!(rendered.right.is_empty());
    }

    #[test]
    fn what_went_wrong_and_how_long_it_took_go_right_at_the_end_of_the_first_line() {
        let mut facts = facts();
        facts.status = Some(2);
        facts.took = Some(Duration::from_secs(245));
        facts.repo = Some(Repo {
            head: Head::Branch("main".into()),
            changes: Some(Changes { staged: 1, changed: 2, untracked: 0, ahead: 3, behind: 0 }),
        });
        let rendered = render(&prompt(""), &palette(), &facts);
        let first = line(&rendered.lines[0]);
        assert!(first.starts_with("~/uni-term main +1 !2 ↑3 "), "{first}");
        assert!(first.ends_with(" ✘ 2 4m05s"), "{first}");
        assert_eq!(width(&rendered.lines[0]), 59, "one cell short of the 60-cell edge");
        assert_eq!(rendered.lines[0][11].fg, Some(palette().ansi[3]), "a branch with changes in yellow");
        assert_eq!(rendered.lines[1][0].fg, Some(palette().ansi[1]), "the mark is red after a failure");
        // Too narrow for both: the right side is left out, the left is whole.
        facts.width = Some(30);
        assert_eq!(line(&render(&prompt(""), &palette(), &facts).lines[0]), "~/uni-term main +1 !2 ↑3");
        facts.width = None;
        assert_eq!(
            line(&render(&prompt(""), &palette(), &facts).lines[0]),
            "~/uni-term main +1 !2 ↑3",
            "width not known"
        );
    }

    #[test]
    fn rainbow_puts_each_segment_on_its_color_with_arrows_between() {
        let rendered = render(&prompt("style = \"rainbow\""), &palette(), &facts());
        let first = &rendered.lines[0];
        assert_eq!(line(first), " ~/uni-term \u{E0B0} main \u{E0B0}");
        let (blue, green) = (palette().ansi[4], palette().ansi[2]);
        assert_eq!((first[1].fg, first[1].bg), (Some(palette().background), Some(blue)), "dark text on the blue");
        let arrow = first[12];
        assert_eq!((arrow.fg, arrow.bg), (Some(blue), Some(green)), "the blue into the green");
        let end = first.last().expect("cells");
        assert_eq!((end.fg, end.bg), (Some(green), None), "the last into the terminal's background");
    }

    #[test]
    fn classic_is_one_band_with_thin_lines_and_the_separators_follow_the_setting() {
        let rendered = render(&prompt("style = \"classic\"\nseparator = \"round\""), &palette(), &facts());
        let first = &rendered.lines[0];
        assert_eq!(line(first), " ~/uni-term \u{E0B5} main \u{E0B4}");
        assert!(
            first[..first.len() - 1].iter().all(|cell| cell.bg == Some(palette().bar)),
            "one band of the bar color"
        );
        assert_eq!(first[1].fg, Some(palette().ansi[4]), "the words in their colors on it");
        let slant = render(&prompt("style = \"rainbow\"\nseparator = \"slant\""), &palette(), &facts());
        assert_eq!(line(&slant.lines[0]), " ~/uni-term \u{E0BC} main \u{E0BC}");
    }

    #[test]
    fn plain_is_ascii_only() {
        let mut facts = facts();
        facts.status = Some(1);
        facts.repo = Some(Repo {
            head: Head::Branch("main".into()),
            changes: Some(Changes { changed: 1, ..Changes::default() }),
        });
        facts.venv = Some(".venv".into());
        let rendered = render(&prompt("style = \"plain\"\nlines = 1"), &palette(), &facts);
        let all: String = rendered.lines.iter().map(|cells| line(cells)).collect();
        assert_eq!(all, "~/uni-term (main*) > ");
        assert!(all.is_ascii());
    }

    #[test]
    fn one_line_puts_the_right_side_in_the_shells_own_place_where_it_has_one() {
        let mut facts = facts();
        facts.status = Some(1);
        let one = prompt("lines = 1");
        assert_eq!(line(&render(&one, &palette(), &facts).lines[0]), "~/uni-term main ❯ ");
        assert!(render(&one, &palette(), &facts).right.is_empty(), "PowerShell has no place for it");
        facts.shell = Shell::Zsh;
        assert_eq!(line(&render(&one, &palette(), &facts).right), "✘ 1");
    }

    #[test]
    fn gradients_change_the_color_from_cell_to_cell() {
        let palette = palette();
        let (blue, green) = (palette.ansi[4], palette.ansi[2]);
        let segments = render(&prompt("gradient = \"segments\""), &palette, &facts());
        let dir = &segments.lines[0][..10];
        assert_eq!(
            (dir[0].fg, dir[9].fg),
            (Some(blue), Some(green)),
            "the directory from its blue to the branch's green"
        );
        let own = render(&prompt("dir_color = [\"red\", \"yellow\"]"), &palette, &facts());
        assert_eq!(
            (own.lines[0][0].fg, own.lines[0][9].fg),
            (Some(palette.ansi[1]), Some(palette.ansi[3])),
            "its own two"
        );
        let whole = render(&prompt("gradient = \"whole\"\ncolors = [\"#000000\", \"#ffffff\"]"), &palette, &facts());
        let cells: Vec<&Cell> = whole.lines[0].iter().filter(|cell| cell.fg.is_some()).collect();
        assert_eq!((cells[0].fg, cells.last().expect("cells").fg), (Some([0, 0, 0]), Some([255, 255, 255])));
        let off = render(&prompt(""), &palette, &facts());
        assert!(off.lines[0][..10].iter().all(|cell| cell.fg == Some(blue)), "no gradient: one color");
    }

    #[test]
    fn a_segment_with_nothing_to_say_is_left_out() {
        let mut facts = facts();
        facts.repo = None;
        facts.status = Some(0);
        facts.took = Some(Duration::from_millis(1999));
        let rendered = render(&prompt("style = \"rainbow\"\nright = [\"status\", \"duration\"]"), &palette(), &facts);
        assert_eq!(line(&rendered.lines[0]), " ~/uni-term \u{E0B0}", "no repository, no failure, too short");
        facts.cwd = PathBuf::from("/home/me");
        assert_eq!(line(&render(&prompt(""), &palette(), &facts).lines[0]), "~", "the home itself");
        facts.cwd = PathBuf::from("/srv/data");
        assert_eq!(line(&render(&prompt(""), &palette(), &facts).lines[0]), "/srv/data", "outside the home");
    }

    #[test]
    fn with_no_segments_the_prompt_is_the_mark_alone() {
        let rendered = render(&prompt("left = []\nright = []"), &palette(), &facts());
        assert_eq!(
            rendered.lines.iter().map(|cells| line(cells)).collect::<Vec<_>>(),
            ["❯ "],
            "no empty line above it"
        );
        let one = render(&prompt("left = []\nright = []\nlines = 1"), &palette(), &facts());
        assert_eq!(one.lines.iter().map(|cells| line(cells)).collect::<Vec<_>>(), ["❯ "]);
    }

    #[test]
    fn a_control_character_in_a_name_is_shown_and_breaks_nothing() {
        let mut facts = facts();
        facts.cwd = PathBuf::from("/home/me/two\nlines\u{1b}[31m");
        facts.repo = Some(Repo { head: Head::Branch("bad\rname".into()), changes: None });
        let rendered = render(&prompt(""), &palette(), &facts);
        assert_eq!(line(&rendered.lines[0]), "~/two?lines?[31m bad?name");
        assert_eq!(rendered.lines.len(), 2, "still the segments' line and the input's");
    }

    #[test]
    fn durations_read_as_a_person_says_them() {
        assert_eq!(took(Duration::from_secs(2)), "2s");
        assert_eq!(took(Duration::from_secs(59)), "59s");
        assert_eq!(took(Duration::from_mins(1)), "1m00s");
        assert_eq!(took(Duration::from_secs(3599)), "59m59s");
        assert_eq!(took(Duration::from_mins(123)), "2h03m");
    }
}
