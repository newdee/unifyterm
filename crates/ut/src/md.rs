//! `ut md FILE`: markdown as styled text in the terminal, the way `glow`
//! and `mdcat` show it. Headings bold, emphasis and code in their styles,
//! lists and quotes indented, tables with ruled borders lined up by display
//! width (CJK takes two columns), links as terminal hyperlinks. Paragraphs
//! are wrapped to the width. Output that is no terminal gets no colors
//! (unless `--color=always`), but the same layout.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{self, IsTerminal as _, Write as _};

use pulldown_cmark::{Alignment, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use unicode_width::UnicodeWidthStr as _;

const USAGE: &str = "usage: ut md [--width COLUMNS] [--color=always|never|auto] [FILE]

  Shows markdown (FILE, or standard input) as styled text.
  --width   wrap at this many columns (default: the terminal's width, at most 100)
  --color   styles: always, never, or auto (only when the output is a terminal)";

const USAGE_ERROR: i32 = 2;
const TROUBLE: i32 = 1;
const DEFAULT_WIDTH: usize = 100;

/// Whether styles are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Color {
    Styled,
    Plain,
}

/// A piece of inline text and the styles it has.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[expect(clippy::struct_excessive_bools, reason = "independent text styles, one SGR code each")]
struct Style {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    underline: bool,
    link: Option<String>,
}

impl Style {
    /// The SGR codes that turn this style on.
    fn codes(&self) -> String {
        let mut codes = Vec::new();
        if self.bold {
            codes.push("1");
        }
        if self.italic {
            codes.push("3");
        }
        if self.underline || self.link.is_some() {
            codes.push("4");
        }
        if self.strike {
            codes.push("9");
        }
        if self.code {
            codes.push("36");
        }
        if codes.is_empty() { String::new() } else { format!("\x1b[{}m", codes.join(";")) }
    }
}

/// A word to lay out: its text, style, and whether a space comes before it.
#[derive(Debug, Clone)]
struct Word {
    text: String,
    style: Style,
    space_before: bool,
}

/// Which line of a block is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    First,
    Later,
}

/// How a block's lines begin: the first line's prefix, then the others'.
#[derive(Debug, Clone)]
struct Indent {
    first: String,
    rest: String,
}

/// The table being read: its alignments, rows of cells (each cell its text), the cell open.
#[derive(Debug, Default)]
struct Table {
    alignments: Vec<Alignment>,
    rows: Vec<Vec<String>>,
    head_rows: usize,
    cell: String,
}

/// Turns markdown events into text.
struct Writer {
    width: usize,
    color: Color,
    out: String,
    words: Vec<Word>,
    /// The style stack: one entry a style opened.
    styles: Vec<Style>,
    /// Prefixes of the blocks open (list items, quotes), outermost first.
    indents: Vec<Indent>,
    /// The list numbers open: `None` for a bulleted list.
    lists: Vec<Option<u64>>,
    heading: Option<HeadingLevel>,
    code_block: Option<String>,
    table: Option<Table>,
    /// A space is owed before the next word (text ended with one, or a soft break).
    space: bool,
}

/// How many columns `text` takes on screen.
fn width(text: &str) -> usize {
    text.width()
}

impl Writer {
    fn style(&self) -> Style {
        self.styles.last().cloned().unwrap_or_default()
    }

    fn push_style(&mut self, change: impl FnOnce(&mut Style)) {
        let mut style = self.style();
        change(&mut style);
        self.styles.push(style);
    }

    /// A blank line between blocks (not at the top, not twice).
    fn gap(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with("\n\n") {
            self.out.push('\n');
        }
    }

    /// The prefix of the next line: the first line of a block, or a later one.
    fn prefix(&mut self, which: Line) -> String {
        let mut prefix = String::new();
        let last = self.indents.len().saturating_sub(1);
        for (index, indent) in self.indents.iter_mut().enumerate() {
            if which == Line::First && index == last {
                prefix.push_str(&indent.first);
                // A bullet is written once: later lines line up under the text.
                indent.first.clone_from(&indent.rest);
            } else {
                prefix.push_str(&indent.rest);
            }
        }
        prefix
    }

    fn text(&mut self, text: &str) {
        if let Some(table) = self.table.as_mut() {
            table.cell.push_str(text);
            return;
        }
        if let Some(block) = self.code_block.as_mut() {
            block.push_str(text);
            return;
        }
        let style = self.style();
        let mut first = true;
        for piece in text.split(' ') {
            if !first {
                self.space = true;
            }
            first = false;
            if piece.is_empty() {
                continue;
            }
            self.words.push(Word { text: piece.to_owned(), style: style.clone(), space_before: self.space });
            self.space = false;
        }
    }

    /// Writes the words collected as wrapped lines, in this block's indent.
    fn flush(&mut self) {
        if self.words.is_empty() {
            return;
        }
        let words = std::mem::take(&mut self.words);
        let heading_style = self.heading.map(|level| {
            let mut style = Style { bold: true, ..Style::default() };
            style.underline = level == HeadingLevel::H1;
            style
        });
        let mut line = String::new();
        let mut used = 0;
        let mut prefix = self.prefix(Line::First);
        let total = self.width;
        let room = |prefix: &str| total.saturating_sub(width(prefix)).max(10);
        for word in words {
            let style = match &heading_style {
                Some(heading) => Style { bold: true, underline: heading.underline, ..word.style.clone() },
                None => word.style.clone(),
            };
            let gap = usize::from(word.space_before && used > 0);
            if used > 0 && used + gap + width(&word.text) > room(&prefix) {
                let _ = writeln!(self.out, "{prefix}{line}");
                line.clear();
                used = 0;
                prefix = self.prefix(Line::Later);
            }
            if used > 0 && word.space_before {
                line.push(' ');
                used += 1;
            }
            line.push_str(&self.styled(&word.text, &style));
            used += width(&word.text);
        }
        let _ = writeln!(self.out, "{prefix}{line}");
        self.space = false;
    }

    /// `text` in `style`, with the styles reset after it; a link as a terminal hyperlink.
    fn styled(&self, text: &str, style: &Style) -> String {
        if self.color == Color::Plain {
            return text.to_owned();
        }
        let codes = style.codes();
        let body = if codes.is_empty() { text.to_owned() } else { format!("{codes}{text}\x1b[0m") };
        match &style.link {
            Some(url) => format!("\x1b]8;;{url}\x1b\\{body}\x1b]8;;\x1b\\"),
            None => body,
        }
    }

    fn dim(&self, text: &str) -> String {
        if self.color == Color::Plain { text.to_owned() } else { format!("\x1b[2m{text}\x1b[0m") }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.gap(),
            Tag::Heading { level, .. } => {
                self.gap();
                self.heading = Some(level);
                let marks = "#".repeat(match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                });
                // Plain output keeps the heading's level visible.
                if self.color == Color::Plain {
                    self.words.push(Word { text: marks, style: Style::default(), space_before: false });
                    self.space = true;
                }
            }
            Tag::BlockQuote(_) => {
                self.gap();
                let bar = format!("{} ", self.dim("│"));
                self.indents.push(Indent { first: bar.clone(), rest: bar });
            }
            Tag::CodeBlock(_) => {
                self.gap();
                self.code_block = Some(String::new());
            }
            Tag::List(start) => {
                if self.lists.is_empty() {
                    self.gap();
                }
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                let depth = self.lists.len();
                let marker = match self.lists.last_mut() {
                    Some(Some(number)) => {
                        let marker = format!("{number}.");
                        *number += 1;
                        marker
                    }
                    _ => if depth % 2 == 1 { "•" } else { "◦" }.to_owned(),
                };
                let pad = " ".repeat(width(&marker) + 1);
                self.indents.push(Indent { first: format!("{marker} "), rest: pad });
            }
            Tag::Emphasis => self.push_style(|style| style.italic = true),
            Tag::Strong => self.push_style(|style| style.bold = true),
            Tag::Strikethrough => self.push_style(|style| style.strike = true),
            Tag::Link { dest_url, .. } => {
                let url = dest_url.to_string();
                self.push_style(|style| style.link = Some(url));
            }
            Tag::Image { .. } => {
                self.push_style(|_| {});
                self.text("[image:");
                self.space = true;
            }
            Tag::Table(alignments) => {
                self.gap();
                self.table = Some(Table { alignments, ..Table::default() });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.rows.push(Vec::new());
                }
            }
            Tag::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    table.cell.clear();
                }
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush(),
            TagEnd::Heading(_) => {
                self.flush();
                self.heading = None;
            }
            TagEnd::BlockQuote(_) | TagEnd::Item => {
                self.flush();
                self.indents.pop();
            }
            TagEnd::CodeBlock => {
                let block = self.code_block.take().unwrap_or_default();
                for line in block.trim_end_matches('\n').lines() {
                    let prefix = self.prefix(Line::Later);
                    let _ = writeln!(self.out, "{prefix}  {}", self.dim(line));
                }
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
            }

            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::Link => {
                if let Some(url) = self.styles.pop().and_then(|style| style.link) {
                    // The address after the text too: not every terminal makes links clickable.
                    let shown = self.dim(&format!("({url})"));
                    self.words.push(Word { text: shown, style: Style::default(), space_before: true });
                }
            }
            TagEnd::Image => {
                self.styles.pop();
                self.text("]");
                if let Some(last) = self.words.last_mut() {
                    last.space_before = false;
                }
            }
            TagEnd::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    let cell = std::mem::take(&mut table.cell).trim().to_owned();
                    if let Some(row) = table.rows.last_mut() {
                        row.push(cell);
                    }
                }
            }
            TagEnd::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.head_rows = table.rows.len();
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.write_table(&table);
                }
            }
            _ => {}
        }
    }

    /// A table with ruled borders, its columns as wide as their widest cell.
    fn write_table(&mut self, table: &Table) {
        let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        let widths: Vec<usize> = (0..columns)
            .map(|column| {
                table.rows.iter().filter_map(|row| row.get(column)).map(|cell| width(cell)).max().unwrap_or(0)
            })
            .collect();
        let rule = |left: &str, middle: &str, right: &str| {
            let parts: Vec<String> = widths.iter().map(|width| "─".repeat(width + 2)).collect();
            format!("{left}{}{right}", parts.join(middle))
        };
        let prefix = self.prefix(Line::Later);
        let _ = writeln!(self.out, "{prefix}{}", self.dim(&rule("┌", "┬", "┐")));
        for (index, row) in table.rows.iter().enumerate() {
            let mut line = self.dim("│");
            for (column, wide) in widths.iter().enumerate() {
                let cell = row.get(column).map_or("", String::as_str);
                let pad = wide - width(cell);
                let (before, after) = match table.alignments.get(column) {
                    Some(Alignment::Right) => (pad, 0),
                    Some(Alignment::Center) => (pad / 2, pad - pad / 2),
                    _ => (0, pad),
                };
                let text = if index < table.head_rows {
                    self.styled(cell, &Style { bold: true, ..Style::default() })
                } else {
                    cell.to_owned()
                };
                let _ = write!(line, " {}{text}{} {}", " ".repeat(before), " ".repeat(after), self.dim("│"));
            }
            let _ = writeln!(self.out, "{prefix}{line}");
            if index + 1 == table.head_rows {
                let _ = writeln!(self.out, "{prefix}{}", self.dim(&rule("├", "┼", "┤")));
            }
        }
        let _ = writeln!(self.out, "{prefix}{}", self.dim(&rule("└", "┴", "┘")));
    }
}

/// `markdown` as text `columns` wide.
pub(crate) fn render(markdown: &str, columns: usize, color: Color) -> String {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut writer = Writer {
        width: columns,
        color,
        out: String::new(),
        words: Vec::new(),
        styles: Vec::new(),
        indents: Vec::new(),
        lists: Vec::new(),
        heading: None,
        code_block: None,
        table: None,
        space: false,
    };
    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(tag) => writer.start(tag),
            Event::End(tag) => writer.end(tag),
            Event::Text(text) => writer.text(&text),
            Event::Code(code) => {
                let style = Style { code: true, ..writer.style() };
                if let Some(table) = writer.table.as_mut() {
                    table.cell.push_str(&code);
                } else {
                    writer.words.push(Word { text: code.to_string(), style, space_before: writer.space });
                    writer.space = false;
                }
            }
            Event::SoftBreak => writer.space = true,
            Event::HardBreak => writer.flush(),
            Event::Rule => {
                writer.flush();
                writer.gap();
                let rule = writer.dim(&"─".repeat(columns.min(60)));
                let _ = writeln!(writer.out, "{rule}");
            }
            Event::TaskListMarker(done) => {
                // The bullet becomes a box.
                if let Some(indent) = writer.indents.last_mut() {
                    if done { "☑ " } else { "☐ " }.clone_into(&mut indent.first);
                    "  ".clone_into(&mut indent.rest);
                }
            }
            Event::Html(html) | Event::InlineHtml(html) => writer.text(html.trim()),
            _ => {}
        }
    }
    writer.flush();
    writer.out
}

pub(crate) fn main(args: &[OsString]) -> i32 {
    let mut words = args.iter().map(|arg| arg.to_string_lossy().into_owned());
    let (mut columns, mut color, mut file) = (None, None, None);
    while let Some(word) = words.next() {
        match word.as_str() {
            "--help" | "-h" => {
                let _ = writeln!(io::stdout(), "{USAGE}");
                return 0;
            }
            "--width" => {
                match words.next().and_then(|value| value.parse::<usize>().ok()).filter(|value| *value >= 20) {
                    Some(width) => columns = Some(width),
                    None => return usage("--width is a number of columns, at least 20"),
                }
            }
            "--color=always" => color = Some(Color::Styled),
            "--color=never" => color = Some(Color::Plain),
            "--color=auto" => color = None,
            other if other.starts_with("--") => return usage(&format!("unknown option '{other}'")),
            _ if file.is_none() => file = Some(word),
            _ => return usage(&format!("unexpected argument '{word}'")),
        }
    }
    let text = match &file {
        Some(file) if file != "-" => std::fs::read_to_string(file).map_err(|error| format!("{file}: {error}")),
        _ => io::read_to_string(io::stdin()).map_err(|error| format!("standard input: {error}")),
    };
    let text = match text {
        Ok(text) => text,
        Err(problem) => {
            let _ = writeln!(io::stderr(), "ut md: {problem}");
            return TROUBLE;
        }
    };
    let terminal = io::stdout().is_terminal();
    let color = color.unwrap_or(if terminal { Color::Styled } else { Color::Plain });
    let columns = columns.unwrap_or_else(|| {
        let width = if terminal {
            terminal_size::terminal_size().map(|(terminal_size::Width(w), _)| usize::from(w))
        } else {
            None
        };
        width.unwrap_or(80).min(DEFAULT_WIDTH)
    });
    match io::stdout().lock().write_all(render(&text, columns, color).as_bytes()) {
        Err(error) if error.kind() != io::ErrorKind::BrokenPipe => {
            let _ = writeln!(io::stderr(), "ut md: {error}");
            TROUBLE
        }
        _ => 0,
    }
}

fn usage(problem: &str) -> i32 {
    let _ = writeln!(io::stderr(), "ut md: {problem}\n{USAGE}");
    USAGE_ERROR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_output_keeps_the_layout() {
        let markdown = "# Title\n\nSome *words* and **more** with `code`, wrapped when the line is long enough to wrap.\n\n- one\n- two\n  - deeper\n\n1. first\n2. second\n\n> quoted\n\n```\nlet x = 1;\n```\n\n- [ ] todo\n- [x] done\n";
        let out = render(markdown, 30, Color::Plain);
        let expected = "# Title\n\nSome words and more with code,\nwrapped when the line is long\nenough to wrap.\n\n• one\n• two\n  ◦ deeper\n\n1. first\n2. second\n\n│ quoted\n\n  let x = 1;\n\n☐ todo\n☑ done\n";
        assert_eq!(out, expected, "\n{out}");
        assert!(out.lines().all(|line| width(line) <= 30));
    }

    #[test]
    fn tables_line_up_by_display_width() {
        let markdown = "| name | 数量 |\n|:-----|-----:|\n| 中文 | 3 |\n| ab | 12 |\n";
        let out = render(markdown, 80, Color::Plain);
        assert_eq!(
            out,
            "┌──────┬──────┐\n│ name │ 数量 │\n├──────┼──────┤\n│ 中文 │    3 │\n│ ab   │   12 │\n└──────┴──────┘\n",
            "\n{out}"
        );
        let widths: Vec<usize> = out.lines().map(width).collect();
        assert!(widths.iter().all(|w| *w == widths[0]), "every line as wide: {widths:?}");
    }

    #[test]
    fn styles_links_and_images() {
        let out = render("A [link](https://x.org) and **bold** ![logo](a.png)", 80, Color::Styled);
        assert!(out.contains("\x1b]8;;https://x.org\x1b\\\x1b[4mlink\x1b[0m\x1b]8;;\x1b\\"), "{out:?}");
        assert!(out.contains("(https://x.org)"));
        assert!(out.contains("\x1b[1mbold\x1b[0m"));
        assert!(out.contains("[image: logo]"), "{out:?}");
        let heading = render("# Big", 80, Color::Styled);
        assert_eq!(heading, "\x1b[1;4mBig\x1b[0m\n");
        assert_eq!(render("", 80, Color::Plain), "");
        assert_eq!(render("---", 20, Color::Plain), "────────────────────\n");
    }
}
