//! Whether a newer UnifyTerm is out (`updates.check`).
//!
//! At most once a day the window asks GitHub for the latest release, in the
//! background; what it learned is kept in the data directory
//! (`update.json`), so a window opened again the same day asks nothing. A
//! newer version is named in the row of tabs, with the way to update that
//! fits how this copy was installed. A click opens the release page, and
//! that version is not named again. When GitHub cannot be reached nothing is
//! said: the next window asks again.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// GitHub's answer for the latest release (not a pre-release).
const LATEST: &str = "https://api.github.com/repos/newdee/unifyterm/releases/latest";
/// The page a click opens: the latest release, its notes and its files.
pub(crate) const RELEASE_PAGE: &str = "https://github.com/newdee/unifyterm/releases/latest";
/// How long what was learned is used before GitHub is asked again.
const ASK_EVERY: Duration = Duration::from_hours(24);
/// How long an answer is waited for.
const WAIT: Duration = Duration::from_secs(10);
/// The file in the data directory.
const STATE_FILE: &str = "update.json";

/// What was learned, kept between windows.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct State {
    /// When GitHub last answered, in seconds since 1970.
    checked: u64,
    /// The latest version it named (`0.2.3`).
    latest: Option<String>,
    /// The version whose notice was clicked: not named again.
    dismissed: Option<String>,
}

/// A newer version, and how this copy is updated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Newer {
    pub(crate) version: String,
    pub(crate) how: &'static str,
}

impl Newer {
    /// The words in the row of tabs.
    pub(crate) fn text(&self) -> String {
        format!("UnifyTerm {} is out: {}", self.version, self.how)
    }
}

/// `0.2.3` or `v0.2.3` as numbers; `None` for anything else (a pre-release's `-rc1` too).
fn version_of(text: &str) -> Option<[u64; 3]> {
    let mut parts = text.strip_prefix('v').unwrap_or(text).split('.').map(|part| part.parse::<u64>().ok());
    let version = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some(version)
}

/// Whether `latest` is a later version than `current`.
fn is_newer(latest: &str, current: &str) -> bool {
    matches!((version_of(latest), version_of(current)), (Some(latest), Some(current)) if latest > current)
}

fn state_file(data: &Path) -> PathBuf {
    data.join(STATE_FILE)
}

/// What was kept; nothing for a file that is absent or that cannot be read (it is then written anew).
fn read_state(data: &Path) -> State {
    std::fs::read(state_file(data)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

fn write_state(data: &Path, state: &State) -> Result<()> {
    std::fs::create_dir_all(data).with_context(|| format!("make {}", data.display()))?;
    let file = state_file(data);
    std::fs::write(&file, serde_json::to_vec_pretty(state)?).with_context(|| format!("write {}", file.display()))
}

fn seconds(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

/// The newer version to name, if there is one. `ask` asks GitHub for the
/// latest version; it is called only when the kept answer is a day old or
/// there is none. `current` is this copy's version, `program` its file.
pub(crate) fn check(
    data: &Path,
    now: SystemTime,
    current: &str,
    program: &Path,
    ask: impl FnOnce() -> Result<String>,
) -> Result<Option<Newer>> {
    let mut state = read_state(data);
    let fresh = Duration::from_secs(seconds(now).saturating_sub(state.checked)) < ASK_EVERY;
    if !(fresh && state.latest.is_some()) {
        let latest = ask()?;
        state.checked = seconds(now);
        state.latest = Some(latest.strip_prefix('v').unwrap_or(&latest).to_owned());
        write_state(data, &state)?;
    }
    Ok(state
        .latest
        .filter(|latest| is_newer(latest, current) && state.dismissed.as_ref() != Some(latest))
        .map(|version| Newer { version, how: how_to_update(program) }))
}

/// The notice for `version` was clicked: it is not shown again.
pub(crate) fn dismiss(data: &Path, version: &str) -> Result<()> {
    let mut state = read_state(data);
    state.dismissed = Some(version.to_owned());
    write_state(data, &state)
}

/// GitHub's latest release's tag (`v0.2.3`). Nothing about the user is sent:
/// the program's name and version are the only words it is told.
pub(crate) fn ask_github() -> Result<String> {
    let config = ureq::Agent::config_builder().timeout_global(Some(WAIT)).build();
    let agent: ureq::Agent = config.into();
    let answer: serde_json::Value = agent
        .get(LATEST)
        .header("User-Agent", concat!("unifyterm/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .context("ask GitHub for the latest release")?
        .body_mut()
        .read_json()
        .context("read GitHub's answer")?;
    answer["tag_name"].as_str().map(str::to_owned).context("GitHub's answer names no tag")
}

/// How this copy is updated, by where its file is.
fn how_to_update(program: &Path) -> &'static str {
    let path = program.to_string_lossy().replace('\\', "/").to_lowercase();
    if path.contains("/scoop/apps/") {
        "scoop update unifyterm"
    } else if ["/cellar/", "/homebrew/", "/linuxbrew/"].iter().any(|part| path.contains(part)) {
        "brew upgrade unifyterm"
    } else if path.contains("/appdata/local/programs/unifyterm/")
        || path.contains("/.local/bin/")
        || path.contains("/applications/unifyterm.app/")
    {
        "run the install line again"
    } else {
        "download it from the release page"
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn versions_are_compared_as_numbers() {
        assert!(is_newer("0.2.10", "0.2.9"), "10 is more than 9");
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("0.2.2", "0.2.2"));
        assert!(!is_newer("0.2.1", "0.2.2"));
        assert!(!is_newer("0.3.0-rc1", "0.2.2"), "a pre-release is not named");
        assert!(!is_newer("nonsense", "0.2.2"));
        assert_eq!(version_of("1.2"), None);
        assert_eq!(version_of("1.2.3.4"), None);
    }

    #[test]
    fn each_way_of_installing_has_its_way_of_updating() {
        let how = |path: &str| how_to_update(Path::new(path));
        assert_eq!(how(r"C:\Users\u\scoop\apps\unifyterm\current\unifyterm.exe"), "scoop update unifyterm");
        assert_eq!(how("/opt/homebrew/Cellar/unifyterm/0.2.2/bin/unifyterm"), "brew upgrade unifyterm");
        assert_eq!(how("/home/linuxbrew/.linuxbrew/bin/unifyterm"), "brew upgrade unifyterm");
        assert_eq!(how(r"C:\Users\u\AppData\Local\Programs\unifyterm\unifyterm.exe"), "run the install line again");
        assert_eq!(how("/home/u/.local/bin/unifyterm"), "run the install line again");
        assert_eq!(how("/Users/u/Applications/UnifyTerm.app/Contents/MacOS/unifyterm"), "run the install line again");
        assert_eq!(how(r"C:\Program Files\unifyterm\unifyterm.exe"), "download it from the release page");
        assert_eq!(how("/usr/bin/unifyterm"), "download it from the release page");
    }

    #[test]
    fn github_is_asked_once_a_day_and_a_clicked_version_is_not_named_again() {
        let data = tempfile::tempdir().unwrap();
        let data = data.path();
        let program = Path::new("/usr/bin/unifyterm");
        let start = SystemTime::UNIX_EPOCH + Duration::from_hours(500_000);
        let asked = Cell::new(0);
        let ask = |answer: &'static str| {
            let asked = &asked;
            move || {
                asked.set(asked.get() + 1);
                Ok(answer.to_owned())
            }
        };
        let newer = check(data, start, "0.2.2", program, ask("v0.2.3")).unwrap();
        assert_eq!(
            newer.as_ref().map(Newer::text).as_deref(),
            Some("UnifyTerm 0.2.3 is out: download it from the release page")
        );
        assert_eq!(asked.get(), 1);

        let later = start + Duration::from_hours(1);
        assert!(check(data, later, "0.2.2", program, ask("v9.9.9")).unwrap().is_some());
        assert_eq!(asked.get(), 1, "within a day the kept answer is used");
        assert!(check(data, later, "0.2.3", program, ask("v9.9.9")).unwrap().is_none(), "updated: nothing to name");

        dismiss(data, "0.2.3").unwrap();
        assert!(check(data, later, "0.2.2", program, ask("v9.9.9")).unwrap().is_none(), "clicked: not named again");

        let next_day = start + ASK_EVERY;
        assert_eq!(check(data, next_day, "0.2.2", program, ask("v0.2.4")).unwrap().unwrap().version, "0.2.4");
        assert_eq!(asked.get(), 2, "a day later GitHub is asked again");
    }

    #[test]
    fn no_answer_says_nothing_and_a_bad_file_is_written_anew() {
        let data = tempfile::tempdir().unwrap();
        let data = data.path();
        let program = Path::new("/usr/bin/unifyterm");
        let now = SystemTime::UNIX_EPOCH + Duration::from_hours(500_000);
        assert!(check(data, now, "0.2.2", program, || anyhow::bail!("offline")).is_err());
        assert!(!state_file(data).exists(), "a failed ask keeps nothing: the next window asks again");
        std::fs::write(state_file(data), "not json").unwrap();
        assert!(check(data, now, "0.2.2", program, || Ok("v0.2.3".into())).unwrap().is_some());
        assert_eq!(read_state(data).latest.as_deref(), Some("0.2.3"));
    }

    /// Asks the real GitHub: run by hand (`cargo test -p unifyterm -- --ignored github_names`).
    #[test]
    #[ignore = "asks GitHub over the network"]
    fn github_names_the_latest_release() {
        let tag = ask_github().unwrap();
        assert!(version_of(&tag).is_some(), "{tag}");
    }
}
