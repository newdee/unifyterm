//! jq: the jaq interpreter behind jq's name and options.
//!
//! jaq publishes its interpreter as libraries and its command line as a
//! program only, so the command line is repeated here: this file and its
//! two neighbors are jaq 3.1.2's `main.rs`, `cli.rs` and `filter.rs` (MIT,
//! © Michael Färber), changed to be called with arguments and return an exit
//! code, and without the interactive `repl` and `--run-tests`.

mod cli;
mod filter;

use cli::{Cli, Format};
use core::fmt::{self, Display, Formatter};
use filter::run;
use jaq_all::data::{Filter, Runner};
use jaq_all::fmts::read;
use jaq_all::fmts::write::{Writer, with_stdout, write};
use jaq_all::jaq_core::Vars;
use jaq_all::json::Val;
use jaq_all::json::write::{Pp, Styles};
use jaq_all::load::{Color, FileReports, FileReportsDisp, Paint};
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// jq's exit codes.
const OK: i32 = 0;
const FALSE_OR_NULL: i32 = 1;
const USAGE_OR_IO: i32 = 2;
const COMPILE: i32 = 3;
const NO_OUTPUT: i32 = 4;
const RUNTIME: i32 = 5;
/// What a shell reports for a command ended by a closed pipe.
const BROKEN_PIPE: i32 = 141;

pub(crate) fn main(args: Vec<OsString>) -> i32 {
    use env_logger::Env;
    // jaq writes what the `debug` and `stderr` filters print through a logger.
    // `try_init`: a logger set earlier in this process stays.
    let _ = env_logger::Builder::from_env(Env::default().filter_or("LOG", "jaq=debug"))
        .format(|buf, record| match record.level() {
            // format error messages (yielded by `stderr`) without newline
            log::Level::Error => write!(buf, "{}", record.args()),
            // format debug messages such as `["DEBUG:", [1, 2, 3]]`
            level => writeln!(buf, "[\"{}:\", {}]", level, record.args()),
        })
        .try_init();

    // last line of defense against I/O errors that happen during printing,
    // e.g. catching broken pipes during error printing
    main_io(args, &mut io::stdin(), &mut io::stdout(), &mut io::stderr())
        .unwrap_or_else(|e: io::Error| Error::from(e).code())
}

fn main_io(args: Vec<OsString>, inp: &mut io::Stdin, out: &mut io::Stdout, err: &mut io::Stderr) -> io::Result<i32> {
    let cli = match Cli::parse(args) {
        Ok(cli) => cli,
        Err(e) => {
            writeln!(err, "Error: {e}")?;
            return Ok(USAGE_OR_IO);
        }
    };

    if cli.version {
        writeln!(out, "jq (jaq, in ut {})", env!("CARGO_PKG_VERSION"))?;
        Ok(OK)
    } else if cli.help {
        write!(out, "{}", include_str!("help.txt"))?;
        Ok(OK)
    } else {
        real_main(&cli, inp, out).or_else(|e| {
            let shown = if cli.color_errors() { ErrorColor::colored(&e) } else { ErrorColor::plain(&e) };
            write!(err, "{shown}")?;
            Ok(e.code())
        })
    }
}

impl Cli {
    fn runner(&self) -> Runner {
        Runner { null_input: self.null_input, color_err: self.color_errors(), writer: self.writer() }
    }

    fn writer(&self) -> Writer {
        Writer { pp: self.pp(), format: self.to.unwrap_or_default(), join: self.join_output }
    }

    fn pp(&self) -> Pp {
        Pp {
            indent: (!self.compact_output).then(|| self.indent()),
            sort_keys: self.sort_keys,
            styles: self.styles(),
            sep_space: !self.compact_output || matches!(self.to, Some(Format::Yaml)),
        }
    }

    fn styles(&self) -> Styles {
        self.color_output()
            .then(Styles::ansi)
            .map(|c| match std::env::var("JQ_COLORS") {
                Err(_) => c,
                Ok(s) => c.parse(&s),
            })
            .unwrap_or_default()
    }
}

fn real_main(cli: &Cli, inp: &mut io::Stdin, out: &mut io::Stdout) -> Result<i32, Error> {
    let var_val = binds(cli)?;
    let (var_names, mut vars): (Vec<String>, Vec<Val>) = var_val.into_iter().unzip();

    let (var_vals, filter) = match &cli.filter {
        None => (Vec::new(), Filter::default()),
        Some(filter) => {
            let (path, code) = match filter {
                cli::Filter::FromFile(path) => (path.into(), std::fs::read_to_string(path)?),
                cli::Filter::Inline(filter) => ("<inline>".into(), filter.clone()),
            };
            filter::parse_compile(&path, &code, &var_names, &cli.library_path).map_err(Error::Report)?
        }
    };
    vars.extend(var_vals);

    let runner = &cli.runner();
    let writer = &runner.writer;

    let unwrap_or_json = |fmt: Option<Format>| fmt.unwrap_or_default();
    let last = if cli.files.is_empty() {
        let input_filename = Val::utf8_str("<stdin>");
        let vars = Vars::new(vars);

        let format = unwrap_or_json(cli.from);
        let s = read::read_string(format, inp.lock())?;
        let inputs = read::read(format, inp.lock(), &s, cli.slurp);
        let input = (&input_filename, inputs);
        with_stdout(out, |out| run(runner, &filter, vars, input, |v| write(out, writer, &v)))?
    } else {
        let mut last = None;
        for file in &cli.files {
            let input_filename = Val::utf8_str(file.as_os_str().to_string_lossy().into_owned());
            let vars = Vars::new(vars.clone());

            let path = Path::new(file);
            let bytes = read::load_file(path).map_err(|e| Error::Io(Some(path.display().to_string()), e))?;
            let format = unwrap_or_json(cli.from.or_else(|| Format::determine(path)));
            let s = read::bytes_str(format, &bytes)?;
            let inputs = read::parse(format, &bytes, s, cli.slurp);
            let input = (&input_filename, inputs);

            if cli.in_place {
                // create a temporary file where output is written to
                let location = path.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or(Path::new("."));
                let mut tmp = tempfile::Builder::new().prefix("jq").tempfile_in(location)?;

                last = run(runner, &filter, vars.clone(), input, |output| write(tmp.as_file_mut(), writer, &output))?;

                // replace the input file with the temporary file
                std::mem::drop(bytes);
                let perms = std::fs::metadata(path)?.permissions();
                tmp.persist(path).map_err(|e| Error::Io(None, e.into()))?;
                std::fs::set_permissions(path, perms)?;
            } else {
                last = with_stdout(out, |out| run(runner, &filter, vars.clone(), input, |v| write(out, writer, &v)))?;
            }
        }
        last
    };

    if cli.exit_status {
        last.map_or_else(|| Err(Error::NoOutput), |b| b.then_some(OK).ok_or(Error::FalseOrNull))
    } else {
        Ok(OK)
    }
}

fn binds(cli: &Cli) -> Result<Vec<(String, Val)>, Error> {
    let arg = cli.arg.iter().map(|(k, s)| {
        let s = s.to_owned();
        Ok((k.to_owned(), Val::utf8_str(s)))
    });
    let argjson = cli.argjson.iter().map(|(k, s)| {
        let err = |e| Error::Parse(format!("{e} (for value passed to `--argjson {k}`)"));
        let s = s.as_bytes();
        Ok((k.to_owned(), read::json::parse_single(s).map_err(err)?))
    });
    let rawfile = cli.rawfile.iter().map(|(k, path)| {
        let err = |e| Error::Io(Some(Path::new(path).display().to_string()), e);
        let s = read::load_file(path).map_err(err)?;
        Ok((k.to_owned(), Val::utf8_str(s)))
    });
    let slurpfile = cli.slurpfile.iter().map(|(k, path)| {
        let err = |e| Error::Io(Some(Path::new(path).display().to_string()), e);
        Ok((k.to_owned(), read::json_array(path).map_err(err)?))
    });

    let positional = cli.args.iter().cloned().map(|s| Ok(Val::from(s)));
    let positional = positional.collect::<Result<Vec<_>, Error>>()?;

    let var_val = arg.chain(rawfile).chain(slurpfile).chain(argjson);
    let mut var_val = var_val.collect::<Result<Vec<_>, Error>>()?;

    var_val.push(("ARGS".to_string(), named_and_positional(&positional, &var_val)));
    let env = std::env::vars().map(|(k, v)| (k.into(), Val::from(v)));
    var_val.push(("ENV".to_string(), Val::obj(env.collect())));

    Ok(var_val)
}

fn named_and_positional(positional: &[Val], named: &[(String, Val)]) -> Val {
    let key = |k: &str| k.to_string().into();
    let positional = positional.iter().cloned();
    let named = named.iter().map(|(var, val)| (key(var), val.clone()));
    let obj = [(key("positional"), positional.collect()), (key("named"), Val::obj(named.collect()))];
    Val::obj(obj.into_iter().collect())
}

#[derive(Debug)]
enum Error {
    Io(Option<String>, io::Error),
    Report(Vec<FileReports<PathBuf>>),
    Parse(String),
    Jaq(jaq_all::json::Error),
    Halt(i32),
    FalseOrNull,
    NoOutput,
}

struct ErrorColor<'e>(&'e Error, Paint);

impl<'e> ErrorColor<'e> {
    /// The error with jaq's colors.
    fn colored(e: &'e Error) -> Self {
        Self(e, |f: &mut Formatter<'_>, c: &Option<Color>, d: &dyn Display| match c {
            Some(color) => color.ansi(f, d),
            None => d.fmt(f),
        })
    }

    /// The error as plain text.
    fn plain(e: &'e Error) -> Self {
        Self(e, |f: &mut Formatter<'_>, _c: &_, d: &dyn Display| d.fmt(f))
    }
}

impl fmt::Display for ErrorColor<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let Self(error, color) = self;
        match error {
            Error::FalseOrNull | Error::NoOutput | Error::Halt(_) => Ok(()),
            Error::Io(_, e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            Error::Io(prefix, e) => {
                write!(f, "Error: ")?;
                if let Some(p) = prefix {
                    write!(f, "{p}: ")?;
                }
                writeln!(f, "{e}")
            }
            Error::Report(reports) => reports.iter().try_for_each(|fr| {
                FileReportsDisp::new(fr).with_paint(*color).with_path(|p| format!("[{}]", p.display())).fmt(f)
            }),
            Error::Parse(e) => writeln!(f, "Error: failed to parse: {e}"),
            Error::Jaq(e) => writeln!(f, "Error: {e}"),
        }
    }
}

impl Error {
    /// The exit code this error ends jq with.
    fn code(self) -> i32 {
        match self {
            Self::FalseOrNull => FALSE_OR_NULL,
            Self::Io(_, e) if e.kind() == io::ErrorKind::BrokenPipe => BROKEN_PIPE,
            Self::Io(_, _) => USAGE_OR_IO,
            Self::Report(_) => COMPILE,
            Self::NoOutput => NO_OUTPUT,
            Self::Parse(_) | Self::Jaq(_) => RUNTIME,
            Self::Halt(exit_code) => exit_code,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(None, e)
    }
}
