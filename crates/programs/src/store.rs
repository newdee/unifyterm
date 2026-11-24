//! The folder the descriptions are kept in, one file a program, and the
//! descriptions read from it, read again when the folder changes.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crate::{MOST_BYTES, Program, read};

/// The folder's name in UnifyTerm's data directory.
const FOLDER: &str = "programs";
/// How long the folder is not looked at again after a look.
const BETWEEN_LOOKS: Duration = Duration::from_secs(2);

/// The folder for the descriptions, in the data directory `data`.
#[must_use]
pub fn directory_in(data: &Path) -> PathBuf {
    data.join(FOLDER)
}

/// The coarsest clock a file system keeps a file's time with (FAT: two seconds).
const COARSEST_CLOCK: Duration = Duration::from_secs(2);

/// Whether a file written at `modified` may be written once more without its time changing:
/// a file system's clock ticks in steps, and two writes in one step leave one time. A file
/// read while its time is this young is read again later, or a change of the same size in
/// the same step would never be seen. (A time in the future, from a clock set back, is not young.)
#[must_use]
pub fn stamp_is_young(modified: SystemTime) -> bool {
    SystemTime::now().duration_since(modified).is_ok_and(|age| age < COARSEST_CLOCK)
}

/// What a file is known by: when one of these changes, the folder is read again.
type Stamp = Vec<(PathBuf, u64, Option<SystemTime>)>;

fn stamp(directory: &Path) -> Stamp {
    let Ok(entries) = std::fs::read_dir(directory) else { return Vec::new() };
    let mut files: Stamp = entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|extension| extension.eq_ignore_ascii_case("json")))
        .filter_map(|entry| {
            let about = entry.metadata().ok()?;
            about.is_file().then(|| (entry.path(), about.len(), about.modified().ok()))
        })
        .collect();
    files.sort();
    files
}

/// Reads every description in `directory`, in the order of the files'
/// names, and says which files were not read and why. Of two files for one
/// program, the first is kept.
#[must_use]
pub fn load(directory: &Path) -> (Vec<Program>, Vec<String>) {
    let mut programs: Vec<Program> = Vec::new();
    let mut refused = Vec::new();
    for (file, size, _) in stamp(directory) {
        let shown = file.display();
        if usize::try_from(size).is_ok_and(|size| size > MOST_BYTES) {
            refused.push(format!("{shown}: {}", crate::Error::TooLarge(usize::try_from(size).unwrap_or(usize::MAX))));
            continue;
        }
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(error) => {
                refused.push(format!("{shown}: {error}"));
                continue;
            }
        };
        match read(&text) {
            Ok(read) if programs.iter().any(|program| program.key == read.program.key) => {
                refused.push(format!("{shown}: another file already describes {}", read.program.name));
            }
            Ok(read) => programs.push(read.program),
            Err(error) => refused.push(format!("{shown}: {error}")),
        }
    }
    (programs, refused)
}

#[derive(Debug, Default)]
struct Kept {
    programs: Arc<[Program]>,
    stamp: Stamp,
    looked: Option<Instant>,
    looking: bool,
}

/// The descriptions in a folder, kept in memory. Asking for them never
/// waits for the disk: the folder is looked at on another thread, at most
/// once in two seconds, and what was read last is given meanwhile.
#[derive(Debug, Clone, Default)]
pub struct Programs {
    directory: Option<PathBuf>,
    kept: Arc<Mutex<Kept>>,
}

impl Programs {
    /// The descriptions in `directory`; reading them starts now.
    #[must_use]
    pub fn in_directory(directory: PathBuf) -> Self {
        let programs = Self { directory: Some(directory), kept: Arc::default() };
        let _ = programs.now();
        programs
    }

    /// These descriptions, and no folder: for tests.
    #[must_use]
    pub fn fixed(programs: Vec<Program>) -> Self {
        Self { directory: None, kept: Arc::new(Mutex::new(Kept { programs: programs.into(), ..Kept::default() })) }
    }

    /// The descriptions as last read. Starts a look at the folder when the last one is old enough.
    #[must_use]
    pub fn now(&self) -> Arc<[Program]> {
        let mut kept = self.kept.lock().expect("lock poisoned");
        let due = kept.looked.is_none_or(|looked| looked.elapsed() >= BETWEEN_LOOKS);
        if let Some(directory) = self.directory.as_ref().filter(|_| due && !kept.looking) {
            kept.looking = true;
            let (directory, shared) = (directory.clone(), Arc::clone(&self.kept));
            let started =
                std::thread::Builder::new().name("command-lists".into()).spawn(move || look(&directory, &shared));
            if let Err(error) = started {
                log::error!("start a thread to read the command lists: {error}");
                kept.looking = false;
            }
        }
        Arc::clone(&kept.programs)
    }

    /// Looks at the folder now and waits for it: for a caller that can wait.
    pub fn refresh(&self) {
        if let Some(directory) = &self.directory {
            look(directory, &self.kept);
        }
    }
}

/// Reads the folder again when its files are not the ones read last.
fn look(directory: &Path, shared: &Mutex<Kept>) {
    let found = stamp(directory);
    let changed = shared.lock().expect("lock poisoned").stamp != found;
    // Reading takes a moment for a large description: not while the lock is held.
    let read = changed.then(|| load(directory));
    let mut kept = shared.lock().expect("lock poisoned");
    if let Some((programs, refused)) = read {
        for line in refused {
            log::warn!("command list not read: {line}");
        }
        log::info!("{} command lists read from {}", programs.len(), directory.display());
        kept.programs = programs.into();
        // A file whose time is young is kept without its time: the next look finds it changed and reads it again.
        kept.stamp = found
            .into_iter()
            .map(|(file, size, modified)| (file, size, modified.filter(|modified| !stamp_is_young(*modified))))
            .collect();
    }
    kept.looked = Some(Instant::now());
    kept.looking = false;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::PANES;

    #[test]
    fn the_folder_is_read_and_what_is_no_description_is_named() {
        let folder = tempfile::tempdir().expect("a folder");
        std::fs::write(folder.path().join("panes.json"), PANES).expect("write");
        std::fs::write(folder.path().join("again.json"), PANES).expect("write");
        std::fs::write(folder.path().join("broken.json"), "{").expect("write");
        std::fs::write(folder.path().join("notes.txt"), "not looked at").expect("write");
        let (programs, refused) = load(folder.path());
        assert_eq!(programs.len(), 1, "two files for one program: one kept");
        assert_eq!(programs[0].key, "panes");
        assert_eq!(refused.len(), 2, "{refused:?}");
        assert!(refused[0].contains("broken.json") && refused[0].contains("not an OpenCLI description"), "{refused:?}");
        assert!(refused[1].contains("panes.json") && refused[1].contains("already describes"), "{refused:?}");
        let (none, quiet) = load(&folder.path().join("not-there"));
        assert!(none.is_empty() && quiet.is_empty(), "no folder: no descriptions, and nothing to say");
    }

    #[test]
    fn a_file_put_in_the_folder_later_is_read_at_the_next_look() {
        let folder = tempfile::tempdir().expect("a folder");
        let programs = Programs::in_directory(folder.path().to_owned());
        programs.refresh();
        assert!(programs.now().is_empty());
        std::fs::write(folder.path().join("panes.json"), PANES).expect("write");
        programs.refresh();
        assert_eq!(programs.now().len(), 1);
        // As long, and written in the same step of the file system's clock as often as not:
        // neither the size nor the time tells it from the file read a moment ago.
        let changed = PANES.replace("\"1.2.0\"", "\"1.3.0\"");
        assert_eq!(changed.len(), PANES.len(), "the same size");
        std::fs::write(folder.path().join("panes.json"), &changed).expect("write");
        programs.refresh();
        assert_eq!(programs.now()[0].version, "1.3.0", "a file that changed is read again");
        std::fs::remove_file(folder.path().join("panes.json")).expect("remove");
        programs.refresh();
        assert!(programs.now().is_empty(), "a file taken away is forgotten");
    }

    #[test]
    fn a_time_is_young_for_two_seconds_and_never_when_it_is_ahead() {
        let now = SystemTime::now();
        assert!(stamp_is_young(now - Duration::from_millis(10)));
        assert!(!stamp_is_young(now - Duration::from_secs(3)));
        assert!(!stamp_is_young(now + Duration::from_secs(3600)), "a clock that was set back");
    }

    #[test]
    fn fixed_descriptions_look_at_no_folder() {
        let read = read(PANES).expect("a description").program;
        let programs = Programs::fixed(vec![read]);
        programs.refresh();
        assert_eq!(programs.now().len(), 1);
        assert!(Programs::default().now().is_empty());
        assert_eq!(directory_in(Path::new("data")), Path::new("data").join("programs"));
    }
}
