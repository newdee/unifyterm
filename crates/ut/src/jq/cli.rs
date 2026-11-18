//! jq's command line. From jaq 3.1.2's `cli.rs` (MIT, © Michael Färber),
//! changed in three places: the arguments come from the caller and not from
//! the process, `--run-tests` is left out, and color needs no Windows call
//! (the terminals UnifyTerm's commands run in take colors as they are).
use core::fmt;
pub(crate) use jaq_all::fmts::Format;
use std::ffi::OsString;
use std::io::{self, IsTerminal};
use std::path::PathBuf;

/// The command's arguments, after its own name.
type Args = std::vec::IntoIter<OsString>;

#[derive(Debug, Default)]
#[expect(clippy::struct_excessive_bools, reason = "as in jaq: one field for each of jq's flags, which are independent")]
pub(crate) struct Cli {
    // Input options
    pub(crate) from: Option<Format>,
    pub(crate) null_input: bool,
    /// When input is read from files,
    /// jaq yields an array for each file, whereas
    /// jq produces only a single array.
    pub(crate) slurp: bool,

    // Output options
    pub(crate) to: Option<Format>,
    pub(crate) compact_output: bool,
    /// This flag enables `--raw-output`.
    pub(crate) join_output: bool,
    pub(crate) in_place: bool,
    pub(crate) sort_keys: bool,
    pub(crate) color_output: bool,
    pub(crate) monochrome_output: bool,
    pub(crate) tab: bool,
    pub(crate) indent: Option<usize>,

    // Compilation options
    pub(crate) from_file: bool,
    /// If this option is given multiple times, all given directories are searched.
    pub(crate) library_path: Vec<PathBuf>,

    // Key-value options
    pub(crate) arg: Vec<(String, String)>,
    pub(crate) argjson: Vec<(String, String)>,
    pub(crate) slurpfile: Vec<(String, OsString)>,
    pub(crate) rawfile: Vec<(String, OsString)>,

    // Positional arguments
    /// If this argument is not given, it is assumed to be `.`, the identity filter.
    pub(crate) filter: Option<Filter>,
    pub(crate) files: Vec<PathBuf>,
    pub(crate) args: Vec<String>,
    /// If there is some last output value `v`,
    /// then the exit status code is
    /// 1 if `v < true` (that is, if `v` is `false` or `null`) and
    /// 0 otherwise.
    /// If there is no output value, then the exit status code is 4.
    ///
    /// If any error occurs, then this option has no effect.
    pub(crate) exit_status: bool,
    pub(crate) version: bool,
    pub(crate) help: bool,
}

#[derive(Debug)]
pub(crate) enum Filter {
    Inline(String),
    FromFile(PathBuf),
}

impl Cli {
    fn positional(&mut self, mode: &Mode, arg: OsString) -> Result<(), Error> {
        if self.filter.is_none() {
            self.filter =
                Some(if self.from_file { Filter::FromFile(arg.into()) } else { Filter::Inline(arg.into_string()?) });
        } else {
            match mode {
                Mode::Files => self.files.push(arg.into()),
                Mode::Args => self.args.push(arg.into_string()?),
            }
        }
        Ok(())
    }

    fn long(&mut self, mode: &mut Mode, arg: &str, args: &mut Args) -> Result<(), Error> {
        let int = |s: OsString| s.into_string().ok()?.parse().ok();
        match arg {
            // handle all arguments after "--"
            "" => args.try_for_each(|arg| self.positional(mode, arg))?,

            "from" => self.from = Some(parse_format("--from", args)?),
            "null-input" => self.short('n', args)?,
            "raw-input" => self.short('R', args)?,
            "raw-input0" => self.from = Some(Format::Raw0),
            "slurp" => self.short('s', args)?,

            "to" => self.to = Some(parse_format("--to", args)?),
            "compact-output" => self.short('c', args)?,
            "raw-output" => self.short('r', args)?,
            "raw-output0" => self.to = Some(Format::Raw0),
            "join-output" => self.short('j', args)?,
            "in-place" => self.short('i', args)?,
            "sort-keys" => self.short('S', args)?,
            "color-output" => self.short('C', args)?,
            "monochrome-output" => self.short('M', args)?,
            "tab" => self.tab = true,
            "indent" => self.indent = Some(args.next().and_then(int).ok_or(Error::Int("--indent"))?),
            "from-file" => self.short('f', args)?,
            "library-path" => self.short('L', args)?,
            "arg" => {
                let (name, value) = parse_key_val("--arg", args)?;
                self.arg.push((name, value.into_string()?));
            }
            "argjson" => {
                let (name, value) = parse_key_val("--argjson", args)?;
                self.argjson.push((name, value.into_string()?));
            }
            "slurpfile" => self.slurpfile.push(parse_key_val("--slurpfile", args)?),
            "rawfile" => self.rawfile.push(parse_key_val("--rawfile", args)?),

            "args" => *mode = Mode::Args,
            "exit-status" => self.short('e', args)?,
            "version" => self.short('V', args)?,
            "help" => self.short('h', args)?,

            arg => Err(Error::Flag(format!("--{arg}")))?,
        }
        Ok(())
    }

    fn short(&mut self, arg: char, args: &mut Args) -> Result<(), Error> {
        match arg {
            'R' => self.from = Some(Format::Raw),
            'n' => self.null_input = true,
            's' => self.slurp = true,

            'r' => self.to = Some(Format::Raw),
            'c' => self.compact_output = true,
            'j' => {
                self.join_output = true;
                self.to.get_or_insert(Format::Raw);
            }
            'i' => self.in_place = true,
            'S' => self.sort_keys = true,
            'C' => self.color_output = true,
            'M' => self.monochrome_output = true,

            'f' => self.from_file = true,
            'L' => self.library_path.push(args.next().ok_or(Error::Path("-L"))?.into()),
            'e' => self.exit_status = true,
            'V' => self.version = true,
            'h' => self.help = true,
            arg => Err(Error::Flag(format!("-{arg}")))?,
        }
        Ok(())
    }

    /// `args`: the command's arguments, its own name first.
    pub(crate) fn parse(args: Vec<OsString>) -> Result<Self, Error> {
        let mut cli = Self::default();
        let mut mode = Mode::Files;
        let mut args = args.into_iter();
        args.next();
        while let Some(arg) = args.next() {
            match arg.to_str() {
                // we've got a valid UTF-8 argument here
                Some(s) => match s.strip_prefix("--") {
                    Some(rest) => cli.long(&mut mode, rest, &mut args)?,
                    None => match s.strip_prefix("-") {
                        Some(rest) => rest.chars().try_for_each(|c| cli.short(c, &mut args))?,
                        None => cli.positional(&mode, arg)?,
                    },
                },
                // we've got invalid UTF-8, so it is no valid flag
                None => cli.positional(&mode, arg)?,
            }
        }
        Ok(cli)
    }

    fn color_if(&self, f: impl Fn() -> bool) -> bool {
        let no_color = || std::env::var("NO_COLOR").is_ok_and(|v| !v.is_empty());
        !self.monochrome_output && (self.color_output || (!no_color() && f()))
    }

    pub(crate) fn color_output(&self) -> bool {
        self.color_if(|| io::stdout().is_terminal() && !self.in_place)
    }

    pub(crate) fn color_errors(&self) -> bool {
        self.color_if(|| io::stderr().is_terminal())
    }

    pub(crate) fn indent(&self) -> String {
        if self.tab { "\t".into() } else { " ".repeat(self.indent.unwrap_or(2)) }
    }
}

#[derive(Debug)]
pub(crate) enum Error {
    Flag(String),
    Utf8(OsString),
    KeyValue(&'static str),
    Int(&'static str),
    Path(&'static str),
    Format(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const FMTS: &str = Format::ALL;
        match self {
            Self::Flag(s) => write!(f, "unknown flag: {s}"),
            Self::Utf8(s) => write!(f, "invalid UTF-8: {}", s.display()),
            Self::KeyValue(o) => write!(f, "{o} expects a key and a value"),
            Self::Int(o) => write!(f, "{o} expects an integer"),
            Self::Path(o) => write!(f, "{o} expects a path"),
            Self::Format(o) => write!(f, "{o} expects a data format (possible values: {FMTS})"),
        }
    }
}

/// Conversion of errors from [`OsString::into_string`].
impl From<OsString> for Error {
    fn from(e: OsString) -> Self {
        Self::Utf8(e)
    }
}

fn parse_format(arg: &'static str, args: &mut Args) -> Result<Format, Error> {
    let err = || Error::Format(arg);
    let fmt = args.next().and_then(|a| a.into_string().ok());
    Format::parse(&fmt.ok_or_else(err)?).ok_or_else(err)
}

fn parse_key_val(arg: &'static str, args: &mut Args) -> Result<(String, OsString), Error> {
    let err = || Error::KeyValue(arg);
    let key = args.next().ok_or_else(err)?.into_string()?;
    let val = args.next().ok_or_else(err)?;
    Ok((key, val))
}

/// Interpretation of positional arguments.
enum Mode {
    Args,
    Files,
}
