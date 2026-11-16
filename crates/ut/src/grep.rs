//! grep: GNU grep's options, output and exit codes over ripgrep's searcher
//! and regex matcher.
//!
//! What is here is the part that is GNU grep's alone: the options, basic
//! regular expressions (ripgrep's are extended ones), the `file:line:text`
//! output with its colors, and which messages go where. Finding the lines
//! is the libraries' work.
//!
//! Not done: `-P` (Perl expressions), `-z`, back-references. Each is
//! refused by name, not ignored.

use std::ffi::{OsStr, OsString};
use std::io::{self, IsTerminal as _, Read as _, Write};
use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use grep_matcher::Matcher as _;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkMatch};
use walkdir::WalkDir;

/// grep's exit codes: a line was selected, none was, something went wrong.
const SELECTED: i32 = 0;
const NONE_SELECTED: i32 = 1;
const TROUBLE: i32 = 2;

/// How much of a file's start is looked at to decide whether it is text, as GNU grep does.
const BINARY_PROBE: usize = 32 * 1024;
/// What standard input is called in output.
const STDIN_LABEL: &str = "(standard input)";

/// GNU grep's default colors (`GREP_COLORS`), each ending with "erase to
/// the end of the line" as grep writes them.
const COLOR_MATCH: &str = "\x1b[01;31m\x1b[K";
const COLOR_FILE: &str = "\x1b[35m\x1b[K";
const COLOR_LINE: &str = "\x1b[32m\x1b[K";
const COLOR_SEPARATOR: &str = "\x1b[36m\x1b[K";
const COLOR_END: &str = "\x1b[m\x1b[K";

const USAGE: &str = "usage: grep [OPTION]... PATTERNS [FILE]...

Pattern:      -E extended   -F fixed strings   -G basic (default)
              -e PATTERNS   -f FILE   -i ignore case   -w whole words   -x whole lines
Selection:    -v lines that do not match   -m NUM stop after NUM lines
              -a binary files as text   -I skip binary files
Output:       -n line numbers   -b byte offsets   -H/-h with/without file names
              -o only the matching parts   -c count   -l/-L files with/without a match
              -q quiet   -s no messages about files   -Z a NUL after file names
              -A NUM, -B NUM, -C NUM lines of context   --color[=always|never|auto]
              --line-buffered
Files:        -r, -R recursive   --include=GLOB   --exclude=GLOB   --exclude-dir=GLOB

Exit status: 0 a line was selected, 1 none was, 2 an error.";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Syntax {
    Basic,
    Extended,
    Fixed,
}

#[derive(Debug)]
struct Options {
    patterns: Vec<String>,
    syntax: Syntax,
    ignore_case: bool,
    invert: bool,
    word: bool,
    whole_line: bool,
    count: bool,
    files_with_matches: bool,
    files_without_match: bool,
    quiet: bool,
    no_messages: bool,
    only_matching: bool,
    line_number: bool,
    byte_offset: bool,
    /// `None`: file names when there is more than one file to search.
    with_filename: Option<bool>,
    null_after_name: bool,
    max_count: Option<u64>,
    after: usize,
    before: usize,
    recursive: bool,
    /// With `-R`, links to directories are followed while descending.
    follow_links: bool,
    text: bool,
    skip_binary: bool,
    color: bool,
    line_buffered: bool,
    include: Vec<String>,
    exclude: Vec<String>,
    exclude_dir: Vec<String>,
    files: Vec<OsString>,
}

/// What the arguments ask for instead of a search.
enum Early {
    /// Print this and end with success.
    Print(String),
    /// A mistake: print this to stderr and end with trouble.
    Usage(String),
}

fn parse_number(option: &str, text: &str) -> Result<usize, Early> {
    text.parse().map_err(|_| Early::Usage(format!("{text}: invalid number for {option}")))
}

fn parse(args: Vec<OsString>) -> Result<Options, Early> {
    let mut o = Options {
        patterns: Vec::new(),
        syntax: Syntax::Basic,
        ignore_case: false,
        invert: false,
        word: false,
        whole_line: false,
        count: false,
        files_with_matches: false,
        files_without_match: false,
        quiet: false,
        no_messages: false,
        only_matching: false,
        line_number: false,
        byte_offset: false,
        with_filename: None,
        null_after_name: false,
        max_count: None,
        after: 0,
        before: 0,
        recursive: false,
        follow_links: false,
        text: false,
        skip_binary: false,
        color: false,
        line_buffered: false,
        include: Vec::new(),
        exclude: Vec::new(),
        exclude_dir: Vec::new(),
        files: Vec::new(),
    };
    let mut pattern_given = false;
    let mut operands: Vec<OsString> = Vec::new();
    let mut args = args.into_iter().skip(1);
    // An option's value: what follows `=` or the option's letter, else the next argument.
    fn value(option: &str, inline: Option<String>, rest: &mut impl Iterator<Item = OsString>) -> Result<String, Early> {
        match inline {
            Some(value) => Ok(value),
            None => rest
                .next()
                .map(|v| v.to_string_lossy().into_owned())
                .ok_or_else(|| Early::Usage(format!("option {option} requires an argument"))),
        }
    }
    let patterns_from_file = |file: &str| -> Result<Vec<String>, Early> {
        let text = std::fs::read_to_string(file).map_err(|e| Early::Usage(format!("{file}: {}", message(&e))))?;
        Ok(text.lines().map(str::to_owned).collect())
    };
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy().into_owned();
        if text == "--" {
            operands.extend(args.by_ref());
            break;
        }
        if let Some(long) = text.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((name, value)) => (name, Some(value.to_owned())),
                None => (long, None),
            };
            let option = format!("--{name}");
            match name {
                "help" => return Err(Early::Print(USAGE.into())),
                "version" => return Err(Early::Print(format!("grep (ut {})", env!("CARGO_PKG_VERSION")))),
                "extended-regexp" => o.syntax = Syntax::Extended,
                "fixed-strings" => o.syntax = Syntax::Fixed,
                "basic-regexp" => o.syntax = Syntax::Basic,
                "regexp" => {
                    o.patterns.push(value(&option, inline, &mut args)?);
                    pattern_given = true;
                }
                "file" => {
                    o.patterns.extend(patterns_from_file(&value(&option, inline, &mut args)?)?);
                    pattern_given = true;
                }
                "ignore-case" => o.ignore_case = true,
                "no-ignore-case" => o.ignore_case = false,
                "invert-match" => o.invert = true,
                "word-regexp" => o.word = true,
                "line-regexp" => o.whole_line = true,
                "count" => o.count = true,
                "files-with-matches" => o.files_with_matches = true,
                "files-without-match" => o.files_without_match = true,
                "quiet" | "silent" => o.quiet = true,
                "no-messages" => o.no_messages = true,
                "only-matching" => o.only_matching = true,
                "line-number" => o.line_number = true,
                "byte-offset" => o.byte_offset = true,
                "with-filename" => o.with_filename = Some(true),
                "no-filename" => o.with_filename = Some(false),
                "null" => o.null_after_name = true,
                "max-count" => o.max_count = Some(parse_number(&option, &value(&option, inline, &mut args)?)? as u64),
                "after-context" => o.after = parse_number(&option, &value(&option, inline, &mut args)?)?,
                "before-context" => o.before = parse_number(&option, &value(&option, inline, &mut args)?)?,
                "context" => {
                    let lines = parse_number(&option, &value(&option, inline, &mut args)?)?;
                    (o.after, o.before) = (lines, lines);
                }
                "recursive" => o.recursive = true,
                "dereference-recursive" => (o.recursive, o.follow_links) = (true, true),
                "text" => o.text = true,
                "line-buffered" => o.line_buffered = true,
                "include" => o.include.push(value(&option, inline, &mut args)?),
                "exclude" => o.exclude.push(value(&option, inline, &mut args)?),
                "exclude-dir" => o.exclude_dir.push(value(&option, inline, &mut args)?),
                "color" | "colour" => {
                    o.color = match inline.as_deref() {
                        None | Some("always") => true,
                        Some("never") => false,
                        Some("auto") => io::stdout().is_terminal(),
                        Some(other) => return Err(Early::Usage(format!("{other}: invalid argument for {option}"))),
                    }
                }
                "perl-regexp" | "null-data" => return Err(Early::Usage(format!("{option} is not supported"))),
                _ => return Err(Early::Usage(format!("unrecognized option '{option}'"))),
            }
            continue;
        }
        if text.len() > 1 && text.starts_with('-') {
            let letters: Vec<char> = text[1..].chars().collect();
            let mut at = 0;
            while at < letters.len() {
                let letter = letters[at];
                at += 1;
                // For a letter that takes a value: the rest of this argument, if any.
                let mut inline = || {
                    let rest: String = letters[at..].iter().collect();
                    at = letters.len();
                    (!rest.is_empty()).then_some(rest)
                };
                let option = format!("-{letter}");
                match letter {
                    'E' => o.syntax = Syntax::Extended,
                    'F' => o.syntax = Syntax::Fixed,
                    'G' => o.syntax = Syntax::Basic,
                    'e' => {
                        o.patterns.push(value(&option, inline(), &mut args)?);
                        pattern_given = true;
                    }
                    'f' => {
                        o.patterns.extend(patterns_from_file(&value(&option, inline(), &mut args)?)?);
                        pattern_given = true;
                    }
                    'i' | 'y' => o.ignore_case = true,
                    'v' => o.invert = true,
                    'w' => o.word = true,
                    'x' => o.whole_line = true,
                    'c' => o.count = true,
                    'l' => o.files_with_matches = true,
                    'L' => o.files_without_match = true,
                    'q' => o.quiet = true,
                    's' => o.no_messages = true,
                    'o' => o.only_matching = true,
                    'n' => o.line_number = true,
                    'b' => o.byte_offset = true,
                    'H' => o.with_filename = Some(true),
                    'h' => o.with_filename = Some(false),
                    'Z' => o.null_after_name = true,
                    'm' => o.max_count = Some(parse_number(&option, &value(&option, inline(), &mut args)?)? as u64),
                    'A' => o.after = parse_number(&option, &value(&option, inline(), &mut args)?)?,
                    'B' => o.before = parse_number(&option, &value(&option, inline(), &mut args)?)?,
                    'C' => {
                        let lines = parse_number(&option, &value(&option, inline(), &mut args)?)?;
                        (o.after, o.before) = (lines, lines);
                    }
                    'r' => o.recursive = true,
                    'R' => (o.recursive, o.follow_links) = (true, true),
                    'a' => o.text = true,
                    'I' => o.skip_binary = true,
                    'V' => return Err(Early::Print(format!("grep (ut {})", env!("CARGO_PKG_VERSION")))),
                    'P' | 'z' => return Err(Early::Usage(format!("{option} is not supported"))),
                    _ => return Err(Early::Usage(format!("invalid option -- '{letter}'"))),
                }
            }
            continue;
        }
        operands.push(arg);
    }
    let mut operands = operands.into_iter();
    if !pattern_given {
        let pattern = operands.next().ok_or_else(|| Early::Usage("no pattern given".into()))?;
        o.patterns.push(pattern.to_string_lossy().into_owned());
    }
    o.files = operands.collect();
    Ok(o)
}

/// A basic regular expression as an extended one. In a basic expression
/// `( ) { } | + ?` are ordinary characters and become operators with a
/// backslash before them; in an extended one it is the other way around.
/// Bracket expressions (`[...]`) are the same in both and pass unchanged.
fn basic_to_extended(pattern: &str) -> String {
    const SWAPPED: &str = "(){}|+?";
    let mut out = String::with_capacity(pattern.len());
    let mut chars = pattern.chars().peekable();
    let mut at_start = true;
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(next) if SWAPPED.contains(next) => out.push(next),
                Some(next) => out.extend(['\\', next]),
                None => out.push_str("\\\\"),
            },
            '[' => {
                out.push('[');
                // `]` first in the brackets (after an optional `^`) is itself.
                if chars.peek() == Some(&'^') {
                    out.extend(chars.next());
                }
                if chars.peek() == Some(&']') {
                    out.push_str("\\]");
                    chars.next();
                }
                while let Some(inner) = chars.next() {
                    // `[:alpha:]`, `[.x.]`, `[=x=]` nest one more pair of brackets.
                    if inner == '[' && matches!(chars.peek(), Some(':' | '.' | '=')) {
                        out.push('[');
                        for class in chars.by_ref() {
                            out.push(class);
                            if class == ']' {
                                break;
                            }
                        }
                        continue;
                    }
                    out.push(inner);
                    if inner == ']' {
                        break;
                    }
                }
            }
            // A star with nothing before it to repeat is an ordinary star.
            '*' if at_start => out.push_str("\\*"),
            c if SWAPPED.contains(c) => out.extend(['\\', c]),
            c => out.push(c),
        }
        // After a group's opening or an alternation a star is again at a start.
        at_start = out.ends_with('(') && !out.ends_with("\\(") || out.ends_with('|') && !out.ends_with("\\|");
    }
    out
}

fn matcher(o: &Options) -> Result<RegexMatcher, String> {
    // A pattern with line ends in it is several patterns.
    let patterns: Vec<String> = o
        .patterns
        .iter()
        .flat_map(|p| p.split('\n'))
        .map(|p| if o.syntax == Syntax::Basic { basic_to_extended(p) } else { p.to_owned() })
        .collect();
    RegexMatcherBuilder::new()
        .case_insensitive(o.ignore_case)
        .word(o.word)
        .whole_line(o.whole_line)
        .fixed_strings(o.syntax == Syntax::Fixed)
        .line_terminator(Some(b'\n'))
        .build_many(&patterns)
        .map_err(|error| error.to_string())
}

/// An I/O error as GNU grep words it, the same on every system: the
/// system's own text differs by platform and by language.
fn message(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::NotFound => "No such file or directory".into(),
        io::ErrorKind::PermissionDenied => "Permission denied".into(),
        io::ErrorKind::IsADirectory => "Is a directory".into(),
        io::ErrorKind::InvalidData => "Invalid data".into(),
        _ => {
            let text = error.to_string();
            text.split(" (os error").next().unwrap_or(&text).to_owned()
        }
    }
}

/// What is searched: a file, or standard input.
enum Source {
    File(PathBuf),
    Stdin,
}

/// Writes the selected lines of one source in GNU grep's form.
struct Printer<'a, W: Write> {
    out: &'a mut W,
    o: &'a Options,
    matcher: &'a RegexMatcher,
    /// The source's name as printed before each line, if names are printed.
    name: Option<&'a [u8]>,
    /// Lines are counted and not printed (`-c`, `-l`, `-L`, `-q`, a binary file).
    silent: bool,
    selected: u64,
    /// Bytes that are not text were met (standard input, which is not probed first).
    binary: bool,
}

impl<W: Write> Printer<'_, W> {
    /// A piece of text in a color, if colors are on.
    fn colored(&mut self, color: &str, text: &[u8]) -> io::Result<()> {
        if self.o.color {
            self.out.write_all(color.as_bytes())?;
            self.out.write_all(text)?;
            self.out.write_all(COLOR_END.as_bytes())
        } else {
            self.out.write_all(text)
        }
    }

    /// What comes before a line's text: `name:`, `number:`, `offset:`, as asked for.
    fn prefix(&mut self, separator: u8, line: Option<u64>, offset: u64) -> io::Result<()> {
        let separator = [separator];
        if let Some(name) = self.name {
            self.colored(COLOR_FILE, name)?;
            if self.o.null_after_name { self.out.write_all(b"\0")? } else { self.colored(COLOR_SEPARATOR, &separator)? }
        }
        if let (true, Some(line)) = (self.o.line_number, line) {
            self.colored(COLOR_LINE, line.to_string().as_bytes())?;
            self.colored(COLOR_SEPARATOR, &separator)?;
        }
        if self.o.byte_offset {
            self.colored(COLOR_LINE, offset.to_string().as_bytes())?;
            self.colored(COLOR_SEPARATOR, &separator)?;
        }
        Ok(())
    }

    /// The places in `line` that match, as byte ranges.
    fn matches(&self, line: &[u8]) -> Vec<(usize, usize)> {
        let mut found = Vec::new();
        // The matcher cannot fail on a line it has already searched.
        let _ = self.matcher.find_iter(line, |m| {
            // An empty match selects a line but is nothing to show.
            if m.start() < m.end() {
                found.push((m.start(), m.end()));
            }
            true
        });
        found
    }

    /// One line of output: its prefix, then its text with the matches in color.
    fn line(&mut self, separator: u8, number: Option<u64>, offset: u64, bytes: &[u8]) -> io::Result<()> {
        let text = bytes.strip_suffix(b"\n").unwrap_or(bytes);
        self.prefix(separator, number, offset)?;
        if self.o.color {
            let mut at = 0;
            for (start, end) in self.matches(text) {
                self.out.write_all(&text[at..start])?;
                self.colored(COLOR_MATCH, &text[start..end])?;
                at = end;
            }
            self.out.write_all(&text[at..])?;
        } else {
            self.out.write_all(text)?;
        }
        self.out.write_all(b"\n")?;
        if self.o.line_buffered { self.out.flush() } else { Ok(()) }
    }

    /// Whether the search goes on after a line was selected.
    fn more(&self) -> bool {
        self.o.max_count.is_none_or(|most| self.selected < most)
    }
}

impl<W: Write> Sink for Printer<'_, W> {
    type Error = io::Error;

    fn matched(&mut self, _: &Searcher, found: &SinkMatch<'_>) -> io::Result<bool> {
        self.selected += 1;
        if self.silent {
            // One selected line is all that `-q`, `-l` and `-L` need to know.
            let one_is_enough = self.o.quiet || self.o.files_with_matches || self.o.files_without_match;
            return Ok(!one_is_enough && self.more());
        }
        let (bytes, offset) = (found.bytes(), found.absolute_byte_offset());
        if self.o.only_matching {
            // With `-v` the selected lines have no match in them: nothing is printed.
            for (start, end) in self.matches(bytes.strip_suffix(b"\n").unwrap_or(bytes)) {
                self.prefix(b':', found.line_number(), offset + start as u64)?;
                self.colored(COLOR_MATCH, &bytes[start..end])?;
                self.out.write_all(b"\n")?;
            }
        } else {
            self.line(b':', found.line_number(), offset, bytes)?;
        }
        Ok(self.more())
    }

    fn context(&mut self, _: &Searcher, context: &SinkContext<'_>) -> io::Result<bool> {
        if !self.silent && !self.o.only_matching {
            self.line(b'-', context.line_number(), context.absolute_byte_offset(), context.bytes())?;
        }
        Ok(true)
    }

    fn context_break(&mut self, _: &Searcher) -> io::Result<bool> {
        if !self.silent && !self.o.only_matching {
            self.colored(COLOR_SEPARATOR, b"--")?;
            self.out.write_all(b"\n")?;
        }
        Ok(true)
    }

    fn binary_data(&mut self, _: &Searcher, _: u64) -> io::Result<bool> {
        self.binary = true;
        Ok(false)
    }
}

/// Whether the file's start has a NUL in it, which is how GNU grep tells a
/// binary file from text.
fn is_binary(file: &Path) -> io::Result<bool> {
    let mut start = Vec::with_capacity(BINARY_PROBE);
    std::fs::File::open(file)?.take(BINARY_PROBE as u64).read_to_end(&mut start)?;
    Ok(start.contains(&0))
}

/// What a search of every source found.
#[derive(Default)]
struct Outcome {
    selected: bool,
    failed: bool,
}

struct Grep<'a, W: Write> {
    o: &'a Options,
    matcher: RegexMatcher,
    out: &'a mut W,
    with_filename: bool,
    outcome: Outcome,
}

impl<W: Write> Grep<'_, W> {
    fn complain(&mut self, name: &str, problem: &str) {
        self.outcome.failed = true;
        if !self.o.no_messages {
            let _ = writeln!(io::stderr(), "grep: {name}: {problem}");
        }
    }

    /// A file's name alone on a line (`-l`, `-L`), ended as `-Z` says.
    fn name_line(&mut self, name: &[u8]) -> io::Result<()> {
        if self.o.color {
            self.out.write_all(COLOR_FILE.as_bytes())?;
            self.out.write_all(name)?;
            self.out.write_all(COLOR_END.as_bytes())?;
        } else {
            self.out.write_all(name)?;
        }
        self.out.write_all(if self.o.null_after_name { b"\0" } else { b"\n" })
    }

    /// Searches one source and prints what `-c`, `-l` and `-L` print after it.
    fn search(&mut self, source: &Source, name: &str) -> io::Result<()> {
        let binary_file = match source {
            Source::File(file) if !self.o.text => match is_binary(file) {
                Ok(binary) => binary,
                Err(error) => {
                    self.complain(name, &message(&error));
                    return Ok(());
                }
            },
            _ => false,
        };
        if binary_file && self.o.skip_binary {
            return Ok(());
        }
        let summary = self.o.count || self.o.files_with_matches || self.o.files_without_match || self.o.quiet;
        let mut builder = SearcherBuilder::new();
        builder.line_number(true).invert_match(self.o.invert);
        // Context is for printed lines: a count or a binary file has none.
        if !summary && !binary_file {
            builder.before_context(self.o.before).after_context(self.o.after);
        }
        // A file was probed above and is searched whole either way; standard
        // input is not, so the searcher stops at the first NUL it meets.
        let stop_at_nul = matches!(source, Source::Stdin) && !self.o.text;
        builder.binary_detection(if stop_at_nul { BinaryDetection::quit(0) } else { BinaryDetection::none() });
        let mut searcher = builder.build();
        let name_bytes = name.as_bytes();
        let mut printer = Printer {
            out: self.out,
            o: self.o,
            matcher: &self.matcher,
            name: self.with_filename.then_some(name_bytes),
            silent: summary || binary_file,
            selected: 0,
            binary: false,
        };
        let searched = match source {
            Source::File(file) => searcher.search_path(&self.matcher, file, &mut printer),
            Source::Stdin => searcher.search_reader(&self.matcher, io::stdin().lock(), &mut printer),
        };
        let (selected, binary) = (printer.selected, binary_file || printer.binary);
        if let Err(error) = searched {
            if error.kind() == io::ErrorKind::BrokenPipe {
                return Err(error);
            }
            self.complain(name, &message(&error));
            return Ok(());
        }
        self.outcome.selected |= selected > 0;
        if self.o.quiet {
            return Ok(());
        }
        if self.o.files_with_matches || self.o.files_without_match {
            if (selected > 0) == self.o.files_with_matches {
                self.name_line(name_bytes)?;
            }
        } else if self.o.count {
            if self.with_filename {
                self.out.write_all(name_bytes)?;
                self.out.write_all(if self.o.null_after_name { b"\0" } else { b":" })?;
            }
            writeln!(self.out, "{selected}")?;
        } else if binary && selected > 0 {
            // GNU grep 3.5 and later say this on stderr and print no line of the file.
            let _ = writeln!(io::stderr(), "grep: {name}: binary file matches");
        }
        Ok(())
    }
}

fn globs(patterns: &[String]) -> Result<Option<GlobSet>, String> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern).map_err(|error| error.to_string())?);
    }
    builder.build().map(Some).map_err(|error| error.to_string())
}

/// The name of a file found under a directory: the directory as it was
/// typed, then the way down with `/` between the parts on every system, so
/// that the same search prints the same bytes on Windows and on Linux.
/// `root`: `None` for the directory that is searched when none is named,
/// whose files are named without a `./`.
fn found_name(root: Option<&str>, below: &Path) -> String {
    let below: Vec<_> = below.components().map(|part| part.as_os_str().to_string_lossy()).collect();
    let below = below.join("/");
    match root {
        None => below,
        Some(root) => format!("{}/{below}", root.trim_end_matches(['/', '\\'])),
    }
}

fn run(o: &Options, out: &mut impl Write) -> Result<Outcome, String> {
    let (include, exclude, exclude_dir) = (globs(&o.include)?, globs(&o.exclude)?, globs(&o.exclude_dir)?);
    let wanted = |name: &OsStr| include.as_ref().is_none_or(|g| g.is_match(name)) && !exclude.as_ref().is_some_and(|g| g.is_match(name));
    // No file named: the current directory with `-r`, else standard input.
    let dot = OsString::from(".");
    let implied_dot = o.recursive && o.files.is_empty();
    let operands: Vec<&OsString> = if implied_dot { vec![&dot] } else { o.files.iter().collect() };
    let with_filename = o.with_filename.unwrap_or(o.recursive || operands.len() > 1);
    let mut grep = Grep { o, matcher: matcher(o)?, out, with_filename, outcome: Outcome::default() };

    let finish = |result: io::Result<()>| match result {
        // The reader stopped reading: nothing more to do, and no error.
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(false),
        Err(error) => Err(error.to_string()),
        Ok(()) => Ok(true),
    };
    if operands.is_empty() {
        finish(grep.search(&Source::Stdin, STDIN_LABEL))?;
        return Ok(grep.outcome);
    }
    for operand in operands {
        if operand == "-" {
            if !finish(grep.search(&Source::Stdin, STDIN_LABEL))? {
                break;
            }
            continue;
        }
        let path = Path::new(operand);
        // A file the user named is called what the user typed.
        let typed = operand.to_string_lossy();
        if !path.is_dir() {
            if !finish(grep.search(&Source::File(path.to_path_buf()), &typed))? {
                break;
            }
            continue;
        }
        if !o.recursive {
            grep.complain(&typed, "Is a directory");
            continue;
        }
        // In name order, so that the output is the same on every system and every run.
        let walk = WalkDir::new(path).follow_links(o.follow_links).sort_by_file_name().into_iter();
        let walk = walk.filter_entry(|entry| {
            entry.depth() == 0 || !(entry.file_type().is_dir() && exclude_dir.as_ref().is_some_and(|g| g.is_match(entry.file_name())))
        });
        for entry in walk {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    let name = error.path().map(|path| path.to_string_lossy().into_owned()).unwrap_or_default();
                    let problem = error.io_error().map_or_else(|| error.to_string(), message);
                    grep.complain(&name, &problem);
                    continue;
                }
            };
            if !entry.file_type().is_file() || !wanted(entry.file_name()) {
                continue;
            }
            let below = entry.path().strip_prefix(path).unwrap_or(entry.path());
            let name = found_name((!implied_dot).then_some(&*typed), below);
            if !finish(grep.search(&Source::File(entry.path().to_path_buf()), &name))? {
                return Ok(grep.outcome);
            }
        }
    }
    Ok(grep.outcome)
}

pub fn main(args: Vec<OsString>) -> i32 {
    let options = match parse(args) {
        Ok(options) => options,
        Err(Early::Print(text)) => {
            let _ = writeln!(io::stdout(), "{text}");
            return SELECTED;
        }
        Err(Early::Usage(problem)) => {
            let _ = writeln!(io::stderr(), "grep: {problem}\nTry 'grep --help' for more information.");
            return TROUBLE;
        }
    };
    let stdout = io::stdout();
    // To a terminal each line is written as it is found; to a file or a pipe, in blocks.
    let mut out: Box<dyn Write> =
        if stdout.is_terminal() { Box::new(io::LineWriter::new(stdout.lock())) } else { Box::new(io::BufWriter::new(stdout.lock())) };
    let outcome = match run(&options, &mut out) {
        Ok(outcome) => outcome,
        Err(problem) => {
            let _ = out.flush();
            let _ = writeln!(io::stderr(), "grep: {problem}");
            return TROUBLE;
        }
    };
    let _ = out.flush();
    // With -q a selected line is success even when a file could not be read.
    if outcome.failed && !(options.quiet && outcome.selected) {
        TROUBLE
    } else if outcome.selected {
        SELECTED
    } else {
        NONE_SELECTED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of this test's own with a few files in it.
    fn scratch(test: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ut-grep-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub/deep")).unwrap();
        std::fs::write(root.join("a.txt"), "alpha\nbeta gamma\nAlpha two\n\nlast line").unwrap();
        std::fs::write(root.join("b.log"), "beta\nnothing here\n").unwrap();
        std::fs::write(root.join("sub/c.txt"), "gamma\nalpha in sub\n").unwrap();
        std::fs::write(root.join("sub/deep/d.txt"), "alpha deep\n").unwrap();
        std::fs::write(root.join("bin.dat"), b"alpha\0binary\n").unwrap();
        root
    }

    /// Runs grep with `args` in `dir`'s files (named relative to it) and
    /// gives its output and exit code.
    fn grep(dir: &Path, args: &[&str]) -> (String, i32) {
        let mut full = vec![OsString::from("grep")];
        for arg in args {
            let path = dir.join(arg);
            // An argument that names something in the directory is that path.
            full.push(if !arg.is_empty() && !arg.starts_with('-') && path.exists() { path.into_os_string() } else { OsString::from(arg) });
        }
        let options = parse(full).map_err(|_| "bad arguments").unwrap();
        let mut out = Vec::new();
        let outcome = run(&options, &mut out).unwrap();
        let code = if outcome.failed && !(options.quiet && outcome.selected) {
            TROUBLE
        } else if outcome.selected {
            SELECTED
        } else {
            NONE_SELECTED
        };
        // The directory's own path is not part of what is checked.
        let dir = dir.to_string_lossy();
        let text = String::from_utf8(out).unwrap().replace(&format!("{dir}{}", std::path::MAIN_SEPARATOR), "").replace(&format!("{dir}/"), "");
        (text, code)
    }

    #[test]
    fn basic_expressions_become_extended_ones() {
        for (basic, extended) in [
            ("a(b)", r"a\(b\)"),
            (r"a\(b\)", "a(b)"),
            ("a|b", r"a\|b"),
            (r"a\|b", "a|b"),
            ("a+", r"a\+"),
            (r"a\+", "a+"),
            (r"a\{2,3\}", "a{2,3}"),
            ("a{2}", r"a\{2\}"),
            ("*a", r"\*a"),
            (r"\(*a\)", r"(\*a)"),
            ("a*", "a*"),
            ("[(|)]", "[(|)]"),
            ("[]a]", r"[\]a]"),
            ("[^]a]", r"[^\]a]"),
            ("[[:alpha:]+]", "[[:alpha:]+]"),
            (r"a\.b", r"a\.b"),
            ("a\\", "a\\\\"),
        ] {
            assert_eq!(basic_to_extended(basic), extended, "{basic}");
        }
    }

    #[test]
    fn lines_are_selected_and_named_as_gnu_grep_does() {
        let dir = scratch("plain");
        assert_eq!(grep(&dir, &["alpha", "a.txt"]), ("alpha\n".into(), 0));
        assert_eq!(grep(&dir, &["-i", "alpha", "a.txt"]), ("alpha\nAlpha two\n".into(), 0));
        assert_eq!(grep(&dir, &["-n", "beta", "a.txt", "b.log"]), ("a.txt:2:beta gamma\nb.log:1:beta\n".into(), 0));
        assert_eq!(grep(&dir, &["-h", "beta", "a.txt", "b.log"]), ("beta gamma\nbeta\n".into(), 0));
        assert_eq!(grep(&dir, &["-H", "alpha", "a.txt"]), ("a.txt:alpha\n".into(), 0));
        assert_eq!(grep(&dir, &["zzz", "a.txt"]), (String::new(), 1));
        assert_eq!(grep(&dir, &["-v", "a", "a.txt"]), ("\n".into(), 0), "only the empty line has no a");
        assert_eq!(grep(&dir, &["line", "a.txt"]), ("last line\n".into(), 0), "a last line with no line end gets one");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn counts_names_and_parts() {
        let dir = scratch("summary");
        assert_eq!(grep(&dir, &["-c", "a", "a.txt"]), ("4\n".into(), 0));
        assert_eq!(grep(&dir, &["-c", "beta", "a.txt", "b.log"]), ("a.txt:1\nb.log:1\n".into(), 0));
        assert_eq!(grep(&dir, &["-c", "zzz", "a.txt"]), ("0\n".into(), 1));
        assert_eq!(grep(&dir, &["-l", "gamma", "a.txt", "b.log"]), ("a.txt\n".into(), 0));
        assert_eq!(grep(&dir, &["-L", "gamma", "a.txt", "b.log"]), ("b.log\n".into(), 0));
        assert_eq!(grep(&dir, &["-o", "-i", "alpha", "a.txt"]), ("alpha\nAlpha\n".into(), 0));
        assert_eq!(grep(&dir, &["-on", "a", "b.log"]), ("1:a\n".into(), 0));
        assert_eq!(grep(&dir, &["-q", "alpha", "a.txt"]), (String::new(), 0));
        assert_eq!(grep(&dir, &["-m1", "a", "a.txt"]), ("alpha\n".into(), 0));
        assert_eq!(grep(&dir, &["-b", "beta", "a.txt"]), ("6:beta gamma\n".into(), 0));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn context_lines_and_group_breaks() {
        let dir = scratch("context");
        std::fs::write(dir.join("n.txt"), "1\n2\nhit\n4\n5\n6\n7\nhit\n9\n").unwrap();
        assert_eq!(grep(&dir, &["-n", "-A1", "hit", "n.txt"]), ("3:hit\n4-4\n--\n8:hit\n9-9\n".into(), 0));
        assert_eq!(grep(&dir, &["-B1", "hit", "n.txt"]), ("2\nhit\n--\n7\nhit\n".into(), 0));
        assert_eq!(grep(&dir, &["-C", "3", "hit", "n.txt"]), ("1\n2\nhit\n4\n5\n6\n7\nhit\n9\n".into(), 0), "groups that touch are one");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn kinds_of_pattern() {
        let dir = scratch("patterns");
        assert_eq!(grep(&dir, &["-E", "alpha|gamma", "sub/c.txt"]), ("gamma\nalpha in sub\n".into(), 0));
        assert_eq!(grep(&dir, &["alpha\\|gamma", "sub/c.txt"]), ("gamma\nalpha in sub\n".into(), 0), "basic: \\| is or");
        assert_eq!(grep(&dir, &["alpha|gamma", "sub/c.txt"]), (String::new(), 1), "basic: | is itself");
        assert_eq!(grep(&dir, &["-F", "a.", "a.txt"]), (String::new(), 1), "fixed: the dot is a dot");
        assert_eq!(grep(&dir, &["-w", "alph", "a.txt"]), (String::new(), 1));
        assert_eq!(grep(&dir, &["-x", "alpha", "a.txt"]), ("alpha\n".into(), 0));
        assert_eq!(grep(&dir, &["-e", "last", "-e", "^beta", "a.txt"]), ("beta gamma\nlast line\n".into(), 0));
        assert_eq!(grep(&dir, &["-c", "", "b.log"]), ("2\n".into(), 0), "the empty pattern matches every line");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn directories_are_searched_in_name_order() {
        let dir = scratch("recursive");
        let (text, code) = grep(&dir, &["-rn", "alpha", "sub"]);
        assert_eq!((text.as_str(), code), ("sub/c.txt:2:alpha in sub\nsub/deep/d.txt:1:alpha deep\n", 0));
        let (text, _) = grep(&dir, &["-rl", "--include=*.txt", "alpha", "."]);
        assert_eq!(text, "./a.txt\n./sub/c.txt\n./sub/deep/d.txt\n", "the binary file and the .log are not .txt");
        let (text, _) = grep(&dir, &["-rl", "--exclude-dir=deep", "--exclude=*.dat", "alpha", "."]);
        assert_eq!(text, "./a.txt\n./sub/c.txt\n");
        let (text, code) = grep(&dir, &["alpha", "sub"]);
        assert_eq!((text.as_str(), code), ("", 2), "a directory without -r is an error");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn found_files_are_named_with_slashes() {
        assert_eq!(found_name(Some("src"), Path::new("a.rs")), "src/a.rs");
        assert_eq!(found_name(Some("src/"), &Path::new("sub").join("deep").join("a.rs")), "src/sub/deep/a.rs");
        assert_eq!(found_name(Some(r"C:\work\"), &Path::new("sub").join("a.rs")), r"C:\work/sub/a.rs", "the typed part stays as typed");
        assert_eq!(found_name(Some("."), Path::new("a.rs")), "./a.rs");
        assert_eq!(found_name(None, &Path::new("sub").join("a.rs")), "sub/a.rs", "no directory named: no ./");
    }

    #[test]
    fn binary_files_and_missing_files() {
        let dir = scratch("binary");
        assert_eq!(grep(&dir, &["alpha", "bin.dat"]), (String::new(), 0), "a match, said on stderr, and no line printed");
        assert_eq!(grep(&dir, &["-c", "alpha", "bin.dat"]), ("1\n".into(), 0));
        assert_eq!(grep(&dir, &["-a", "-c", "binary", "bin.dat"]), ("1\n".into(), 0));
        assert_eq!(grep(&dir, &["-I", "alpha", "bin.dat"]), (String::new(), 1), "skipped");
        assert_eq!(grep(&dir, &["-s", "alpha", "no-such-file"]), (String::new(), 2));
        assert_eq!(grep(&dir, &["-sq", "alpha", "no-such-file", "a.txt"]), (String::new(), 0), "-q: found is success");
        assert_eq!(grep(&dir, &["-s", "alpha", "no-such-file", "a.txt"]).1, 2, "without -q the error counts");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn colors_are_gnu_greps() {
        let dir = scratch("color");
        let (text, _) = grep(&dir, &["--color=always", "-n", "gamma", "a.txt", "b.log"]);
        assert_eq!(
            text,
            "\x1b[35m\x1b[Ka.txt\x1b[m\x1b[K\x1b[36m\x1b[K:\x1b[m\x1b[K\x1b[32m\x1b[K2\x1b[m\x1b[K\x1b[36m\x1b[K:\x1b[m\x1b[Kbeta \x1b[01;31m\x1b[Kgamma\x1b[m\x1b[K\n"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn mistakes_in_the_options_are_named() {
        let problem = |args: &[&str]| match parse(std::iter::once("grep").chain(args.iter().copied()).map(OsString::from).collect()) {
            Err(Early::Usage(problem)) => problem,
            _ => panic!("no usage error for {args:?}"),
        };
        assert_eq!(problem(&[]), "no pattern given");
        assert_eq!(problem(&["-A"]), "option -A requires an argument");
        assert_eq!(problem(&["-A", "x", "p"]), "x: invalid number for -A");
        assert_eq!(problem(&["--nope", "p"]), "unrecognized option '--nope'");
        assert_eq!(problem(&["-P", "p"]), "-P is not supported");
        assert_eq!(problem(&["-Q", "p"]), "invalid option -- 'Q'");
        let options = parse(["grep", "-rniA2", "--", "-pattern", "f1", "f2"].map(OsString::from).to_vec()).map_err(|_| ()).unwrap();
        assert!(options.recursive && options.line_number && options.ignore_case && options.after == 2);
        assert_eq!((options.patterns, options.files.len()), (vec!["-pattern".to_owned()], 2));
    }
}
