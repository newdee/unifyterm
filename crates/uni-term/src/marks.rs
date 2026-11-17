//! The marks a shell's hook puts in its output: where the prompt starts
//! and ends, when a command starts and ends and how, what the command was,
//! which directory the shell is in. They are the standard ones other
//! terminals and keepane read too:
//!
//! | sequence                     | mark                                   |
//! |------------------------------|----------------------------------------|
//! | `OSC 133;A`                  | the prompt starts                      |
//! | `OSC 133;B`                  | the prompt ends; typing starts         |
//! | `OSC 133;C`                  | the command starts                     |
//! | `OSC 133;D[;code]`           | the command ended, with this exit code |
//! | `OSC 7;file://host/path`     | the directory                          |
//! | `OSC 9;9;path`               | the directory (Windows Terminal's)     |
//! | `OSC 633;E;text`             | the command's text (VS Code's)         |
//! | `OSC 633;P;Cwd=path`         | the directory (VS Code's)              |
//! | `OSC 633;P;Shell=name`       | which shell this is (uni-term's)       |
//!
//! Each ends with BEL or with ST (`ESC \`). [`Scanner`] finds them in the
//! bytes a program prints, whatever pieces the bytes come in. The bytes
//! are not changed: the screen's library passes over these sequences.

/// Where an operating system command (OSC) sequence starts: `ESC ]`.
const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
/// Cancel and substitute: they end a sequence that is under way, unfinished.
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
/// The longest sequence followed; a longer one is given up (its bytes still go to the screen).
const LONGEST: usize = 64 * 1024;

/// One mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mark {
    PromptStart,
    InputStart,
    CommandStart,
    /// `None`: the shell did not say how it ended.
    CommandEnd(Option<i32>),
    /// The shell's directory, with `/` for separators.
    Directory(String),
    CommandLine(String),
    Shell(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    /// In an OSC sequence; its text so far is in `payload`.
    Osc,
    /// An ESC inside an OSC sequence: `\` ends it.
    OscEscape,
    /// An OSC sequence too long to follow: its bytes are passed over until it ends.
    Skipping,
    SkippingEscape,
}

/// Finds marks in output that arrives in pieces.
#[derive(Debug)]
pub(crate) struct Scanner {
    state: State,
    payload: Vec<u8>,
}

impl Default for Scanner {
    fn default() -> Self {
        Self { state: State::Ground, payload: Vec::new() }
    }
}

impl Scanner {
    /// The marks that end in `bytes`, each with the offset just past its
    /// last byte. A mark begun in an earlier piece counts where it ends.
    pub(crate) fn scan(&mut self, bytes: &[u8]) -> Vec<(usize, Mark)> {
        let mut found = Vec::new();
        for (at, &byte) in bytes.iter().enumerate() {
            if let Some(mark) = self.step(byte) {
                found.push((at + 1, mark));
            }
        }
        found
    }

    fn step(&mut self, byte: u8) -> Option<Mark> {
        match self.state {
            State::Ground => {
                if byte == ESC {
                    self.state = State::Escape;
                }
            }
            State::Escape => {
                self.state = match byte {
                    b']' => {
                        self.payload.clear();
                        State::Osc
                    }
                    ESC => State::Escape,
                    _ => State::Ground,
                };
            }
            State::Osc => match byte {
                BEL => return self.finish(),
                ESC => self.state = State::OscEscape,
                CAN | SUB => self.state = State::Ground,
                _ if self.payload.len() >= LONGEST => {
                    self.payload.clear();
                    self.state = State::Skipping;
                }
                _ => self.payload.push(byte),
            },
            State::OscEscape => {
                if byte == b'\\' {
                    return self.finish();
                }
                // Not a terminator: the ESC begins something new.
                self.state = State::Escape;
                return self.step(byte);
            }
            State::Skipping => match byte {
                BEL | CAN | SUB => self.state = State::Ground,
                ESC => self.state = State::SkippingEscape,
                _ => {}
            },
            State::SkippingEscape => {
                self.state = if byte == b'\\' { State::Ground } else { State::Escape };
                if self.state == State::Escape {
                    return self.step(byte);
                }
            }
        }
        None
    }

    fn finish(&mut self) -> Option<Mark> {
        self.state = State::Ground;
        let payload = std::mem::take(&mut self.payload);
        parse(&String::from_utf8_lossy(&payload))
    }
}

/// The mark an OSC sequence's text is, if it is one.
fn parse(text: &str) -> Option<Mark> {
    let (number, rest) = text.split_once(';').unwrap_or((text, ""));
    match number {
        "133" => {
            let mut fields = rest.split(';');
            match fields.next()? {
                "A" => Some(Mark::PromptStart),
                "B" => Some(Mark::InputStart),
                "C" => Some(Mark::CommandStart),
                "D" => Some(Mark::CommandEnd(fields.next().and_then(|code| code.trim().parse().ok()))),
                _ => None,
            }
        }
        "7" => directory_from_url(rest).map(Mark::Directory),
        "9" => rest.strip_prefix("9;").map(|path| Mark::Directory(slashed(unquoted(path)))),
        "633" => {
            let (kind, value) = rest.split_once(';').unwrap_or((rest, ""));
            match kind {
                "E" => {
                    // A nonce may follow the text after a `;`, which the text escapes as \x3b.
                    let text = value.split(';').next().unwrap_or("");
                    Some(Mark::CommandLine(unescaped(text)))
                }
                "P" => {
                    let (key, value) = value.split_once('=')?;
                    match key {
                        "Cwd" => Some(Mark::Directory(slashed(&unescaped(value)))),
                        "Shell" => Some(Mark::Shell(value.to_owned())),
                        _ => None,
                    }
                }
                "A" => Some(Mark::PromptStart),
                "B" => Some(Mark::InputStart),
                "C" => Some(Mark::CommandStart),
                "D" => Some(Mark::CommandEnd(value.split(';').next().and_then(|code| code.trim().parse().ok()))),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `file://host/path` as a path with `/`: `/C:/x` (a Windows drive) becomes `C:/x`.
fn directory_from_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    let path = percent_decoded(path);
    let windows_drive = path.len() >= 3 && path.as_bytes()[0] == b'/' && path.as_bytes()[2] == b':';
    Some(slashed(if windows_drive { &path[1..] } else { &path }))
}

/// A path with `/` for every `\`.
fn slashed(path: &str) -> String {
    path.replace('\\', "/")
}

/// A path without quotes around it (Windows Terminal's form allows them).
fn unquoted(path: &str) -> &str {
    path.strip_prefix('"').and_then(|path| path.strip_suffix('"')).unwrap_or(path)
}

/// `%20` and the like as the bytes they stand for.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let pair = bytes.get(at + 1..at + 3).and_then(|hex| std::str::from_utf8(hex).ok());
        match (bytes[at], pair.and_then(|hex| u8::from_str_radix(hex, 16).ok())) {
            (b'%', Some(value)) => {
                out.push(value);
                at += 3;
            }
            (byte, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// VS Code's escaping undone: `\\` is `\`, `\xHH` is that byte.
fn unescaped(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\\' {
            if bytes.get(at + 1) == Some(&b'\\') {
                out.push(b'\\');
                at += 2;
                continue;
            }
            let hex = bytes.get(at + 2..at + 4).and_then(|hex| std::str::from_utf8(hex).ok());
            if bytes.get(at + 1) == Some(&b'x')
                && let Some(value) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok())
            {
                out.push(value);
                at += 4;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks(bytes: &[u8]) -> Vec<Mark> {
        Scanner::default().scan(bytes).into_iter().map(|(_, mark)| mark).collect()
    }

    #[test]
    fn every_kind_of_mark_is_read() {
        let cases: [(&[u8], Mark); 14] = [
            (b"\x1b]133;A\x07", Mark::PromptStart),
            (b"\x1b]133;B\x1b\\", Mark::InputStart),
            (b"\x1b]133;C\x07", Mark::CommandStart),
            (b"\x1b]133;D;0\x07", Mark::CommandEnd(Some(0))),
            (b"\x1b]133;D;-1073741510\x07", Mark::CommandEnd(Some(-1_073_741_510))),
            (b"\x1b]133;D\x07", Mark::CommandEnd(None)),
            (b"\x1b]7;file://host/C:/Users/me%20x\x07", Mark::Directory("C:/Users/me x".into())),
            (b"\x1b]7;file://host/home/me\x1b\\", Mark::Directory("/home/me".into())),
            (b"\x1b]9;9;C:\\Users\\me\x1b\\", Mark::Directory("C:/Users/me".into())),
            (b"\x1b]9;9;\"C:\\a b\"\x07", Mark::Directory("C:/a b".into())),
            (b"\x1b]633;E;ls -l \\x3b echo \\\\x\x07", Mark::CommandLine("ls -l ; echo \\x".into())),
            (b"\x1b]633;E;git status;nonce123\x07", Mark::CommandLine("git status".into())),
            (b"\x1b]633;P;Cwd=C:\\work\x07", Mark::Directory("C:/work".into())),
            (b"\x1b]633;P;Shell=pwsh\x07", Mark::Shell("pwsh".into())),
        ];
        for (bytes, mark) in cases {
            assert_eq!(marks(bytes), std::slice::from_ref(&mark), "{:?}", String::from_utf8_lossy(bytes));
        }
    }

    #[test]
    fn other_sequences_are_no_marks() {
        for bytes in [
            &b"\x1b]0;title\x07"[..],
            b"\x1b]133;Z\x07",
            b"\x1b]633;P;Other=1\x07",
            b"\x1b]9;4;1;50\x07",
            b"\x1b[31mred\x1b[0m",
            b"plain text ] 133;A",
        ] {
            assert!(marks(bytes).is_empty(), "{:?}", String::from_utf8_lossy(bytes));
        }
    }

    /// Prompt, typing, a command and its end, with text between.
    const SESSION: &[u8] = b"out\x1b]133;D;0\x07\x1b]133;A\x07PS C:/x> \x1b]133;B\x07ls\r\n\x1b]633;E;ls\x07\x1b]133;C\x07a b\r\n\x1b]133;D;2\x1b\\";

    #[test]
    fn pieces_of_any_size_give_the_same_marks_at_the_same_places() {
        let whole: Vec<(usize, Mark)> = Scanner::default().scan(SESSION);
        assert_eq!(whole.len(), 6);
        for size in [1, 2, 3, 7, 16] {
            let mut scanner = Scanner::default();
            let mut found = Vec::new();
            for (index, piece) in SESSION.chunks(size).enumerate() {
                found.extend(scanner.scan(piece).into_iter().map(|(end, mark)| (index * size + end, mark)));
            }
            assert_eq!(found, whole, "pieces of {size}");
        }
    }

    #[test]
    fn an_unfinished_or_endless_sequence_does_no_harm() {
        let mut scanner = Scanner::default();
        // Cancelled half way: the mark after it is still found.
        assert_eq!(scanner.scan(b"\x1b]133;A\x18\x1b]133;B\x07").len(), 1);
        // An ESC that is no terminator starts a new sequence.
        assert_eq!(marks(b"\x1b]133;A\x1b]133;C\x07"), [Mark::CommandStart]);
        // Longer than followed: given up, and what comes after is read.
        let mut long = b"\x1b]633;E;".to_vec();
        long.extend(std::iter::repeat_n(b'x', LONGEST + 10));
        long.extend(b"\x07\x1b]133;A\x07");
        assert_eq!(marks(&long), [Mark::PromptStart]);
        // Any bytes at all: no panic, the scanner's memory stays bounded.
        let mut number: u32 = 1;
        let noise: Vec<u8> = (0..1 << 20)
            .map(|_| {
                number ^= number << 13;
                number ^= number >> 17;
                number ^= number << 5;
                number.to_le_bytes()[0]
            })
            .collect();
        let mut scanner = Scanner::default();
        let _ = scanner.scan(&noise);
        assert!(scanner.payload.len() <= LONGEST);
    }
}
