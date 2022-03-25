use std::env;
use std::fs::read_to_string;
use std::io::{self};
use std::path::{Path, PathBuf};
use clap::{Args, Parser, Subcommand, AppSettings};

use blstrs::Scalar;
use ff::PrimeField;
use pairing_lib::{Engine, MultiMillerLoop};
use serde::{Deserialize, Serialize};

use lurk::eval::IO;
use lurk::store::{Ptr, Store};
use lurk::writer::Write;

use fcomm::{self, evaluate, Commitment, Error, FileStore, Function, Opening, Proof};

macro_rules! prl {
  ($($arg:expr),*) => { if *fcomm::VERBOSE.get().expect("verbose flag uninitialized") {
    println!($($arg),*) } };
}

/// Functional commitments
#[derive(Parser, Debug)]
#[clap(version, about, long_about = None)]
#[clap(global_setting(AppSettings::DeriveDisplayOrder))]
struct Cli {
  /// Do not evaluate inputs before passing to function when opening. Otherwise inputs are
  /// evaluated, but the evaluation is not proved.
  #[clap(long)]
  no_eval_input: bool,
  
  /// Iteration limit
  #[clap(short, long, default_value = "1000")]
  limit: usize,
  
  /// Exit with error on failed verification
  #[clap(short, long)]
  error: bool,
  
  /// Chain commitment openings. Opening includes commitment to new function along with output.
  #[clap(long)]
  chain: bool,
  
  /// Be verbose
  #[clap(short, long)]
  verbose: bool,
  
  #[clap(subcommand)]
  command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
  /// Commits a function to the scalar store
  Commit(Commit),
  
  /// Creates an opening
  Open(Open),
  
  /// Evaluates an expression
  Eval(Eval),
  
  /// Generates a proof for the given expression
  Prove(Prove),
  
  /// Verifies a proof
  Verify(Verify),
}

#[derive(Args, Debug)]
struct Commit {
  /// Path to function source
  #[clap(short, long, parse(from_os_str))]
  function: PathBuf,
  
  /// Path to functional commitment
  #[clap(short, long, parse(from_os_str))]
  commitment: PathBuf,
}

#[derive(Args, Debug)]
struct Open {
  /// Path to function source
  #[clap(short, long, parse(from_os_str))]
  function: PathBuf,
  
  /// Path to function input
  #[clap(short, long, parse(from_os_str))]
  input: PathBuf,
  
  /// Path to proof input
  #[clap(short, long, parse(from_os_str))]
  proof: PathBuf,
  
  /// Path to functional commitment (required if chaining openings)
  #[clap(short, long, parse(from_os_str))]
  commitment: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct Eval {
  /// Path to expression source
  #[clap(short = 'x', long, parse(from_os_str))]
  expression: PathBuf,
}

#[derive(Args, Debug)]
struct Prove {
  /// Path to expression source
  #[clap(short = 'x', long, parse(from_os_str))]
  expression: PathBuf,
  
  /// Path to proof input
  #[clap(short, long, parse(from_os_str))]
  proof: PathBuf,
}

#[derive(Args, Debug)]
struct Verify {
  /// Path to proof input
  #[clap(short, long, parse(from_os_str))]
  proof: PathBuf,
}

impl Commit {
  fn commit(&self, limit: usize) -> Result<(), Error> {
    let s = &mut Store::<Scalar>::default();
    
    let mut function = Function::read_from_path(&self.function)?;
    let fun_ptr = function.fun_ptr(s, limit);
    let commitment = if let Some(secret) = function.secret {
      Commitment::from_ptr_and_secret(s, &fun_ptr, secret)
    } else {
      let (commitment, secret) = Commitment::from_ptr_with_hiding(s, &fun_ptr);
      function.secret = Some(secret);
      
      function.write_to_path(&self.function);
      
      commitment
    };
    commitment.write_to_path(&self.commitment);
    
    Ok(())
  }
}

impl Open {
  fn open(&self, chain: bool, limit: usize, no_eval_input: bool) -> Result<(), Error> {
    let mut s = Store::<Scalar>::default();
    
    let function = Function::read_from_path(&self.function)?;
    let input = input(&mut s, &self.input, no_eval_input, limit)?;
    let out_path = &self.proof;
    
    // Needed if we are creating a chained commitment.
    let chained_function_path = chain.then(|| path_successor(&self.function));
    
    let proof = Opening::create_and_prove(
      &mut s,
      input,
      function,
      limit,
      chain,
      self.commitment.as_ref(),
      chained_function_path,
    )?;
    
    // Write first, so prover can debug if proof doesn't verify (it should).
    proof.write_to_path(out_path);
    proof.verify().expect("created opening doesn't verify");
    
    Ok(())
  }
}

impl Eval {
  fn eval(&self, limit: usize) -> Result<(), Error> {
    let mut s = Store::<Scalar>::default();
    
    let expr = expression(&mut s, &self.expression)?;
    
    let (out_expr, iterations) = evaluate(&mut s, expr, limit);
    
    println!("[{} iterations] {}", iterations, out_expr.fmt_to_string(&s));
    
    Ok(())
  }
} 

impl Prove {
  fn prove(&self, limit: usize) -> Result<(), Error> {
    let mut s = Store::<Scalar>::default();
    
    let expr = expression(&mut s, &self.expression)?;
    
    let proof = Proof::eval_and_prove(&mut s, expr, limit)?;
    
    // Write first, so prover can debug if proof doesn't verify (it should).
    proof.write_to_path(&self.proof);
    proof.verify().expect("created proof doesn't verify");
    
    Ok(())
  }
}  

impl Verify {
  fn verify(&self, cli_error: bool) -> Result<(), Error> {
    let result = proof(Some(&self.proof))?.verify()?;
    
    serde_json::to_writer(io::stdout(), &result)?;
    
    if result.verified {
      prl!("Verification succeeded.");
    } else if cli_error {
      serde_json::to_writer(io::stderr(), &result)?;
      std::process::exit(1);
    };
    
    Ok(())
  }
}

fn read_from_path<P: AsRef<Path>, F: PrimeField + Serialize>(
  store: &mut Store<F>,
  path: P
) -> Result<Ptr<F>, Error> {
  let path = env::current_dir()?.join(path);
  
  let input = read_to_string(path)?;
  
  let src = store.read(&input).unwrap();
  
  Ok(src)
}  

fn read_eval_from_path<P: AsRef<Path>, F: PrimeField + Serialize>(
  store: &mut Store<F>,
  path: P,
  limit: usize
) -> Result<(Ptr<F>, Ptr<F>), Error> {
  let src = read_from_path(store, path)?;
  let (
    IO {
      expr,
      env: _,
      cont: _,
    },
    _iterations,
  ) = evaluate(store, src, limit);
  
  Ok((expr, src))
}

fn read_no_eval_from_path<P: AsRef<Path>, F: PrimeField + Serialize>(
  store: &mut Store<F>,
  path: P,
) -> Result<(Ptr<F>, Ptr<F>), Error> {
  let src = read_from_path(store, path)?;
  
  let quote = store.sym("quote");
  let quoted = store.list(&[quote, src]);
  Ok((quoted, src))
}

fn path_successor<P: AsRef<Path>>(path: P) -> PathBuf {
  let p = path.as_ref().to_path_buf();
  let new_index = if let Some(extension) = p.extension() {
    let index = if let Some(e) = extension.to_str() {
      e.to_string().parse::<usize>().unwrap_or(0) + 1
    } else {
      1
    };
    
    index
  } else {
    1
  };
  let mut new_path = p;
  new_path.set_extension(new_index.to_string());
  
  new_path
}

fn _lurk_function<P: AsRef<Path>, F: PrimeField + Serialize>(
  store: &mut Store<F>,
  function_path: P,
  limit: usize
) -> Result<(Ptr<F>, Ptr<F>), Error> {
  let (function, src) = read_eval_from_path(store, function_path, limit)
    .expect("failed to read function");
  assert!(function.is_fun(), "FComm can only commit to functions.");
  
  Ok((function, src))
}

fn input<P: AsRef<Path>, F: PrimeField + Serialize>(store: &mut Store<F>, input_path: P, no_eval_input: bool, limit: usize) -> Result<Ptr<F>, Error> {
  let input = if no_eval_input {
    let (quoted, _src) = read_no_eval_from_path(store, input_path)?;
    
    quoted
  } else {
    let (evaled_input, _src) = read_eval_from_path(store, input_path, limit)?;
    evaled_input
  };
  
  Ok(input)
}

fn expression<P: AsRef<Path>, F: PrimeField + Serialize>(store: &mut Store<F>, expression_path: P) -> Result<Ptr<F>, Error> {
  let input = read_from_path(store, expression_path)?;
  
  Ok(input)
}

// Get proof from supplied path or else from stdin.
fn proof<P: AsRef<Path>, E: Engine + MultiMillerLoop>(proof_path: Option<P>) -> Result<Proof<E>, Error>
where
  for<'de> <E as Engine>::Gt: blstrs::Compress + Serialize + Deserialize<'de>,
for<'de> <E as Engine>::G1: Serialize + Deserialize<'de>,
for<'de> <E as Engine>::G1Affine: Serialize + Deserialize<'de>,
for<'de> <E as Engine>::G2Affine: Serialize + Deserialize<'de>,
for<'de> <E as Engine>::Fr: Serialize + Deserialize<'de>,
for<'de> <E as Engine>::Gt: blstrs::Compress + Serialize + Deserialize<'de>,
{
  match proof_path {
    Some(path) => Proof::read_from_path(path),
    None => Proof::read_from_stdin()
  }
}

fn main() -> Result<(), Error> {
  pretty_env_logger::init();
  
  let cli = Cli::parse();
  
  fcomm::VERBOSE
      .set(cli.verbose)
      .expect("could not set verbose flag");
  
  match &cli.command {
    Command::Commit(c)=> {
      c.commit(cli.limit)
    },
    Command::Open(o) => {
      o.open(cli.chain, cli.limit, cli.no_eval_input)
    },
    Command::Eval(e) => {
      e.eval(cli.limit)
    },
    Command::Prove(p) => {
      p.prove(cli.limit)
    },
    Command::Verify(v) => {
      v.verify(cli.error)
    },
  }
}
