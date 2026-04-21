use std::env;

/// Target (output) language for the compiler.
pub enum EmitTarget {
  /// SSA-based 3-operand abstract assembly (IR)
  Abstract,
  /// SysV AMD64 GNU/AT&T assembly
  X86_64,
  /// LLVM IR
  Llvm,
}

/// Configuration options for the compiler.
pub struct Config {
  /// Print time profiling by stages to terminal.
  pub verbose: bool,
  /// Print abstract syntax tree to terminal.
  pub dump_ast: bool,
  /// Stop after lexical, syntactic, and semantic analysis.
  pub check: bool,
  /// Desired level of optimizations.
  pub optimizer_level: u8,
  /// Enable or disable memory and arithmetic safety checks.
  pub allow_unsafe: bool,
  /// Print SSA intermediate representation to terminal.
  pub dump_ir: bool,
  /// Target language for the compiler.
  pub target: EmitTarget,
  /// Path to file of external declarations to link with source code.
  pub header: Option<String>,
  /// Path to source code file.
  pub source: Option<String>,
}

impl Config {
  /// Generate a default config options template.
  fn defalut() -> Self {
    Config {
      verbose: false,
      dump_ast: false,
      check: false,
      optimizer_level: 0,
      allow_unsafe: false,
      dump_ir: false,
      target: EmitTarget::Abstract,
      header: None,
      source: None,
    }
  }
}

/// Produce a config from arguments passed by command line.
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
      "-u" | "--unsafe" => config.allow_unsafe = true,
      "--dump-ir" => config.dump_ir = true,
      "-e" | "--emit" => {
        if index + 1 < args.len() {
          match args[index + 1].as_str() {
            "abs" => config.target = EmitTarget::Abstract,
            "x86-64" => config.target = EmitTarget::X86_64,
            "llvm" => config.target = EmitTarget::Llvm,
            other => panic!("Unknown target {}", other),
          };
          index += 1;
        } else {
          panic!("Expected target");
        }
      }
      "-eabs" => config.target = EmitTarget::Abstract,
      "-ex86-64" => config.target = EmitTarget::X86_64,
      "-ellvm" => config.target = EmitTarget::Llvm,
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
