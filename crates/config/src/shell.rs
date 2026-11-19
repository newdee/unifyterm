//! What a shell is given so that UnifyTerm's commands are the ones it finds:
//! the directory of links to `ut` (one a command), and for PowerShell a start
//! script that takes PowerShell's own aliases of the same names out of the
//! way. The terminal gives them to the shells it starts; `ut shell init`
//! gives them to every shell once `ut shell enable` has asked for it.

/// The variable a shell's start sets once UnifyTerm's part has run in it: a
/// shell variable, never exported, so that a shell started from this one
/// runs it again for itself.
pub const LOADED_VARIABLE: &str = "__unifyterm_loaded";

/// PowerShell's aliases that hide a command of UnifyTerm's, each with the
/// cmdlet it stands for when nobody has changed it.
const POWERSHELL_ALIASES: [(&str, &str); 14] = [
    ("cat", "Get-Content"),
    ("cp", "Copy-Item"),
    ("diff", "Compare-Object"),
    ("dir", "Get-ChildItem"),
    ("echo", "Write-Output"),
    ("kill", "Stop-Process"),
    ("ls", "Get-ChildItem"),
    ("mv", "Move-Item"),
    ("pwd", "Get-Location"),
    ("rm", "Remove-Item"),
    ("rmdir", "Remove-Item"),
    ("sleep", "Start-Sleep"),
    ("sort", "Sort-Object"),
    ("tee", "Tee-Object"),
];

/// PowerShell's functions that hide a command of UnifyTerm's, each with a
/// piece of text its own definition has.
const POWERSHELL_FUNCTIONS: [(&str, &str); 2] = [("mkdir", "New-Item"), ("more", "more.com")];

/// The script PowerShell runs at its start, with `@NAME@` where a value goes.
pub const POWERSHELL_START: &str = include_str!("start.ps1");

/// The script PowerShell runs at its start, for these commands.
///
/// PowerShell finds an alias or a function before a program of the same
/// name. For each of `commands` the alias or function is removed, but only
/// while it is still PowerShell's own: one the user defined is theirs to keep.
/// The rest of what the script does is said in the script (`start.ps1`).
#[must_use]
pub fn powershell_start(commands: &[&str]) -> String {
    let pairs = |table: &[(&str, &str)]| -> String {
        let wanted = table.iter().filter(|(name, _)| commands.contains(name));
        wanted.map(|(name, own)| format!("@('{name}','{own}')")).collect::<Vec<_>>().join(", ")
    };
    // Inside the script's single quotes a `'` is written twice.
    let quoted = |text: &str| text.replace('\'', "''");
    let names = commands.iter().map(|name| format!("'{}'", quoted(name))).collect::<Vec<_>>().join(", ");
    POWERSHELL_START
        .replace("@ALIASES@", &pairs(&POWERSHELL_ALIASES))
        .replace("@FUNCTIONS@", &pairs(&POWERSHELL_FUNCTIONS))
        .replace("@NAMES@", &names)
}

/// FNV-1a's starting value and multiplier for 64 bits.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The name of the link directory for a set of skipped commands: `bin` for
/// none, else `bin-` and a number made from the names. Two windows whose
/// settings skip different commands then each have a directory of their own
/// and do not undo each other's links. The number is the same for the same
/// names in any order, on every system and in every version (FNV-1a).
#[must_use]
pub fn directory_name(skipped: &[&str]) -> String {
    if skipped.is_empty() {
        return "bin".into();
    }
    let mut sorted = skipped.to_vec();
    sorted.sort_unstable();
    let bytes = sorted.join(",").into_bytes();
    let hash = bytes.iter().fold(FNV_OFFSET, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME));
    format!("bin-{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_start_script_removes_only_what_hides_a_command_in_use() {
        let script = powershell_start(&["cat", "ls", "jq", "mkdir", "o'neil"]);
        assert!(script.contains("@('cat','Get-Content'), @('ls','Get-ChildItem')"), "{script}");
        assert!(script.contains("@('mkdir','New-Item')"), "{script}");
        for absent in ["@('rm'", "@('sort'", "@('more'", "@('jq'"] {
            assert!(!script.contains(absent), "{absent} is not hidden by PowerShell or not in use:\n{script}");
        }
        assert!(script.contains("$alias.Definition -eq $pair[1]"), "an alias the user changed is kept");
        assert!(script.contains("UTF8Encoding"), "the console is set to UTF-8");
        assert!(script.contains("function global:prompt {") && script.contains(".Replace('\\', '/')"), "{script}");
        assert!(
            script.contains(".Contains('PS $($executionContext.SessionState.Path.CurrentLocation)$(')"),
            "only PowerShell's own prompt is replaced"
        );
        assert!(
            script.contains("[string[]]@('cat', 'ls', 'jq', 'mkdir', 'o''neil')"),
            "the commands in use are named, quoted:\n{script}"
        );
        assert!(
            script.contains("$found.Source.StartsWith($env:UNIFYTERM_COMMANDS"),
            "the directory is read from UNIFYTERM_COMMANDS"
        );
        assert!(script.contains("[char]0xE000"), "the mark is the one `ut` takes off (ut's src/expand.rs)");
        for place in ["@ALIASES@", "@FUNCTIONS@", "@NAMES@"] {
            assert!(!script.contains(place), "{place} was given its value");
        }
        let none = powershell_start(&[]);
        assert!(none.contains("foreach ($pair in @()) {"), "no command in use: nothing to remove\n{none}");
        assert!(none.contains("[string[]]@()"), "{none}");
    }

    #[test]
    fn the_start_script_runs_once_in_each_powershell() {
        let mark = format!("$global:{LOADED_VARIABLE}");
        let first = POWERSHELL_START.find(&format!("if ({mark}) {{ return }}")).expect("it leaves when it has run");
        let set = POWERSHELL_START.find(&format!("{mark} = $true")).expect("and says that it has");
        let work = POWERSHELL_START.find("$env:UNIFYTERM_COMMANDS").expect("its work");
        assert!(first < set && set < work, "the check and the mark come before anything it does");
        assert!(!POWERSHELL_START.contains(&format!("$env:{LOADED_VARIABLE}")), "never an environment variable");
    }

    #[test]
    fn each_set_of_skipped_commands_has_a_directory() {
        assert_eq!(directory_name(&[]), "bin");
        assert_eq!(directory_name(&["cat"]), "bin-f5e307190ce4a327", "FNV-1a of \"cat\", the same everywhere");
        assert_eq!(directory_name(&["ls", "cat"]), directory_name(&["cat", "ls"]), "the order does not matter");
        assert_ne!(directory_name(&["cat"]), directory_name(&["cat", "ls"]));
        assert_ne!(directory_name(&["a", "bc"]), directory_name(&["ab", "c"]), "the names are kept apart");
    }
}
