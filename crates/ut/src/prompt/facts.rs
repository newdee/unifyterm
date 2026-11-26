//! What a prompt is about, gathered once a prompt: where the shell is, how
//! the last command went, the git repository, the Python environment.
//!
//! Most of it the shell's hook says on the command line (`--status`,
//! `--took`, `--width`); the rest is read here. Git's branch is read from the
//! repository's own files, which costs next to nothing; its changes need
//! `git status`, which is given a time limit and left out when it is slow.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The shell the prompt is for: each takes the escape sequences its own way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shell {
    /// PowerShell 7 and Windows PowerShell 5.1: the prompt function's text, as it is.
    Pwsh,
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    /// The shell called `name` (as the hooks say it).
    pub(crate) fn named(name: &str) -> Option<Self> {
        match name {
            "pwsh" | "powershell" => Some(Self::Pwsh),
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            "fish" => Some(Self::Fish),
            _ => None,
        }
    }

    /// Whether the shell has a place of its own at the line's end (zsh's
    /// `RPROMPT`, fish's `fish_right_prompt`): a one-line prompt's right side goes there.
    pub(crate) const fn has_right_prompt(self) -> bool {
        matches!(self, Self::Zsh | Self::Fish)
    }
}

/// Where HEAD is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Head {
    Branch(String),
    /// Not on a branch: the commit's first seven digits.
    Detached(String),
}

/// What `git status` counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Changes {
    pub(crate) staged: u32,
    pub(crate) changed: u32,
    pub(crate) untracked: u32,
    pub(crate) ahead: u32,
    pub(crate) behind: u32,
}

impl Changes {
    /// Whether the working tree differs from HEAD (commits ahead or behind are not changes in it).
    pub(crate) const fn any(self) -> bool {
        self.staged + self.changed + self.untracked > 0
    }
}

/// The git repository the shell is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Repo {
    pub(crate) head: Head,
    /// `None` when `git status` did not answer in time, or there is no git: the branch alone is shown.
    pub(crate) changes: Option<Changes>,
}

/// The time of day the prompt was written: hours, minutes, seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Clock(pub(crate) u32, pub(crate) u32, pub(crate) u32);

/// What the prompt is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Facts {
    pub(crate) shell: Shell,
    pub(crate) cwd: PathBuf,
    pub(crate) home: Option<PathBuf>,
    /// The last command's exit code; `None` when no command ran since the last prompt.
    pub(crate) status: Option<i32>,
    /// How long the last command took.
    pub(crate) took: Option<Duration>,
    pub(crate) repo: Option<Repo>,
    /// The Python environment's name.
    pub(crate) venv: Option<String>,
    /// The terminal's width in cells, when the hook says it.
    pub(crate) width: Option<usize>,
    pub(crate) now: Clock,
}

/// How long `git status` may take before the prompt goes on without the changes.
const GIT_STATUS_LIMIT: Duration = Duration::from_millis(300);
/// How often a running `git status` is looked at, while it is waited for.
const GIT_STATUS_POLL: Duration = Duration::from_millis(5);
/// A commit is named by this many digits when HEAD is not on a branch.
const SHORT_COMMIT: usize = 7;

/// The repository `start` is in: the first folder from it upward with a `.git`.
/// A `.git` file (a worktree, a submodule) names the real one: `gitdir: PATH`.
pub(crate) fn git_dir(start: &Path) -> Option<PathBuf> {
    for folder in start.ancestors() {
        let dot = folder.join(".git");
        if dot.is_dir() {
            return Some(dot);
        }
        if dot.is_file() {
            let text = std::fs::read_to_string(&dot).ok()?;
            let named = text.lines().find_map(|line| line.strip_prefix("gitdir:"))?.trim();
            let path = Path::new(named);
            return Some(if path.is_absolute() { path.to_path_buf() } else { folder.join(path) });
        }
    }
    None
}

/// Where HEAD is, from the repository's `HEAD` file.
pub(crate) fn head(git_dir: &Path) -> Option<Head> {
    let text = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let text = text.trim();
    match text.strip_prefix("ref:") {
        Some(reference) => {
            let reference = reference.trim();
            Some(Head::Branch(reference.strip_prefix("refs/heads/").unwrap_or(reference).to_owned()))
        }
        None if text.len() >= SHORT_COMMIT && text.bytes().all(|byte| byte.is_ascii_hexdigit()) => {
            Some(Head::Detached(text[..SHORT_COMMIT].to_owned()))
        }
        None => None,
    }
}

/// What `git status --porcelain=v2 --branch` printed, counted.
pub(crate) fn changes(porcelain: &str) -> Changes {
    let mut counted = Changes::default();
    for line in porcelain.lines() {
        if let Some(ab) = line.strip_prefix("# branch.ab ") {
            for part in ab.split_whitespace() {
                let number = |digits: &str| digits.parse().unwrap_or(0);
                if let Some(ahead) = part.strip_prefix('+') {
                    counted.ahead = number(ahead);
                } else if let Some(behind) = part.strip_prefix('-') {
                    counted.behind = number(behind);
                }
            }
        } else if let Some(entry) = line.strip_prefix("1 ").or_else(|| line.strip_prefix("2 ")) {
            // XY: what is staged, what is changed in the working tree; `.` is nothing.
            let mut xy = entry.chars();
            if xy.next().is_some_and(|x| x != '.') {
                counted.staged += 1;
            }
            if xy.next().is_some_and(|y| y != '.') {
                counted.changed += 1;
            }
        } else if line.starts_with("u ") {
            counted.changed += 1;
        } else if line.starts_with("? ") {
            counted.untracked += 1;
        }
    }
    counted
}

/// `git status` in `folder`, counted; `None` when git is not there, fails,
/// or takes longer than the limit (it is stopped then).
fn status_of(folder: &Path) -> Option<Changes> {
    let mut child = Command::new("git")
        .args(["--no-optional-locks", "status", "--porcelain=v2", "--branch", "--untracked-files=normal"])
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // What git prints is read while it runs: a status larger than the pipe holds (thousands of
    // files) would otherwise leave git waiting to write, and never ending.
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        stdout.read_to_string(&mut out).map(|_| out)
    });
    let deadline = Instant::now() + GIT_STATUS_LIMIT;
    loop {
        match child.try_wait() {
            Ok(Some(ended)) if ended.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if Instant::now() >= deadline => {
                // Stopped, its pipe closes and the reader ends: nothing is left behind.
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(GIT_STATUS_POLL),
        }
    }
    let out = reader.join().ok()?.ok()?;
    Some(changes(&out))
}

/// The repository `folder` is in, with its changes when git answers in time.
pub(crate) fn repo(folder: &Path) -> Option<Repo> {
    let head = head(&git_dir(folder)?)?;
    Some(Repo { head, changes: status_of(folder) })
}

/// The Python environment in use: the last part of `VIRTUAL_ENV`, else conda's name.
pub(crate) fn venv(virtual_env: Option<&str>, conda: Option<&str>) -> Option<String> {
    let from_path = virtual_env
        .filter(|path| !path.is_empty())
        .and_then(|path| path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next())
        .filter(|name| !name.is_empty());
    from_path.or(conda.filter(|name| !name.is_empty())).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_is_a_branch_or_a_detached_commit() {
        let folder = tempfile::tempdir().expect("a folder");
        let git = folder.path().join(".git");
        std::fs::create_dir_all(&git).expect("a .git");
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").expect("write");
        assert_eq!(head(&git), Some(Head::Branch("main".into())));
        std::fs::write(git.join("HEAD"), "ref: refs/heads/feature/prompt\n").expect("write");
        assert_eq!(head(&git), Some(Head::Branch("feature/prompt".into())), "a branch with a slash");
        std::fs::write(git.join("HEAD"), "3c3e6b9a1f2d4e5b6c7d8e9f0a1b2c3d4e5f6a7b\n").expect("write");
        assert_eq!(head(&git), Some(Head::Detached("3c3e6b9".into())));
        std::fs::write(git.join("HEAD"), "nonsense").expect("write");
        assert_eq!(head(&git), None);
    }

    #[test]
    fn the_repository_is_found_upward_and_through_a_git_file() {
        let folder = tempfile::tempdir().expect("a folder");
        let root = folder.path();
        std::fs::create_dir_all(root.join("repo/.git")).expect("a .git");
        std::fs::create_dir_all(root.join("repo/src/deep")).expect("folders");
        assert_eq!(git_dir(&root.join("repo/src/deep")), Some(root.join("repo/.git")));
        // A worktree: .git is a file that names the real directory.
        std::fs::create_dir_all(root.join("worktree")).expect("a folder");
        std::fs::write(root.join("worktree/.git"), "gitdir: ../repo/.git/worktrees/w\n").expect("write");
        assert_eq!(git_dir(&root.join("worktree")), Some(root.join("worktree").join("../repo/.git/worktrees/w")));
        assert_eq!(git_dir(&root.join("elsewhere")), None, "no repository above");
    }

    #[test]
    fn git_status_is_counted() {
        let porcelain = "# branch.oid 3c3e6b9\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n\
            1 M. N... 100644 100644 100644 a b src/a.rs\n\
            1 .M N... 100644 100644 100644 a b src/b.rs\n\
            1 MM N... 100644 100644 100644 a b src/c.rs\n\
            2 R. N... 100644 100644 100644 a b R100 new.rs\told.rs\n\
            u UU N... 100644 100644 100644 100644 a b c conflict.rs\n\
            ? notes.md\n? todo.txt\n";
        let counted = changes(porcelain);
        assert_eq!(counted, Changes { staged: 3, changed: 3, untracked: 2, ahead: 2, behind: 1 });
        assert!(counted.any());
        let clean = changes("# branch.oid 3c3e6b9\n# branch.head main\n# branch.ab +0 -0\n");
        assert_eq!(clean, Changes::default());
        assert!(!clean.any());
        assert!(!Changes { ahead: 3, ..Changes::default() }.any(), "commits ahead are not changes in the tree");
    }

    #[test]
    fn the_python_environment_is_its_folders_name_or_condas() {
        assert_eq!(venv(Some("/home/me/project/.venv"), None).as_deref(), Some(".venv"));
        assert_eq!(venv(Some("C:\\work\\env\\"), Some("base")).as_deref(), Some("env"), "the virtual one first");
        assert_eq!(venv(None, Some("science")).as_deref(), Some("science"));
        assert_eq!(venv(Some(""), Some("")), None);
        assert_eq!(venv(None, None), None);
    }

    #[test]
    fn the_shells_are_known_by_the_names_the_hooks_say() {
        assert_eq!(Shell::named("pwsh"), Some(Shell::Pwsh));
        assert_eq!(Shell::named("powershell"), Some(Shell::Pwsh));
        assert_eq!(Shell::named("zsh"), Some(Shell::Zsh));
        assert_eq!(Shell::named("cmd"), None);
        assert!(Shell::Zsh.has_right_prompt() && Shell::Fish.has_right_prompt());
        assert!(!Shell::Pwsh.has_right_prompt() && !Shell::Bash.has_right_prompt());
    }
}
