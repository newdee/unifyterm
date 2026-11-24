//! The fix that needs no model: a command name that is not a program but
//! is one or two keystrokes from one (`gti` for `git`).

use unifyterm_programs::{bare, distance, within_reach};

use crate::Fix;

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
    let allowed = within_reach(&typed);
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
