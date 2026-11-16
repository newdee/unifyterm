//! ut: uni-term's commands. One program that is every command: started
//! under a command's name (`cat`, a link to this program) it is that
//! command; started as `ut` it takes the command as its first argument
//! (`ut cat file`).
//!
//! The commands are other people's, linked in: uutils' coreutils, findutils,
//! sed and diffutils, jaq for `jq`, ripgrep's searcher under `grep`.
//! This file only chooses which one runs.

// The linker says it made an import library: some linked crate exports a
// symbol. That is a note, not a problem with this program.
#![allow(linker_messages)]

mod diff;
mod grep;
mod jq;
mod links;
mod which;

use std::ffi::OsString;
use std::io::Write as _;
use std::path::Path;

/// A command: its arguments (the first is its own name) to its exit code.
type Main = fn(Vec<OsString>) -> i32;

/// The table of uutils commands: each name with the crate that is it.
macro_rules! uutils {
    ($($name:literal => $krate:ident),* $(,)?) => {
        &[$(($name, (|args: Vec<OsString>| { localize($name); $krate::uumain(args.into_iter()) }) as Main)),*]
    };
}

/// uutils coreutils that build on Windows, macOS and Linux.
#[rustfmt::skip]
const COREUTILS: &[(&str, Main)] = uutils![
    "[" => uu_test,
    "arch" => uu_arch,
    "b2sum" => uu_b2sum,
    "base32" => uu_base32,
    "base64" => uu_base64,
    "basename" => uu_basename,
    "basenc" => uu_basenc,
    "cat" => uu_cat,
    "cksum" => uu_cksum,
    "comm" => uu_comm,
    "cp" => uu_cp,
    "csplit" => uu_csplit,
    "cut" => uu_cut,
    "date" => uu_date,
    "dd" => uu_dd,
    "df" => uu_df,
    "dir" => uu_dir,
    "dircolors" => uu_dircolors,
    "dirname" => uu_dirname,
    "du" => uu_du,
    "echo" => uu_echo,
    "env" => uu_env,
    "expand" => uu_expand,
    "expr" => uu_expr,
    "factor" => uu_factor,
    "false" => uu_false,
    "fmt" => uu_fmt,
    "fold" => uu_fold,
    "head" => uu_head,
    "hostid" => uu_hostid,
    "hostname" => uu_hostname,
    "join" => uu_join,
    "kill" => uu_kill,
    "link" => uu_link,
    "ln" => uu_ln,
    "ls" => uu_ls,
    "md5sum" => uu_md5sum,
    "mkdir" => uu_mkdir,
    "mktemp" => uu_mktemp,
    "more" => uu_more,
    "mv" => uu_mv,
    "nice" => uu_nice,
    "nl" => uu_nl,
    "nohup" => uu_nohup,
    "nproc" => uu_nproc,
    "numfmt" => uu_numfmt,
    "od" => uu_od,
    "paste" => uu_paste,
    "pathchk" => uu_pathchk,
    "pr" => uu_pr,
    "printenv" => uu_printenv,
    "printf" => uu_printf,
    "ptx" => uu_ptx,
    "pwd" => uu_pwd,
    "readlink" => uu_readlink,
    "realpath" => uu_realpath,
    "rm" => uu_rm,
    "rmdir" => uu_rmdir,
    "seq" => uu_seq,
    "sha1sum" => uu_sha1sum,
    "sha224sum" => uu_sha224sum,
    "sha256sum" => uu_sha256sum,
    "sha384sum" => uu_sha384sum,
    "sha512sum" => uu_sha512sum,
    "shred" => uu_shred,
    "shuf" => uu_shuf,
    "sleep" => uu_sleep,
    "sort" => uu_sort,
    "split" => uu_split,
    "sum" => uu_sum,
    "sync" => uu_sync,
    "tac" => uu_tac,
    "tail" => uu_tail,
    "tee" => uu_tee,
    "test" => uu_test,
    "timeout" => uu_timeout,
    "touch" => uu_touch,
    "tr" => uu_tr,
    "true" => uu_true,
    "truncate" => uu_truncate,
    "tsort" => uu_tsort,
    "tty" => uu_tty,
    "uname" => uu_uname,
    "unexpand" => uu_unexpand,
    "uniq" => uu_uniq,
    "unlink" => uu_unlink,
    "uptime" => uu_uptime,
    "vdir" => uu_vdir,
    "wc" => uu_wc,
    "whoami" => uu_whoami,
    "yes" => uu_yes,
];

/// uutils coreutils that need Unix: owners, modes, devices, logins.
#[cfg(unix)]
#[rustfmt::skip]
const UNIX_ONLY: &[(&str, Main)] = uutils![
    "chgrp" => uu_chgrp,
    "chmod" => uu_chmod,
    "chown" => uu_chown,
    "chroot" => uu_chroot,
    "groups" => uu_groups,
    "id" => uu_id,
    "install" => uu_install,
    "logname" => uu_logname,
    "mkfifo" => uu_mkfifo,
    "mknod" => uu_mknod,
    "pinky" => uu_pinky,
    "stat" => uu_stat,
    "stty" => uu_stty,
    "users" => uu_users,
    "who" => uu_who,
];
#[cfg(not(unix))]
const UNIX_ONLY: &[(&str, Main)] = &[];

/// Commands from other projects, and the two written here.
const OTHERS: &[(&str, Main)] = &[
    ("cmp", diff::cmp),
    ("diff", diff::diff),
    ("find", find),
    ("grep", grep::main),
    ("jq", jq::main),
    ("sed", sed),
    ("which", which::main),
    ("xargs", xargs),
];

/// Every command, in the order `--list` prints them.
fn commands() -> Vec<(&'static str, Main)> {
    let mut all: Vec<_> = COREUTILS.iter().chain(UNIX_ONLY).chain(OTHERS).copied().collect();
    all.sort_by_key(|(name, _)| *name);
    all
}

fn lookup(name: &str) -> Option<Main> {
    COREUTILS.iter().chain(UNIX_ONLY).chain(OTHERS).find(|(known, _)| *known == name).map(|(_, main)| *main)
}

/// Loads a uutils command's messages. Two names share another's: `[` is
/// `test`, `dir` and `vdir` are `ls`.
fn localize(name: &str) {
    let messages = match name {
        "[" => "test",
        "dir" | "vdir" => "ls",
        other => other,
    };
    if let Err(error) = uucore::locale::setup_localization(messages) {
        let _ = writeln!(std::io::stderr(), "{name}: could not load its messages: {error}");
        std::process::exit(MESSAGES_FAILED);
    }
}

/// The exit code when a command's messages cannot be loaded (uutils' own).
const MESSAGES_FAILED: i32 = 99;
/// The exit code for a mistake in how `ut` itself was called.
const USAGE_ERROR: i32 = 2;

/// findutils wants its arguments as text.
fn text_arguments(name: &str, args: Vec<OsString>) -> Result<Vec<String>, i32> {
    args.into_iter().map(OsString::into_string).collect::<Result<_, _>>().map_err(|invalid| {
        let _ = writeln!(std::io::stderr(), "{name}: invalid (non-UTF-8) argument: {}", invalid.to_string_lossy());
        1
    })
}

fn find(args: Vec<OsString>) -> i32 {
    let args = match text_arguments("find", args) {
        Ok(args) => args,
        Err(code) => return code,
    };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    findutils::find::find_main(&args, &findutils::find::StandardDependencies::new())
}

fn xargs(args: Vec<OsString>) -> i32 {
    let args = match text_arguments("xargs", args) {
        Ok(args) => args,
        Err(code) => return code,
    };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    findutils::xargs::xargs_main(&args)
}

fn sed(args: Vec<OsString>) -> i32 {
    sed::sed::uumain(args.into_iter())
}

/// The name a program was started under: its file's name, without `.exe`
/// on Windows, where names also do not differ by case.
fn invoked_as(program: &OsString) -> String {
    let file = Path::new(program).file_name().unwrap_or(program.as_os_str()).to_string_lossy().into_owned();
    if cfg!(windows) {
        let lower = file.to_lowercase();
        lower.strip_suffix(".exe").unwrap_or(&lower).to_owned()
    } else {
        file
    }
}

const USAGE: &str = "usage: ut COMMAND [ARGUMENTS...]
       ut --list                         print every command's name
       ut --links DIRECTORY [--skip A,B] make DIRECTORY hold one link a command
       ut --version

Started under a command's name (a link made by --links), ut is that command.";

/// Runs `ut` called by its own name. `args` starts after the program's name.
fn ut(args: &[OsString]) -> i32 {
    let mut out = std::io::stdout().lock();
    let first = args.first().map(|arg| arg.to_string_lossy().into_owned());
    match first.as_deref() {
        Some("--list") => {
            for (name, _) in commands() {
                // A reader that stopped reading (`ut --list | head`) is no error.
                if writeln!(out, "{name}").is_err() {
                    break;
                }
            }
            0
        }
        Some("--version" | "-V") => {
            let _ = writeln!(out, "ut {} ({} commands)", env!("CARGO_PKG_VERSION"), commands().len());
            0
        }
        Some("--help" | "-h") => {
            let _ = writeln!(out, "{USAGE}");
            0
        }
        Some("--links") => links::main(&args[1..], &commands().into_iter().map(|(name, _)| name).collect::<Vec<_>>()),
        Some(other) => {
            let _ = writeln!(std::io::stderr(), "ut: unknown command '{other}'. 'ut --list' prints them.");
            USAGE_ERROR
        }
        None => {
            let _ = writeln!(std::io::stderr(), "{USAGE}");
            USAGE_ERROR
        }
    }
}

fn main() {
    // A reader that closes its end (`| head`) ends the command quietly.
    uucore::panic::mute_sigpipe_panic();
    let mut args: Vec<OsString> = std::env::args_os().collect();
    let name = args.first().map(invoked_as).unwrap_or_default();
    let code = if let Some(command) = lookup(&name) {
        // A command sees its own name, not its file's path: `[` asks for a
        // closing `]` only when it is called exactly `[`.
        args[0] = OsString::from(&name);
        command(args)
    } else if let Some(command) = args.get(1).and_then(|arg| arg.to_str()).and_then(lookup) {
        // `ut cat file`: the command's name is the second word, and its
        // messages say `ut cat`.
        uucore::set_utility_is_second_arg();
        args.remove(0);
        command(args)
    } else {
        ut(&args[args.len().min(1)..])
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_one_name() {
        let names: Vec<_> = commands().into_iter().map(|(name, _)| name).collect();
        let mut unique = names.clone();
        unique.dedup();
        assert_eq!(names, unique, "sorted, so a name twice is two neighbors");
        let expected = if cfg!(unix) { 114 } else { 99 };
        assert_eq!(names.len(), expected);
        for name in ["cat", "ls", "[", "grep", "jq", "find", "sed", "diff", "which", "xargs", "sha256sum"] {
            assert!(lookup(name).is_some(), "{name}");
        }
        assert!(lookup("no-such-command").is_none());
    }

    #[test]
    fn a_program_is_known_by_its_files_name() {
        assert_eq!(invoked_as(&"cat".into()), "cat");
        assert_eq!(invoked_as(&"/opt/uni-term/bin/sha256sum".into()), "sha256sum");
        if cfg!(windows) {
            assert_eq!(invoked_as(&r"C:\Program Files\uni-term\bin\CAT.EXE".into()), "cat");
            assert_eq!(invoked_as(&"ut.exe".into()), "ut");
        }
    }
}