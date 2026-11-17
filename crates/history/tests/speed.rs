//! How fast a search is in a large history. Run in release:
//!
//!     cargo test --release -p uni-history --test speed -- --ignored --nocapture
//!
//! The commands and the queries come from a fixed generator, so every run
//! measures the same work.

use std::fmt::Write as _;
use std::time::Instant;

use uni_history::{History, Query, Run};

const COMMANDS: usize = 100_000;
const QUERIES: usize = 1_000;
/// What a single search may take at the 95th percentile, in milliseconds.
const P95_LIMIT_MS: f64 = 5.0;
/// What recording one run may take on average, in milliseconds.
const RECORD_LIMIT_MS: f64 = 2.0;
const NOW: i64 = 1_800_000_000_000;

/// A fixed sequence of numbers (a linear congruential generator).
struct Numbers(u64);

impl Numbers {
    fn next(&mut self, below: usize) -> usize {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(self.0 >> 33).expect("31 bits fit") % below
    }
}

const PROGRAMS: [&str; 16] = [
    "git", "cargo", "npm", "docker", "kubectl", "ls", "cd", "grep", "find", "python", "make", "ssh", "curl", "jq",
    "code", "rg",
];
const WORDS: [&str; 24] = [
    "status",
    "commit",
    "push",
    "pull",
    "test",
    "build",
    "run",
    "install",
    "log",
    "diff",
    "checkout",
    "-m",
    "--all",
    "src",
    "main",
    "release",
    "fix",
    "deploy",
    "config",
    "init",
    "-la",
    "docs",
    "target",
    "README.md",
];

fn command(numbers: &mut Numbers, index: usize) -> String {
    let mut text = PROGRAMS[numbers.next(PROGRAMS.len())].to_owned();
    for _ in 0..=numbers.next(4) {
        text.push(' ');
        text.push_str(WORDS[numbers.next(WORDS.len())]);
    }
    // Every command different, as in a real history of this size.
    write!(text, " {index}").expect("writing to a String cannot fail");
    text
}

/// The value below which `percent` of the sorted values lie (nearest rank).
fn percentile(sorted: &[f64], percent: usize) -> f64 {
    sorted[(sorted.len() - 1) * percent / 100]
}

#[test]
#[ignore = "slow: run in release with --ignored"]
fn a_search_in_100_000_commands_is_fast() {
    let dir = tempfile::tempdir().unwrap();
    let mut history = History::open(&dir.path().join("history.db")).unwrap();
    let mut numbers = Numbers(7);
    let started = Instant::now();
    let mut texts = Vec::with_capacity(COMMANDS);
    for index in 0..COMMANDS {
        let text = command(&mut numbers, index);
        let run = Run {
            text: text.clone(),
            at_ms: NOW - i64::try_from(numbers.next(90 * 24 * 60)).unwrap() * 60_000,
            cwd: format!("C:/work/p{}", numbers.next(20)),
            exit: Some(i32::try_from(numbers.next(5)).unwrap() / 4),
            duration_ms: Some(5),
            shell: "pwsh".into(),
        };
        history.record(&run).unwrap();
        texts.push(text);
    }
    let record_ms = started.elapsed().as_secs_f64() * 1000.0 / f64::from(u32::try_from(COMMANDS).unwrap());

    let mut times = Vec::with_capacity(QUERIES);
    let mut found_total = 0;
    for _ in 0..QUERIES {
        // A beginning of a real command, 1 to 12 characters long: what someone types.
        let source = &texts[numbers.next(texts.len())];
        let length = 1 + numbers.next(12.min(source.len()));
        let typed: String = source.chars().take(length).collect();
        let cwd = format!("C:/work/p{}", numbers.next(20));
        let started = Instant::now();
        let found = history.search(&Query { text: &typed, cwd: &cwd, now_ms: NOW, limit: 8 }).unwrap();
        times.push(started.elapsed().as_secs_f64() * 1000.0);
        found_total += found.len();
    }
    times.sort_by(f64::total_cmp);
    let (p50, p95, p99) = (percentile(&times, 50), percentile(&times, 95), percentile(&times, 99));
    println!(
        "{COMMANDS} commands; record {record_ms:.3} ms each; {QUERIES} searches: p50 {p50:.2} ms, p95 {p95:.2} ms, \
         p99 {p99:.2} ms, max {:.2} ms; {found_total} candidates",
        times[times.len() - 1]
    );
    assert!(found_total >= QUERIES, "every query is a beginning of a stored command: each finds at least one");
    assert!(p95 < P95_LIMIT_MS, "p95 {p95:.2} ms");
    assert!(record_ms < RECORD_LIMIT_MS, "record {record_ms:.3} ms");
}
