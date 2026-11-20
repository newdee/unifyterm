//! Wildcards in a command's arguments, on Windows.
//!
//! On Unix the shell turns `*.txt` into the files' names before a command
//! starts. PowerShell and cmd.exe do not: the command gets `*.txt`. So on
//! Windows `ut` does what the shell does elsewhere: an argument with `*`,
//! `?` or `[...]` in it becomes the names it matches, ordered by their
//! bytes; one that matches nothing stays as it was typed.
//!
//! An argument in quotes is not a pattern (`find . -name "*.rs"`). cmd.exe
//! passes the quotes on, and they are seen here. PowerShell takes them
//! away, so UnifyTerm's PowerShell start script puts [`LITERAL`] first in a
//! quoted argument instead; it is taken off here and the rest is left alone.
//! A command PowerShell runs with `-Command` (an agent's, a script's) does not
//! pass through that reader, so the value of an option that always takes a
//! pattern (`find -name`, `grep --include`: `PATTERN_OPTIONS` below) is
//! taken as typed, quoted or not.
//!
//! A program that starts a command (`find -exec`, `xargs`, `env`) gives it
//! its arguments as they are meant, `*` and all: no shell is in between.
//! So each `ut` command sets [`AS_GIVEN`] for the programs it starts, and a
//! command that finds it set replaces no wildcard. The user can set it too.

use std::ffi::OsString;

/// Set (to anything): the arguments are taken as they are given, with no
/// wildcard replaced. Each `ut` command sets it for the programs it starts.
pub(crate) const AS_GIVEN: &str = "UT_NO_GLOB";

/// First in an argument: the rest is to be taken as it is, not as a
/// pattern. A character from the private-use area, which no file's name and
/// no option begins with. UnifyTerm's PowerShell start script writes the same one.
pub(crate) const LITERAL: char = '\u{E000}';

/// This program's arguments, its own name first: on Windows with the
/// wildcards replaced by what they match, unless [`AS_GIVEN`] is set, and
/// with the mark [`LITERAL`] taken off either way.
pub(crate) fn arguments() -> Vec<OsString> {
    #[cfg(windows)]
    if std::env::var_os(AS_GIVEN).is_none() {
        return windows::arguments();
    }
    std::env::args_os().enumerate().map(|(at, argument)| if at == 0 { argument } else { unmarked(argument) }).collect()
}

/// An argument without the mark [`LITERAL`] in front, if it has it.
fn unmarked(argument: OsString) -> OsString {
    match argument.to_str().and_then(|text| text.strip_prefix(LITERAL)) {
        Some(rest) => OsString::from(rest),
        None => argument,
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    use wild::parser::{CharCode, CommandLineWParser};

    use super::LITERAL;

    /// Whether a unit is one of the characters that make an argument a pattern.
    fn is_wildcard(unit: u16) -> bool {
        b"*?[]".map(u16::from).contains(&unit)
    }

    pub(super) fn arguments() -> Vec<OsString> {
        // The program's own name as the system gives it; the rest from the command line as it was typed.
        let name = std::env::args_os().next().unwrap_or_default();
        let mut all = vec![name];
        all.extend(expand(&command_line()).into_iter().skip(1));
        all
    }

    /// The command line this process was started with, quotes and all.
    fn command_line() -> Vec<u16> {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCommandLineW() -> *const u16;
        }
        // SAFETY: GetCommandLineW has no preconditions.
        let start = unsafe { GetCommandLineW() };
        let mut length = 0;
        // SAFETY: the system's string ends with a NUL and lives as long as the
        // process, so every unit up to and with that NUL can be read.
        while unsafe { start.wrapping_add(length).read() } != 0 {
            length += 1;
        }
        // SAFETY: the `length` units before the NUL were just read one by one.
        unsafe { std::slice::from_raw_parts(start, length) }.to_vec()
    }

    /// Options whose value is always a pattern for the command to match, never
    /// files: `find`'s tests by name and `grep`'s file filters. The argument
    /// after one is taken as typed. This is what lets `find . -name "*.rs"` work
    /// where nothing marks the quotes: a command an agent or a script gives
    /// PowerShell (`-Command`) does not pass through the start script's reader.
    /// (`--include=*.rs` needs no help: no file has that name.)
    const PATTERN_OPTIONS: [&str; 13] = [
        "-name",
        "-iname",
        "-path",
        "-ipath",
        "-wholename",
        "-iwholename",
        "-regex",
        "-iregex",
        "-lname",
        "-ilname",
        "--include",
        "--exclude",
        "--exclude-dir",
    ];

    /// The arguments of a command line, the program's name first, patterns
    /// replaced by what they match.
    pub(super) fn expand(line: &[u16]) -> Vec<OsString> {
        let mut parser = CommandLineWParser::new(line);
        let mut all: Vec<OsString> = Vec::new();
        loop {
            // Each unit of the argument, and whether it was typed outside quotes.
            let mut units: Vec<(u16, bool)> = Vec::new();
            let more = parser.accumulate_next(|unit| match unit {
                CharCode::Quoted(unit) => units.push((unit, false)),
                CharCode::Unquoted(unit) => units.push((unit, true)),
            });
            if !more {
                return all;
            }
            let after_pattern_option =
                all.last().and_then(|last| last.to_str()).is_some_and(|last| PATTERN_OPTIONS.contains(&last));
            if all.is_empty() || after_pattern_option {
                // The program's own name is a name, whatever characters are in it; a pattern option's value is a pattern.
                let text = OsString::from_wide(&units.iter().map(|(unit, _)| *unit).collect::<Vec<u16>>());
                all.push(super::unmarked(text));
            } else {
                all.extend(expanded(&units));
            }
        }
    }

    /// One argument: itself, or the names it matches.
    fn expanded(units: &[(u16, bool)]) -> Vec<OsString> {
        let text: Vec<u16> = units.iter().map(|(unit, _)| *unit).collect();
        let mut marker = [0; 1];
        if let Some(rest) = text.strip_prefix(&*LITERAL.encode_utf16(&mut marker)) {
            return vec![OsString::from_wide(rest)];
        }
        let as_typed = || vec![OsString::from_wide(&text)];
        if !units.iter().any(|(unit, open)| *open && is_wildcard(*unit)) {
            return as_typed();
        }
        // In the pattern a wildcard typed inside quotes stands for itself: `[*]`.
        let mut pattern = String::new();
        for decoded in char::decode_utf16(units.iter().flat_map(|(unit, open)| {
            let escaped = !*open && is_wildcard(*unit);
            let bracket = |side: u8| escaped.then_some(u16::from(side));
            bracket(b'[').into_iter().chain([*unit]).chain(bracket(b']'))
        })) {
            let Ok(c) = decoded else { return as_typed() };
            pattern.push(c);
        }
        // Letters' case aside: the file system does not tell `A` from `a`.
        let options = glob::MatchOptions { case_sensitive: false, ..Default::default() };
        let Ok(found) = glob::glob_with(&pattern, options) else { return as_typed() };
        let (backslash, slash, dot) = (u16::from(b'\\'), u16::from(b'/'), u16::from(b'.'));
        let parts = |name: &[u16]| -> Vec<Vec<u16>> {
            name.split(|unit| *unit == backslash || *unit == slash).map(<[u16]>::to_vec).collect()
        };
        let typed = parts(&text);
        // The names are written the way the pattern was: with `/` unless a `\` was typed,
        // and with `./` in front when it was typed so (the library leaves it out).
        let slashes = !text.contains(&backslash);
        let here = text.get(..2).filter(|start| *start == [dot, slash] || *start == [dot, backslash]);
        let names: Vec<OsString> = found
            .filter_map(Result::ok)
            .map(|path| std::os::windows::ffi::OsStrExt::encode_wide(path.as_os_str()).collect::<Vec<u16>>())
            // As in a Unix shell, a name that begins with a dot is matched
            // only by a pattern that has the dot typed, part by part from the last.
            .filter(|name| {
                let hidden =
                    |(part, typed): (&Vec<u16>, &Vec<u16>)| part.first() == Some(&dot) && typed.first() != Some(&dot);
                !parts(name).iter().rev().zip(typed.iter().rev()).any(hidden)
            })
            // `.` and `..` are no names a pattern stands for (bash 5.2 leaves them out too).
            .filter(|name| parts(name).last().is_none_or(|last| *last != [dot] && *last != [dot, dot]))
            .map(|name| match here {
                Some(prefix) if !name.starts_with(prefix) => [prefix, &name].concat(),
                _ => name,
            })
            .map(|mut name| {
                if slashes {
                    for unit in &mut name {
                        if *unit == backslash {
                            *unit = slash;
                        }
                    }
                }
                OsString::from_wide(&name)
            })
            .collect();
        if names.is_empty() { as_typed() } else { names }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn line(text: &str) -> Vec<String> {
            let units: Vec<u16> = text.encode_utf16().collect();
            expand(&units).into_iter().map(|argument| argument.to_string_lossy().into_owned()).collect()
        }

        /// A directory with `a.txt`, `B.TXT`, `.hidden.txt`, `c.log` and `sub/d.txt`; its path with `/`.
        fn files(test: &str) -> (tempfile::TempDir, String) {
            let dir = tempfile::Builder::new().prefix(&format!("ut-expand-{test}-")).tempdir().unwrap();
            std::fs::create_dir(dir.path().join("sub")).unwrap();
            for name in ["a.txt", "B.TXT", ".hidden.txt", "c.log", "sub/d.txt"] {
                std::fs::write(dir.path().join(name), "").unwrap();
            }
            let path = dir.path().to_string_lossy().replace('\\', "/");
            (dir, path)
        }

        #[test]
        fn a_pattern_becomes_the_names_it_matches() {
            let (_dir, at) = files("matches");
            assert_eq!(
                line(&format!("ut ls {at}/*.txt -l")),
                ["ut", "ls", &format!("{at}/B.TXT"), &format!("{at}/a.txt"), "-l"],
                "ordered by their bytes (a shell's order in the C locale), letters' case aside, no name that begins with a dot"
            );
            assert_eq!(
                line(&format!("ut {at}/?.log {at}/[ac].txt")),
                ["ut", &format!("{at}/c.log"), &format!("{at}/a.txt")]
            );
            assert_eq!(line(&format!("ut {at}/*/*.txt")), ["ut", &format!("{at}/sub/d.txt")]);
            assert_eq!(
                line(&format!("ut {at}/.*.txt")),
                ["ut", &format!("{at}/.hidden.txt")],
                "a dot that is typed matches"
            );
            assert_eq!(line(&format!("ut {at}/.*")), ["ut", &format!("{at}/.hidden.txt")], "but not . and ..");
        }

        #[test]
        fn names_are_written_the_way_the_pattern_was() {
            // Relative to where the test runs: the crate's own directory.
            assert_eq!(line("ut ./Cargo.t*"), ["ut", "./Cargo.toml"], "./ stays in front");
            assert_eq!(line("ut .\\Cargo.t*"), ["ut", ".\\Cargo.toml"]);
            assert_eq!(line("ut src/mai*.rs"), ["ut", "src/main.rs"], "and is not put where it was not typed");
            let (_dir, at) = files("separators");
            let native = at.replace('/', "\\");
            assert_eq!(line(&format!("ut {native}\\*.log")), ["ut", &format!("{native}\\c.log")]);
            assert_eq!(line(&format!("ut {at}/*.log")), ["ut", &format!("{at}/c.log")]);
        }

        #[test]
        fn what_is_not_a_pattern_stays_as_typed() {
            let (_dir, at) = files("literal");
            let quoted = format!("ut find -name \"{at}/*.txt\" '{at}/*.txt'");
            assert_eq!(line(&quoted), ["ut", "find", "-name", &format!("{at}/*.txt"), &format!("'{at}/*.txt'")]);
            assert_eq!(
                line(&format!("ut {LITERAL}{at}/*.txt")),
                ["ut", &format!("{at}/*.txt")],
                "the mark is taken off"
            );
            assert_eq!(line(&format!("ut {LITERAL} \"{LITERAL}a b\"")), ["ut", "", "a b"]);
            assert_eq!(line(&format!("ut {at}/*.zzz")), ["ut", &format!("{at}/*.zzz")], "nothing matches: as typed");
            assert_eq!(line("ut .items[] a[ -- ] plain \"\""), ["ut", ".items[]", "a[", "--", "]", "plain", ""]);
            assert_eq!(line("ut"), ["ut"]);
            assert_eq!(
                line(&format!("{at}/*.txt x")),
                [format!("{at}/*.txt"), "x".to_owned()],
                "the program's name is never a pattern"
            );
        }

        /// What an agent's PowerShell (`-Command`) passes: the quotes are gone and nothing marks them.
        #[test]
        fn a_pattern_options_value_is_a_pattern_without_quotes() {
            let (_dir, at) = files("options");
            for option in PATTERN_OPTIONS {
                assert_eq!(
                    line(&format!("ut x {option} {at}/*.txt")),
                    ["ut", "x", option, &format!("{at}/*.txt")],
                    "{option}"
                );
            }
            assert_eq!(
                line(&format!("find {at} -iname {at}/*.TXT -o {at}/*.log")),
                ["find", &at, "-iname", &format!("{at}/*.TXT"), "-o", &format!("{at}/c.log")],
                "only the option's own value: the next pattern is replaced again"
            );
            assert_eq!(
                line(&format!("grep x --include {LITERAL}*.rs")),
                ["grep", "x", "--include", "*.rs"],
                "the mark is taken off there too"
            );
            assert_eq!(
                line(&format!("ut echo -e {at}/*.log")),
                ["ut", "echo", "-e", &format!("{at}/c.log")],
                "-e is no such option"
            );
        }

        #[test]
        fn quotes_inside_a_pattern_protect_only_what_is_in_them() {
            let (_dir, at) = files("mixed");
            // `"[ab]".txt` is the name `[ab].txt`, which is not there: as typed, quotes gone.
            assert_eq!(line(&format!("ut {at}/\"[ab]\".txt")), ["ut", &format!("{at}/[ab].txt")]);
            assert_eq!(line(&format!("ut \"{at}\"/*.log")), ["ut", &format!("{at}/c.log")]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_is_taken_off_and_nothing_else() {
        assert_eq!(unmarked(OsString::from(format!("{LITERAL}*.rs"))), "*.rs");
        assert_eq!(
            unmarked(OsString::from(format!("{LITERAL}{LITERAL}x"))),
            OsString::from(format!("{LITERAL}x")),
            "one mark only"
        );
        assert_eq!(
            unmarked(OsString::from(format!("a{LITERAL}"))),
            OsString::from(format!("a{LITERAL}")),
            "only in front"
        );
        assert_eq!(unmarked(OsString::from("*.rs")), "*.rs");
        assert_eq!(unmarked(OsString::new()), "");
    }
}
