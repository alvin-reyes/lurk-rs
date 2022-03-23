use std::path::PathBuf;
use clap::{Parser, Subcommand};

use lurk::repl::repl;

/// Lurk CLI
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Cli {
  /// Eval command
  #[clap(subcommand)]
  command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
  /// Evaluates a Lurk file
  Eval {
    /// Input file
    #[clap(parse(from_os_str))]
    path: PathBuf,
  },
}

fn eval(path: &PathBuf) {
  if path.exists() {
    repl(Some(path)).expect("Failed to evaluate")
  }
  else {
    println!("Err: No such file or directory")
  }
}

fn main() {
  let cli = Cli::parse();

  if let Some(cmd) = &cli.command {
    match cmd {
      Command::Eval{path} => {
	eval(path);
      }
    }
  }
}
