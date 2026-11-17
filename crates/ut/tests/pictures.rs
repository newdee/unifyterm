//! `ut view` and `ut md` print the same text on every system: their text
//! modes on a fixed picture and a fixed document, compared byte for byte
//! with `tests/golden/pictures.txt` (`UT_BLESS=1` writes it anew).

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

/// The test picture: 64 × 64, four colored quarters and a white diagonal (as in `src/view.rs`).
fn picture(path: &Path) {
    let image = image::RgbaImage::from_fn(64, 64, |x, y| {
        if x == y || x + 1 == y {
            return image::Rgba([255, 255, 255, 255]);
        }
        match (x < 32, y < 32) {
            (true, true) => image::Rgba([220, 40, 40, 255]),
            (false, true) => image::Rgba([40, 200, 60, 255]),
            (true, false) => image::Rgba([40, 80, 220, 255]),
            (false, false) => image::Rgba([230, 210, 40, 255]),
        }
    });
    image.save(path).expect("write the picture");
}

const DOCUMENT: &str = "# uni-term

One terminal for *Windows*, **macOS** and Linux, with `ut` commands and a history you can search as you type.

## Lists

- built-in commands
- a history
  - with candidates
1. first
2. second

> Quotes are indented with a bar.

| command | 中文 | runs |
|:--------|:----:|-----:|
| cat     | 是   | 1    |
| jq      | 是   | 12   |

```
ut view photo.png
```

- [x] done
- [ ] to do

See [the plan](docs/plan.md) and ![a picture](a.png).

---
";

fn ut(dir: &Path, words: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_ut"))
        .args(words)
        .current_dir(dir)
        .env("UT_NO_GLOB", "1")
        .output()
        .expect("run ut");
    assert!(output.status.success(), "{words:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).expect("text")
}

#[test]
fn text_pictures_and_markdown_are_the_same_everywhere() {
    let dir = tempfile::tempdir().expect("a directory");
    picture(&dir.path().join("picture.png"));
    std::fs::write(dir.path().join("doc.md"), DOCUMENT).expect("write the document");
    let mut record = String::new();
    for (name, words) in [
        ("view --mode ascii --width 40", &["view", "--mode", "ascii", "--width", "40", "picture.png"][..]),
        ("view --mode braille --width 40", &["view", "--mode", "braille", "--width", "40", "picture.png"]),
        ("view --mode half --width 16", &["view", "--mode", "half", "--width", "16", "picture.png"]),
        ("view (a pipe: ascii, 80 columns at most, no wider than the picture)", &["view", "picture.png"]),
        ("md --width 60 (a pipe: no styles)", &["md", "--width", "60", "doc.md"]),
        ("md --width 60 --color=always", &["md", "--width", "60", "--color=always", "doc.md"]),
    ] {
        writeln!(record, "=== {name}").expect("writing to a String");
        for line in ut(dir.path(), words).split_inclusive('\n') {
            writeln!(record, "{}", line.escape_default()).expect("writing to a String");
        }
    }
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/pictures.txt");
    if std::env::var_os("UT_BLESS").is_some() {
        std::fs::write(&golden, &record).expect("write the record");
        return;
    }
    let kept = std::fs::read_to_string(&golden).expect("the record; write it with UT_BLESS=1").replace("\r\n", "\n");
    if kept != record {
        let differing: Vec<&str> = kept
            .split("=== ")
            .zip(record.split("=== "))
            .filter(|(was, is)| was != is)
            .map(|(_, is)| is.lines().next().unwrap_or(""))
            .collect();
        panic!("differs from the record: {differing:?}");
    }
}

#[test]
fn the_same_picture_twice_is_the_same_text() {
    let dir = tempfile::tempdir().expect("a directory");
    picture(&dir.path().join("picture.png"));
    let one = ut(dir.path(), &["view", "--mode", "braille", "picture.png"]);
    let two = ut(dir.path(), &["view", "--mode", "braille", "picture.png"]);
    assert_eq!(one, two);
}

#[test]
fn mistakes_are_named() {
    let dir = tempfile::tempdir().expect("a directory");
    std::fs::write(dir.path().join("not.png"), "text").expect("write");
    let run = |words: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ut")).args(words).current_dir(dir.path()).output().expect("run ut")
    };
    let missing = run(&["view", "none.png"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("none.png"));
    let bad = run(&["view", "not.png"]);
    assert_eq!(bad.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("not a picture"));
    assert_eq!(run(&["view", "--mode", "x", "not.png"]).status.code(), Some(2));
    assert_eq!(run(&["md", "--width", "5"]).status.code(), Some(2));
}
