//! A history file made on one system and read on another: `tools/round.ps1`
//! runs this on Windows, then `ut history recent --long` on Windows and on
//! Linux against the file, and compares what they print.

use std::time::Duration;

/// Where the file goes (the round sets it).
const SHARED: &str = "UNI_HISTORY_SHARED";
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[test]
#[ignore = "tools/round.ps1 runs it: it writes the file UNI_HISTORY_SHARED names"]
fn write_a_history_for_another_system() {
    let path = std::path::PathBuf::from(std::env::var_os(SHARED).expect("UNI_HISTORY_SHARED names the file"));
    let _ = std::fs::remove_file(&path);
    let mut history = uni_history::History::open(&path).expect("open the file");
    let now = uni_history::now_ms();
    // Days ago, so that "how long ago" reads the same on both systems a minute apart.
    let runs = [
        ("cargo build --release", "C:/Users/me/work", Some(0), 5),
        ("git status", "C:/Users/me/work", Some(0), 4),
        ("ut grep -n TODO src", "C:/Users/me/other dir", Some(1), 3),
        ("echo 中文 and ünïcödé", "C:/Users/me", Some(0), 2),
    ];
    for (text, cwd, exit, days) in runs {
        let run = uni_history::Run {
            text: text.into(),
            at_ms: now - days * DAY_MS,
            cwd: cwd.into(),
            exit,
            duration_ms: Some(i64::try_from(Duration::from_millis(1500).as_millis()).unwrap()),
            shell: "pwsh".into(),
        };
        history.record(&run).expect("record");
    }
    drop(history);
    assert!(path.is_file());
}
