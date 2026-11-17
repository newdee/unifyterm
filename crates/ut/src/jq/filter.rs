//! Filter parsing, compilation, and execution. From jaq 3.1.2's
//! `filter.rs` (MIT, © Michael Färber); the native functions are jaq's own
//! set, without the interactive `repl`.
use super::{Error, Runner, Val, read};
use jaq_all::data::Filter;
use jaq_all::jaq_core::{Exn, ValT, Vars, compile, load};
use jaq_all::load::{FileReports, compile_errors, load_errors};
use std::{io, path::PathBuf};

pub(crate) fn parse_compile(
    path: &PathBuf,
    code: &str,
    vars: &[String],
    paths: &[PathBuf],
) -> Result<(Vec<Val>, Filter), Vec<FileReports<PathBuf>>> {
    use compile::Compiler;
    use load::{Arena, File, Loader, import};

    let default = ["~/.jq", "$ORIGIN/../lib/jq", "$ORIGIN/../lib"].map(Into::into);
    let paths = if paths.is_empty() { &default } else { paths };

    let vars: Vec<_> = vars.iter().map(|v| format!("${v}")).collect();
    let arena = Arena::default();
    let loader = Loader::new(jaq_all::defs()).with_std_read(paths);
    let path = path.into();
    let modules = loader.load(&arena, File { path, code }).map_err(load_errors)?;

    let mut imported = Vec::new();
    import(&modules, |p| {
        let path = p.find(paths, "json")?;
        imported.push(read::json_array(path).map_err(|e| e.to_string())?);
        Ok(())
    })
    .map_err(load_errors)?;

    let compiler = Compiler::default().with_funs(jaq_all::data::funs()).with_global_vars(vars.iter().map(|v| &**v));
    let filter = compiler.compile(modules).map_err(compile_errors)?;
    Ok((imported, filter))
}

/// Run a filter with given input values and run `f` for every value output.
pub(crate) fn run(
    runner: &Runner,
    filter: &Filter,
    vars: Vars<Val>,
    input: (&Val, impl Iterator<Item = io::Result<Val>>),
    mut f: impl FnMut(Val) -> io::Result<()>,
) -> Result<Option<bool>, Error> {
    let mut last = None;
    jaq_all::data::run(runner, filter, vars, input, Error::Parse, |v| {
        let halt = |e: Exn<'_, _>| Error::Halt(e.get_halt().expect("an exception that is not an error is a halt"));
        let v = v.map_err(|e| e.get_err().map_or_else(halt, Error::Jaq))?;

        last = Some(v.as_bool());
        Ok(f(v)?)
    })?;
    Ok(last)
}
