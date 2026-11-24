//! What the descriptions in a folder make of a line: for trying a
//! description before it is trusted.
//!
//!   cargo run -p unifyterm-programs --example try -- FOLDER complete|problem|explain|help|brief LINE
//!
//! FOLDER: where the descriptions are (`ut program directory` prints UnifyTerm's).
//! Prints what the window would show, and how long working it out took.

use std::time::Instant;

use unifyterm_programs::{brief, complete, explain, help, load, problem};

fn main() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [folder, what, line] = arguments.as_slice() else {
        return Err("usage: try FOLDER complete|problem|explain|help|brief LINE".into());
    };
    let started = Instant::now();
    let (programs, refused) = load(std::path::Path::new(folder));
    let read_in = started.elapsed();
    for line in refused {
        println!("not read: {line}");
    }
    let started = Instant::now();
    let said = match what.as_str() {
        "complete" => complete(&programs, line, 6)
            .into_iter()
            .map(|offer| format!("{}    {}", offer.line, offer.note))
            .collect::<Vec<_>>()
            .join("\n"),
        "problem" => problem(&programs, line).map_or_else(
            || "(no problem)".to_owned(),
            |problem| format!("{}\n=> {}", problem.says, problem.corrected.unwrap_or_else(|| "(nothing near)".into())),
        ),
        "explain" => explain(&programs, line).map_or_else(
            || "(not explained here)".to_owned(),
            |told| {
                told.parts
                    .into_iter()
                    .map(|(text, meaning)| format!("{text}    {meaning}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
        ),
        "help" => help(&programs, line, 8 * 1024).unwrap_or_else(|| "(no described program on the line)".into()),
        "brief" => brief(&programs, &[line], 16 * 1024),
        other => return Err(format!("'{other}' is none of complete, problem, explain, help, brief")),
    };
    println!("{said}");
    println!(
        "-- {} descriptions read in {read_in:.1?}; worked out in {:.1?}; {} bytes",
        programs.len(),
        started.elapsed(),
        said.len()
    );
    Ok(())
}
