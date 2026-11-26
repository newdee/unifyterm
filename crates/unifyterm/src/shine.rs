//! The light across the newest prompt (`prompt.shine`): a soft band, a few
//! cells wide, goes from the prompt's first lit cell to its last, and each
//! lit cell is brighter while the band is over it. A cell is lit when it has
//! a character in it, or a color behind it; blank cells are not touched, and
//! a blank stretch wider than two cells is closed up, so the light does not
//! spend its time over nothing.
//!
//! The shell's marks say where the prompt is ([`PromptSpan`]), so this works
//! on any prompt a hook marks: UnifyTerm's own, or the user's.

use std::time::{Duration, Instant};

use unifyterm_config::prompt::Shine;

use crate::prompt::PromptSpan;
use crate::session::{Color, Snapshot, attr};

/// How long the light takes from the first lit cell to the last.
pub(crate) const SWEEP: Duration = Duration::from_millis(1100);
/// How long after a new prompt the light sets out.
const DELAY: Duration = Duration::from_millis(150);
/// The band's half width, in cells.
const WIDTH: f32 = 3.5;
/// A blank stretch wider than this many cells is closed up to it.
const GAP: f32 = 2.0;
/// Under this a cell is drawn as it is.
const FAINT: f32 = 0.02;

/// One light over one prompt: when it began, and how often it comes again (`loop`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sweep {
    pub(crate) span: PromptSpan,
    began: Instant,
    every: Option<Duration>,
}

impl Sweep {
    /// The light for a new prompt at `span`, as `shine` says; `None` when it is off.
    pub(crate) fn new(span: PromptSpan, shine: Shine, every: Duration, now: Instant) -> Option<Self> {
        match shine {
            Shine::Off => None,
            Shine::Once => Some(Self { span, began: now, every: None }),
            Shine::Loop => Some(Self { span, began: now, every: Some(every.max(SWEEP)) }),
        }
    }

    /// The time into the current pass, when one is under way or due: none once a single pass is over.
    fn time_in_pass(&self, now: Instant) -> Option<Duration> {
        let since = now.checked_duration_since(self.began + DELAY)?;
        match self.every {
            None => (since <= SWEEP).then_some(since),
            Some(every) => {
                let every_ms = every.as_millis().max(1);
                Some(Duration::from_millis(u64::try_from(since.as_millis() % every_ms).unwrap_or(0)))
            }
        }
    }

    /// How far along its way the light is at `now`, from 0 to 1, eased to
    /// slow at the end; `None` when no light is on the prompt now.
    pub(crate) fn progress(&self, now: Instant) -> Option<f32> {
        let into = self.time_in_pass(now)?;
        (into <= SWEEP).then(|| {
            let share = into.as_secs_f32() / SWEEP.as_secs_f32();
            1.0 - (1.0 - share).powi(2)
        })
    }

    /// How long until the light is on the prompt again, when it is not now;
    /// `None` when it will not be (a single pass that is over).
    pub(crate) fn until_next(&self, now: Instant) -> Option<Duration> {
        let start = self.began + DELAY;
        if now < start {
            return Some(start - now);
        }
        let into = self.time_in_pass(now)?;
        match self.every {
            Some(every) if into > SWEEP => Some(every.saturating_sub(into)),
            _ => Some(Duration::ZERO),
        }
    }
}

/// How bright the light makes the prompt's lit cells at one moment: each as
/// (view line, column, amount from 0 to 1). Cells not here are as they are.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Glow {
    cells: Vec<(usize, usize, f32)>,
}

impl Glow {
    /// How bright the cell at (`line`, `column`) of the view is made: 0 for most.
    /// Asked for every cell of every frame while the light is on: the cells
    /// are kept in reading order ([`lit`] finds them so), and looked up by halves.
    pub(crate) fn at(&self, line: usize, column: usize) -> f32 {
        self.cells.binary_search_by(|(l, c, _)| (*l, *c).cmp(&(line, column))).map_or(0.0, |found| self.cells[found].2)
    }
}

/// Whether a cell has nothing to light: no character and the screen's own color behind it.
fn blank(snapshot: &Snapshot, line: usize, column: usize) -> bool {
    let cell = &snapshot.row(line)[column];
    let empty = matches!(cell.c, ' ' | '\0') && cell.combining.is_none();
    cell.has(attr::SPACER) || (empty && matches!(cell.bg, Color::Background))
}

/// The prompt's lit cells on the screen, as (view line, column).
fn lit(snapshot: &Snapshot, span: PromptSpan) -> Vec<(usize, usize)> {
    let mut cells = Vec::new();
    for line in span.start.0..=span.input.0 {
        // Lines counted from the top of the history, as the marks are: the view's row, if it shows.
        let Some(view) = (line + snapshot.scrolled).checked_sub(snapshot.history).filter(|view| *view < snapshot.lines)
        else {
            continue;
        };
        let from = if line == span.start.0 { span.start.1 } else { 0 };
        let to = if line == span.input.0 { span.input.1 } else { snapshot.columns };
        for column in from..to.min(snapshot.columns) {
            if !blank(snapshot, view, column) {
                cells.push((view, column));
            }
        }
    }
    cells
}

/// A whole number of cells as a distance.
fn cells(count: usize) -> f32 {
    f32::from(u16::try_from(count).unwrap_or(u16::MAX))
}

/// How bright each of the prompt's lit cells is with the light `along` (0
/// to 1) its way from before the first to past the last.
pub(crate) fn glow_at(snapshot: &Snapshot, span: PromptSpan, along: f32) -> Glow {
    let lit = lit(snapshot, span);
    let mut columns: Vec<usize> = lit.iter().map(|(_, column)| *column).collect();
    columns.sort_unstable();
    columns.dedup();
    // Where each column is on the light's way: wide blank stretches closed up.
    let mut way: Vec<(usize, f32)> = Vec::with_capacity(columns.len());
    for (at, column) in columns.iter().enumerate() {
        let place = match at.checked_sub(1).map(|before| (columns[before], way[before].1)) {
            None => cells(*column),
            Some((before, there)) => there + cells(column - before).min(GAP),
        };
        way.push((*column, place));
    }
    let place_of = |column: usize| way.iter().find(|(known, _)| *known == column).map_or(0.0, |(_, place)| *place);
    let (Some(first), Some(last)) = (way.first(), way.last()) else { return Glow::default() };
    let (from, to) = (first.1 - 2.0 * WIDTH, last.1 + 2.0 * WIDTH);
    let light = from + (to - from) * along;
    let cells = lit
        .into_iter()
        .filter_map(|(line, column)| {
            let amount = (-((place_of(column) - light) / WIDTH).powi(2)).exp();
            (amount >= FAINT).then_some((line, column, amount))
        })
        .collect();
    Glow { cells }
}

/// How bright the prompt's lit cells are at `now`; nothing when no light is on it.
pub(crate) fn glow(snapshot: &Snapshot, sweep: &Sweep, now: Instant) -> Glow {
    sweep.progress(now).map_or_else(Glow::default, |along| glow_at(snapshot, sweep.span, along))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Cell;

    /// A screen of `rows`, as wide as the longest; `#` marks a cell with a color behind it and a blank.
    fn screen(rows: &[&str], history: usize) -> Snapshot {
        let columns = rows.iter().map(|row| row.chars().count()).max().unwrap_or(1);
        let mut cells = Vec::new();
        for row in rows {
            let mut chars: Vec<char> = row.chars().collect();
            chars.resize(columns, ' ');
            for c in chars {
                let (c, bg) = if c == '#' { (' ', Color::Indexed(4)) } else { (c, Color::Background) };
                cells.push(Cell { c, combining: None, fg: Color::Foreground, bg, attrs: 0 });
            }
        }
        Snapshot { columns, lines: rows.len(), cells, cursor: None, scrolled: 0, history }
    }

    #[test]
    fn only_cells_with_something_in_them_are_lit() {
        // `~/x main` then a blank stretch then `3s`; the second line is the mark.
        let snapshot = screen(&["~/x main            3s", "> echo"], 0);
        let span = PromptSpan { start: (0, 0), input: (1, 2) };
        let found = lit(&snapshot, span);
        assert!(found.contains(&(0, 0)) && found.contains(&(0, 21)) && found.contains(&(1, 0)));
        assert!(!found.contains(&(0, 3)) && !found.contains(&(0, 10)), "the blanks are not");
        assert!(
            !found.contains(&(1, 2)) && !found.contains(&(1, 3)),
            "what is typed after the prompt is not the prompt's"
        );
        // A blank with a color behind it is a band's: lit.
        let band = screen(&[" #x# "], 0);
        assert_eq!(lit(&band, PromptSpan { start: (0, 0), input: (0, 5) }), [(0, 1), (0, 2), (0, 3)]);
    }

    #[test]
    fn the_light_goes_from_the_first_lit_cell_to_the_last_and_over_nothing_else() {
        let snapshot = screen(&["~/x main            3s", "> "], 0);
        let span = PromptSpan { start: (0, 0), input: (1, 2) };
        let start = glow_at(&snapshot, span, 0.0);
        assert!(start.cells.is_empty() || start.at(0, 0) < 0.1, "it starts before the first cell");
        let early = glow_at(&snapshot, span, 0.35);
        let late = glow_at(&snapshot, span, 0.8);
        assert!(early.at(0, 1) > late.at(0, 1), "the start is lit first");
        assert!(late.at(0, 21) > early.at(0, 21), "the end later");
        for along in [0.1, 0.3, 0.5, 0.7, 0.9] {
            let glow = glow_at(&snapshot, span, along);
            assert!(glow.at(0, 12).abs() < f32::EPSILON, "a blank cell is never lit");
            assert!(glow.cells.iter().all(|(_, _, amount)| (FAINT..=1.0).contains(amount)));
        }
        // The blank stretch is closed up: halfway along, the light is not lost in it.
        let middle = glow_at(&snapshot, span, 0.5);
        assert!(middle.at(0, 7) > 0.2 || middle.at(0, 20) > 0.2, "{middle:?}");
    }

    #[test]
    fn a_prompt_scrolled_away_or_above_the_screen_is_not_lit() {
        // Two lines of history above: the prompt marked on line 2 is the view's first row.
        let snapshot = screen(&["~/x", "> "], 2);
        let span = PromptSpan { start: (2, 0), input: (3, 2) };
        assert!(glow_at(&snapshot, span, 0.3).at(0, 0) > 0.0);
        let gone = PromptSpan { start: (0, 0), input: (1, 2) };
        assert!(glow_at(&snapshot, gone, 0.3).cells.is_empty(), "in the history, not on the screen");
    }

    #[test]
    fn once_is_one_pass_and_loop_comes_again_every_so_often() {
        let span = PromptSpan { start: (0, 0), input: (0, 2) };
        let now = Instant::now();
        assert_eq!(Sweep::new(span, Shine::Off, Duration::from_secs(4), now), None);
        let once = Sweep::new(span, Shine::Once, Duration::from_secs(4), now).expect("on");
        assert_eq!(once.progress(now), None, "it waits a moment first");
        assert_eq!(once.until_next(now), Some(DELAY));
        let halfway = now + DELAY + SWEEP / 2;
        let along = once.progress(halfway).expect("under way");
        assert!((0.7..0.8).contains(&along), "eased: past the middle at half the time ({along})");
        assert_eq!(once.progress(now + DELAY + SWEEP + Duration::from_millis(1)), None, "over");
        assert_eq!(once.until_next(now + Duration::from_secs(5)), None, "and not again");
        let looped = Sweep::new(span, Shine::Loop, Duration::from_secs(4), now).expect("on");
        let between = now + DELAY + Duration::from_secs(2);
        assert_eq!(looped.progress(between), None, "between two passes");
        assert_eq!(looped.until_next(between), Some(Duration::from_secs(2)));
        assert!(looped.progress(now + DELAY + Duration::from_secs(4) + SWEEP / 4).is_some(), "the next pass");
        let quick = Sweep::new(span, Shine::Loop, Duration::from_millis(100), now).expect("on");
        assert_eq!(quick.every, Some(SWEEP), "a pass is never cut short by the next");
    }
}
