use clap::{Parser, Subcommand};
use archwire_core::diag::{Diagnostics, Problem};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "archwire", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compile a definition file into a draw.io diagram
    Compile {
        def: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Rewrite one place in a definition file in place
    Set {
        def: PathBuf,
        /// `flows[i][j]`, `set.<key>` or `choose.<hole id>`
        at: String,
        value: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Set { def, at, value } => {
            let text = match std::fs::read_to_string(&def) {
                Ok(t) => t,
                Err(e) => return fail("CANNOT_READ", &def, format!("cannot read the definition file: {e}")),
            };
            match archwire_core::edit::set_value(&text, &at, &value) {
                Ok(new_text) => match std::fs::write(&def, new_text) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => fail("CANNOT_WRITE", &def, format!("cannot write the definition file: {e}")),
                },
                Err(problem) => report(vec![problem]),
            }
        }
        Command::Compile { def, out } => {
            let text = match std::fs::read_to_string(&def) {
                Ok(t) => t,
                Err(e) => {
                    let mut errors = vec![problem("CANNOT_READ", &def, format!("cannot read the definition file: {e}"))];
                    errors.extend(remove_stale(&out));
                    return report(errors);
                }
            };
            let mut output = archwire_core::compile(&text);
            match &output.drawio {
                Some(xml) => {
                    if let Err(e) = std::fs::write(&out, xml) {
                        output.diagnostics.errors.push(problem("CANNOT_WRITE", &out, format!("cannot write the output file: {e}")));
                    }
                }
                // No diagram: a file left from an earlier compile must not pass for this one's.
                None => output.diagnostics.errors.extend(remove_stale(&out)),
            }
            print!("{}", output.diagnostics.to_json());
            ExitCode::from(output.diagnostics.exit_code() as u8)
        }
    }
}

fn problem(code: &str, path: &Path, message: String) -> Problem {
    Problem { code: code.into(), got: Some(path.display().to_string()), message, ..Default::default() }
}

/// Removes the output file of an earlier compile, if there is one; a file that cannot be removed is an error.
fn remove_stale(out: &Path) -> Option<Problem> {
    match std::fs::remove_file(out) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Some(problem("CANNOT_WRITE", out, format!("cannot remove the output file of an earlier compile: {e}")))
        }
        _ => None,
    }
}

/// A diagnostics report with only these errors, and exit 1.
fn report(errors: Vec<Problem>) -> ExitCode {
    let diagnostics = Diagnostics { versions: archwire_core::versions(), errors, ..Default::default() };
    print!("{}", diagnostics.to_json());
    ExitCode::from(1)
}

/// A file the command could not touch: a diagnostics report with that one error, and exit 1.
fn fail(code: &str, path: &Path, message: String) -> ExitCode {
    report(vec![problem(code, path, message)])
}
