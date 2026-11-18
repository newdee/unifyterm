// Shared by the crates' build scripts (`include!`d): the commit being built.

/// The commit being built, as `git rev-parse --short=7 HEAD` gives it;
/// `unknown` without git or outside a repository (a source archive).
/// Cargo runs the build script again when HEAD moves.
fn commit() -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let git = root.join(".git");
    let head = git.join("HEAD");
    // Only files that are there: a missing one would run the script on every build.
    let mut watched = vec![head.clone(), git.join("packed-refs")];
    if let Ok(text) = std::fs::read_to_string(&head)
        && let Some(reference) = text.strip_prefix("ref: ")
    {
        watched.push(git.join(reference.trim()));
    }
    for file in watched.iter().filter(|file| file.is_file()) {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    std::process::Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .current_dir(&root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|commit| !commit.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}