//! Runs `ut`'s commands on a fixed set of files and compares every byte
//! they print, and every exit code, with a record kept in
//! `tests/golden/commands.txt`.
//!
//! The record is the same file on Windows, macOS and Linux: the test
//! passing on each is what "the same commands everywhere" means. Run with
//! `UT_BLESS=1` to write the record from what the commands print now (then
//! read the diff: the record is the claim).

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// One command of a pipeline: `ut`'s arguments, the command's name first.
type Step = &'static [&'static str];

struct Case {
    name: &'static str,
    /// What is typed on the first command's standard input.
    input: &'static str,
    /// The commands, each reading what the one before it printed.
    pipeline: &'static [Step],
    paths: Paths,
}

/// Whether what a case prints is paths that were found, not typed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Paths {
    /// No.
    Same,
    /// Yes (`find`): a `\` in them is the system's own separator, which
    /// `ut` is to print as `/` on every system.
    Native,
}

const fn case(name: &'static str, pipeline: &'static [Step]) -> Case {
    Case { name, input: "", pipeline, paths: Paths::Same }
}

const fn typed(name: &'static str, input: &'static str, pipeline: &'static [Step]) -> Case {
    Case { name, input, pipeline, paths: Paths::Same }
}

const fn native(name: &'static str, pipeline: &'static [Step]) -> Case {
    Case { name, input: "", pipeline, paths: Paths::Native }
}

const WORDS: &str = "banana\napple\ncherry\napple pie\nBanana\ndate\n";
const NUMBERS: &str = "10\n9\n100\n-3\n2.5\n";
const CSV: &str = "name,qty,price\nnut,3,0.50\nbolt,12,0.25\nwasher,100,0.01\n";
const JSON: &str = r#"{"items":[{"name":"cat","n":1},{"name":"jq","n":2},{"name":"view","n":3}],"ok":true}"#;
const CJK: &str = "中文一行\nsecond 行\n日本語とemoji 🚀\n";
const TABS: &str = "a\tb\tc\n1\t22\t333\n";
const LEFT: &str = "one\ntwo\nthree\nfour\n";
const RIGHT: &str = "one\nTWO\nthree\nfive\nsix\n";

/// The files every case runs among, with `/` in their names for directories.
const FILES: &[(&str, &str)] = &[
    ("words.txt", WORDS),
    ("numbers.txt", NUMBERS),
    ("table.csv", CSV),
    ("data.json", JSON),
    ("cjk.txt", CJK),
    ("tabs.txt", TABS),
    ("left.txt", LEFT),
    ("right.txt", RIGHT),
    ("tree/a.txt", "alpha\nbeta\n"),
    ("tree/sub/b.txt", "beta\ngamma\n"),
    ("tree/sub/deep/c.log", "alpha in a log\n"),
    ("tree/notes.md", "# alpha\n"),
];

#[rustfmt::skip]
const CASES: &[Case] = &[
    case("cat", &[&["cat", "words.txt"]]),
    case("cat -n", &[&["cat", "-n", "left.txt"]]),
    case("cat two files", &[&["cat", "left.txt", "right.txt"]]),
    case("head -n 3", &[&["head", "-n", "3", "words.txt"]]),
    case("tail -n 2", &[&["tail", "-n", "2", "words.txt"]]),
    case("wc", &[&["wc", "words.txt"]]),
    case("wc -l -w -c", &[&["wc", "-l", "-w", "-c", "left.txt", "right.txt"]]),
    case("wc -m cjk", &[&["wc", "-m", "cjk.txt"]]),
    case("sort", &[&["sort", "words.txt"]]),
    case("sort -r", &[&["sort", "-r", "words.txt"]]),
    case("sort -f", &[&["sort", "-f", "words.txt"]]),
    case("sort -n", &[&["sort", "-n", "numbers.txt"]]),
    case("sort | uniq -c", &[&["sort", "words.txt"], &["uniq", "-c"]]),
    case("cut -d, -f", &[&["cut", "-d,", "-f1,3", "table.csv"]]),
    case("cut -c cjk", &[&["cut", "-c1-2", "cjk.txt"]]),
    case("tr upper", &[&["cat", "words.txt"], &["tr", "a-z", "A-Z"]]),
    case("tr -d", &[&["cat", "words.txt"], &["tr", "-d", "aeiou"]]),
    case("seq", &[&["seq", "1", "5"]]),
    case("seq -w", &[&["seq", "-w", "8", "11"]]),
    case("printf", &[&["printf", "%s-%03d\\n", "a", "7", "b", "42"]]),
    case("echo -n", &[&["echo", "-n", "no line end"]]),
    case("basename", &[&["basename", "/x/y/z.txt", ".txt"]]),
    case("dirname", &[&["dirname", "/x/y/z.txt"]]),
    case("base64", &[&["base64", "left.txt"]]),
    case("base64 round trip", &[&["base64", "cjk.txt"], &["base64", "-d"]]),
    case("md5sum", &[&["md5sum", "words.txt"]]),
    case("sha256sum", &[&["sha256sum", "words.txt", "cjk.txt"]]),
    case("nl", &[&["nl", "left.txt"]]),
    case("tac", &[&["tac", "left.txt"]]),
    case("paste", &[&["paste", "left.txt", "right.txt"]]),
    case("comm", &[&["comm", "left.txt", "right.txt"]]),
    case("expand", &[&["expand", "-t", "4", "tabs.txt"]]),
    case("fold", &[&["fold", "-w", "5", "left.txt"]]),
    case("od -c", &[&["head", "-n", "1", "tabs.txt"], &["od", "-c"]]),
    case("expr", &[&["expr", "3", "+", "4", "*", "5"]]),
    case("factor", &[&["factor", "360", "97"]]),
    case("numfmt", &[&["numfmt", "--to=iec", "1048576", "1500"]]),
    case("test true", &[&["test", "5", "-gt", "3"]]),
    case("test false", &[&["test", "5", "-lt", "3"]]),
    case("[ ]", &[&["[", "-f", "words.txt", "]"]]),
    case("true", &[&["true"]]),
    case("false", &[&["false"]]),
    typed("xargs", "a b\nc\n", &[&["xargs", "echo", "got"]]),
    typed("stdin to cat", "typed\n", &[&["cat", "-"]]),
    typed("tee", "kept\n", &[&["tee", "tee-out.txt"], &["cat", "-", "tee-out.txt"]]),
    case("grep", &[&["grep", "apple", "words.txt"]]),
    case("grep -n two files", &[&["grep", "-n", "three", "left.txt", "right.txt"]]),
    case("grep -c", &[&["grep", "-c", "an", "words.txt"]]),
    case("grep -i", &[&["grep", "-i", "banana", "words.txt"]]),
    case("grep -v", &[&["grep", "-v", "a", "words.txt"]]),
    case("grep -o", &[&["grep", "-o", "an", "words.txt"]]),
    case("grep -E", &[&["grep", "-E", "^(apple|date)$", "words.txt"]]),
    case("grep basic \\|", &[&["grep", "cherry\\|date", "words.txt"]]),
    case("grep -w", &[&["grep", "-w", "apple", "words.txt"]]),
    case("grep -x", &[&["grep", "-x", "apple", "words.txt"]]),
    case("grep -A1", &[&["grep", "-n", "-A1", "two", "left.txt"]]),
    case("grep -rn", &[&["grep", "-rn", "alpha", "tree"]]),
    case("grep -rl --include", &[&["grep", "-rl", "--include=*.txt", "beta", "tree"]]),
    case("grep -L", &[&["grep", "-L", "gamma", "tree/a.txt", "tree/sub/b.txt"]]),
    case("grep --color", &[&["grep", "--color=always", "-n", "an", "words.txt"]]),
    case("grep cjk", &[&["grep", "-n", "行", "cjk.txt"]]),
    case("grep none", &[&["grep", "zzz", "words.txt"]]),
    case("grep missing file", &[&["grep", "-s", "x", "no-such-file"]]),
    case("grep in a pipe", &[&["cat", "words.txt"], &["grep", "-c", "e"]]),
    case("sed s///g", &[&["sed", "s/an/AN/g", "words.txt"]]),
    case("sed -n p", &[&["sed", "-n", "2,3p", "left.txt"]]),
    case("sed d", &[&["sed", "/apple/d", "words.txt"]]),
    case("diff", &[&["diff", "left.txt", "right.txt"]]),
    case("diff same", &[&["diff", "left.txt", "left.txt"]]),
    case("diff -q", &[&["diff", "-q", "left.txt", "right.txt"]]),
    case("cmp", &[&["cmp", "left.txt", "right.txt"]]),
    case("cmp -s same", &[&["cmp", "-s", "left.txt", "left.txt"]]),
    case("jq path", &[&["jq", "-r", ".items[].name", "data.json"]]),
    case("jq -c", &[&["jq", "-c", ".items | map(.n) | add", "data.json"]]),
    case("jq --arg", &[&["jq", "-r", "--arg", "who", "ut", "\"\\($who): \\(.ok)\"", "data.json"]]),
    case("jq pretty", &[&["jq", ".items[0]", "data.json"]]),
    case("jq in a pipe", &[&["cat", "data.json"], &["jq", ".items | length"]]),
    case("jq -e false", &[&["jq", "-e", ".missing", "data.json"]]),
    case("ls -1", &[&["ls", "-1", "tree"]]),
    case("ls -1 two levels", &[&["ls", "-1", "tree", "tree/sub"]]),
    native("find -name", &[&["find", "tree", "-name", "*.txt"], &["sort"]]),
    native("find -type d", &[&["find", "tree", "-type", "d"], &["sort"]]),
    case("which", &[&["which", "no-such-command-anywhere"]]),
    case("unknown command", &[&["no-such-command"]]),
];

/// `ut` as cargo built it for this test.
fn ut() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_ut"))
}

/// A directory of one test's own (tests run side by side) with the fixed files in it.
fn fixture(test: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("ut-commands-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (name, text) in FILES {
        let file = root.join(name);
        let parent = file.parent().expect("a file in the fixture has a directory");
        std::fs::create_dir_all(parent).expect("make a fixture directory");
        std::fs::write(file, text).expect("write a fixture file");
    }
    root
}

/// Runs a pipeline in `dir`, `input` typed to its first command; what the
/// last command printed and how it ended.
fn run(input: &str, pipeline: &[&[&str]], dir: &Path) -> (Vec<u8>, i32) {
    let mut carried = input.as_bytes().to_vec();
    let mut code = 0;
    for step in pipeline {
        let mut child = Command::new(ut())
            .args(*step)
            .current_dir(dir)
            // Messages in English, colors and sizes not taken from where the test runs.
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .env_remove("NO_COLOR")
            .env_remove("GREP_COLORS")
            // No shell is between this test and `ut`: every word is meant as it is, as from a program.
            .env("UT_NO_GLOB", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start ut");
        // Written from a thread of its own: a command that prints much while it reads would fill a pipe otherwise.
        let mut stdin = child.stdin.take().expect("the command was started with its input piped");
        let input = std::mem::take(&mut carried);
        let writer = std::thread::spawn(move || {
            // A command that does not read its input (seq) has closed it: no error.
            let _ = stdin.write_all(&input);
        });
        let output = child.wait_with_output().expect("wait for ut");
        writer.join().expect("the thread writing the input does not panic");
        carried = output.stdout;
        code = output.status.code().unwrap_or(-1);
    }
    (carried, code)
}

/// What every case prints, as the text of the record.
fn record(dir: &Path) -> (String, usize) {
    let mut text = String::new();
    let mut native_paths = 0;
    for case in CASES {
        let (printed, code) = run(case.input, case.pipeline, dir);
        if case.paths == Paths::Native && printed.contains(&b'\\') {
            native_paths += 1;
        }
        writeln!(text, "=== {} (exit {code})", case.name).expect("writing to a String cannot fail");
        // One line of the record for each line printed; bytes outside ASCII as \xNN.
        for line in printed.split_inclusive(|byte| *byte == b'\n') {
            writeln!(text, "{}", line.escape_ascii()).expect("writing to a String cannot fail");
        }
    }
    (text, native_paths)
}

#[test]
fn every_command_prints_what_the_record_says() {
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/commands.txt");
    let dir = fixture("record");
    let (now, native_paths) = record(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
    println!("{} cases; {native_paths} printed paths with this system's own separator", CASES.len());
    assert_eq!(native_paths, 0, "a command printed a path with `\\`");

    if std::env::var_os("UT_BLESS").is_some() {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::write(&golden, &now).unwrap();
        println!("wrote {}", golden.display());
        return;
    }
    let recorded =
        std::fs::read_to_string(&golden).expect("the record; write it with UT_BLESS=1").replace("\r\n", "\n");
    if recorded != now {
        let differing: Vec<&str> = recorded
            .split("=== ")
            .zip(now.split("=== "))
            .filter(|(was, is)| was != is)
            .map(|(_, is)| is.lines().next().unwrap_or(""))
            .collect();
        panic!("{} of {} cases differ from the record: {differing:?}", differing.len(), CASES.len());
    }
}

/// What one command prints in `dir`, as text.
fn printed(dir: &Path, step: &[&str]) -> String {
    let (bytes, code) = run("", &[step], dir);
    assert_eq!(code, 0, "{step:?}");
    String::from_utf8(bytes).expect("the fixture's paths are text")
}

#[test]
fn paths_are_printed_with_slashes_on_every_system() {
    let dir = fixture("slashes");
    let here = printed(&dir, &["pwd"]);
    let real = printed(&dir, &["realpath", "tree/sub/b.txt"]);
    let parent = printed(&dir, &["dirname", real.trim()]);
    let found = printed(&dir, &["find", "tree", "-name", "c.log"]);
    let used = printed(&dir, &["du", "-a", "tree/sub"]);
    std::fs::remove_dir_all(&dir).expect("remove the fixture");
    for (command, text) in [("pwd", &here), ("realpath", &real), ("dirname", &parent), ("find", &found), ("du", &used)]
    {
        assert!(!text.contains('\\'), "{command} printed a backslash: {text}");
        assert!(text.contains('/'), "{command} printed no path: {text}");
    }
    let name = dir.file_name().expect("the fixture has a name").to_string_lossy().into_owned();
    assert!(here.trim_end().ends_with(&name), "pwd is the fixture: {here}");
    assert!(real.trim_end().ends_with("/tree/sub/b.txt"), "{real}");
    assert!(parent.trim_end().ends_with("/tree/sub"), "{parent}");
    assert_eq!(found, "tree/sub/deep/c.log\n");
    assert!(used.contains("tree/sub/deep/c.log") && used.contains("tree/sub/b.txt"), "{used}");
}

/// What `ut` prints in `dir` for a command line given to Windows as it is written here.
#[cfg(windows)]
fn command_line(dir: &Path, line: &str) -> String {
    use std::os::windows::process::CommandExt as _;
    let output = Command::new(ut())
        .raw_arg(line)
        .current_dir(dir)
        .env("LC_ALL", "C")
        .env_remove("UT_NO_GLOB")
        .output()
        .expect("start ut");
    String::from_utf8(output.stdout).expect("the fixture's names are text")
}

#[cfg(windows)]
#[test]
fn on_windows_wildcards_become_names_as_in_a_shell() {
    let dir = fixture("wildcards");
    let all = command_line(&dir, "echo *.txt");
    let quoted = command_line(&dir, "echo \"*.txt\" '*.txt' tree/*/*.txt tree\\*.md no-such-*");
    let counted = command_line(&dir, "wc -l left.txt r*.txt");
    let found = command_line(&dir, "find tree -name \"*.txt\"");
    let again = command_line(&dir, "realpath --relative-to=. tree/*.md");
    // A command started by a command gets its words as meant: `*.txt` reaches echo whole.
    // `ut` by its path: an `echo` found in PATH could be another program's.
    let program = ut().display();
    let started = command_line(&dir, &format!("find tree -name a.txt -exec \"{program}\" echo \"*.txt\" {{}} ;"));
    let piped = command_line(&dir, &format!("xargs -a left.txt -n 9 \"{program}\" echo \"[ot]*\""));
    let marked = Command::new(ut())
        .args(["echo", "\u{e000}*.txt", "\u{e000}"])
        .current_dir(&dir)
        .env("UT_NO_GLOB", "1")
        .output()
        .expect("start ut");
    std::fs::remove_dir_all(&dir).expect("remove the fixture");
    assert_eq!(all, "cjk.txt left.txt numbers.txt right.txt tabs.txt words.txt\n");
    assert_eq!(quoted, "*.txt '*.txt' tree/sub/b.txt tree\\notes.md no-such-*\n");
    assert_eq!(counted, " 4 left.txt\n 5 right.txt\n 9 total\n");
    assert_eq!(found, "tree/a.txt\ntree/sub/b.txt\n", "in quotes the pattern reaches find");
    assert_eq!(again, "tree/notes.md\n", "a command that runs twice replaces wildcards once");
    assert_eq!(started, "*.txt tree\\a.txt\n", "find -exec (the path is find's own, with Windows' separator)");
    assert_eq!(piped, "[ot]* one two three four\n", "xargs");
    assert_eq!(String::from_utf8_lossy(&marked.stdout), "*.txt \n", "with UT_NO_GLOB the mark is still taken off");
}

#[test]
fn the_same_run_twice_prints_the_same() {
    let dir = fixture("twice");
    let (first, _) = record(&dir);
    let (second, _) = record(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(first, second, "a command's output changed between two runs on the same files");
}
