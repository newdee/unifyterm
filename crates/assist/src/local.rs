//! The fix that needs no model: a command name that is not a program but
//! is one or two keystrokes from one (`gti` for `git`).

use crate::Fix;

/// How many single-character edits turn `a` into `b`: insert, delete,
/// replace, or swap two neighbors (optimal string alignment distance).
#[must_use]
pub fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let width = b.len() + 1;
    let mut table = vec![0usize; (a.len() + 1) * width];
    let at = |i: usize, j: usize| i * width + j;
    for i in 0..=a.len() {
        table[at(i, 0)] = i;
    }
    for j in 0..=b.len() {
        table[at(0, j)] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (table[at(i - 1, j)] + 1).min(table[at(i, j - 1)] + 1).min(table[at(i - 1, j - 1)] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(table[at(i - 2, j - 2)] + 1);
            }
            table[at(i, j)] = best;
        }
    }
    table[at(a.len(), b.len())]
}

/// A program's name as compared: no `.exe` or the like, lower case.
fn bare(name: &str) -> String {
    let lower = name.to_lowercase();
    for extension in [".exe", ".cmd", ".bat", ".com", ".ps1"] {
        if let Some(stem) = lower.strip_suffix(extension) {
            return stem.to_owned();
        }
    }
    lower
}

/// A fix for `command` when its first word is no name in `known` but is
/// close to one: one edit for names of up to four letters, two for longer
/// ones. Of names as close, the first in `known` wins: give the most used
/// first. `None` when the name is known, or nothing is close.
#[must_use]
pub fn local_fix(command: &str, known: &[String]) -> Option<Fix> {
    let trimmed = command.trim_start();
    let name = trimmed.split_whitespace().next()?;
    let rest = &trimmed[name.len()..];
    let typed = bare(name);
    if typed.contains(['/', '\\']) || known.iter().any(|known| bare(known) == typed) {
        return None;
    }
    let allowed = if typed.chars().count() <= 4 { 1 } else { 2 };
    let (best, how_far) = known
        .iter()
        .map(|candidate| (candidate, distance(&typed, &bare(candidate))))
        .filter(|(_, how_far)| *how_far <= allowed)
        .min_by_key(|(_, how_far)| *how_far)?;
    let best = bare(best);
    let edits = if how_far == 1 { "one keystroke" } else { "two keystrokes" };
    Some(Fix { command: format!("{best}{rest}"), reason: format!("'{name}' is no command; '{best}' is {edits} away.") })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(names: &[&str]) -> Vec<String> {
        names.iter().map(|&name| name.to_owned()).collect()
    }

    #[test]
    fn the_distance_counts_swaps_as_one() {
        assert_eq!(distance("gti", "git"), 1);
        assert_eq!(distance("git", "git"), 0);
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("dokcer", "docker"), 1);
        assert_eq!(distance("中文", "中午"), 1);
    }

    #[test]
    fn a_misspelt_name_is_put_right_and_the_rest_kept() {
        let names = known(&["git.exe", "cargo", "docker", "ls", "cd", "kubectl"]);
        let fixed = local_fix("gti status -s", &names).unwrap();
        assert_eq!(fixed.command, "git status -s");
        assert!(fixed.reason.contains("'gti'") && fixed.reason.contains("'git'"));
        assert_eq!(local_fix("dokcer ps -a", &names).unwrap().command, "docker ps -a");
        assert_eq!(
            local_fix("kubctel get pods", &names).unwrap().command,
            "kubectl get pods",
            "two edits for a long name"
        );
        assert_eq!(local_fix("  carg build", &names).unwrap().command, "cargo build");
    }

    #[test]
    fn nothing_is_made_up() {
        let names = known(&["git", "ls", "cd", "Get-ChildItem"]);
        assert_eq!(local_fix("git stauts", &names), None, "a known name: the rest is not this rule's");
        assert_eq!(local_fix("GIT.EXE status", &names), None, "case and .exe aside");
        assert_eq!(local_fix("get-childitem", &names), None);
        assert_eq!(local_fix("xx", &names), None, "two edits from 'ls' and 'cd': too far for a short name");
        assert_eq!(local_fix("./build.sh", &names), None, "a path is the user's");
        assert_eq!(local_fix("", &names), None);
        assert_eq!(local_fix("terraform plan", &names), None);
    }
}
