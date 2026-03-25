use std::env;

/// Target code
pub enum EmitTarget {
  Abstract,
  X86_64,
  LLVM,
}

/// Configuration options for the compiler
pub struct Config {
  pub verbose: bool,
  pub dump_ast: bool,
  pub check: bool,
  pub optimizer_level: u8,
  pub dump_ir: bool,
  pub target: EmitTarget,
  pub header: Option<String>,
  pub source: Option<String>,
}

impl Config {
  fn defalut() -> Self {
    Config {
      verbose: false,
      dump_ast: false,
      check: false,
      optimizer_level: 0,
      dump_ir: false,
      target: EmitTarget::Abstract,
      header: None,
      source: None,
    }
  }
}

pub fn parse_args() -> Config {
  let args: Vec<String> = env::args().collect();
  let mut config = Config::defalut();

  let mut index = 1;
  while index < args.len() {
    match args[index].as_str() {
      "-v" | "--verbose" => config.verbose = true,
      "--dump-ast" => config.dump_ast = true,
      "-t" | "--typecheck-only" => config.check = true,
      arg if arg.starts_with("-O") => {
        if arg == "-O" {
          config.optimizer_level = 1;
        } else {
          config.optimizer_level = arg[2..].parse::<u8>().expect("Invalid optimization level")
        }
      }
      "--dump-ir" => config.dump_ir = true,
      "-e" | "--emit" => {
        if index + 1 < args.len() {
          match args[index + 1].as_str() {
            "abs" => config.target = EmitTarget::Abstract,
            "x86-64" => config.target = EmitTarget::X86_64,
            "llvm" => config.target = EmitTarget::LLVM,
            other => panic!("Unknown target {}", other),
          };
          index += 1;
        } else {
          panic!("Expected target");
        }
      }
      "-eabs" => config.target = EmitTarget::Abstract,
      "-ex86-64" => config.target = EmitTarget::X86_64,
      "-ellvm" => config.target = EmitTarget::LLVM,
      "-l" | "--link" => {
        if index + 1 < args.len() {
          config.header = Some(args[index + 1].clone());
          index += 1;
        } else {
          panic!("Expected header file");
        }
      }
      file => {
        if let Some('-') = file.chars().next() {
          panic!("Unknown flag {}", file);
        } else {
          config.source = Some(file.to_string());
        }
      }
    };
    index += 1;
  }

  if config.source.is_none() {
    panic!("Expected source file");
  }

  config
}
