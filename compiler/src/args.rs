use std::env;

/// Target code
pub enum EmitTarget {
  Abstract,
  X86_64,
}

/// Configuration options for the compiler
pub struct Config {
  pub verbose: bool,
  pub check: bool,
  pub optimizer_level: u8,
  pub target: EmitTarget,
  pub file: Option<String>,
}

impl Config {
  fn defalut() -> Self {
    Config {
      verbose: false,
      check: false,
      optimizer_level: 0,
      target: EmitTarget::Abstract,
      file: None,
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
      "-c" | "--check" => config.check = true,
      arg if arg.starts_with("-O") => {
        if arg == "-O" {
          config.optimizer_level = 1;
        } else {
          config.optimizer_level = arg[2..].parse::<u8>().expect("Invalid optimization level")
        }
      }
      "-t" | "--target" => {
        if index + 1 < args.len() {
          match args[index + 1].as_str() {
            "abs" => config.target = EmitTarget::Abstract,
            "x86-64" => config.target = EmitTarget::X86_64,
            other => panic!("Unknown target {}", other),
          };
          index += 1;
        } else {
          panic!("Expected target");
        }
      }
      file => {
        if let Some('-') = file.chars().next() {
          panic!("Unknown flag {}", file);
        } else {
          config.file = Some(file.to_string());
        }
      }
    };
    index += 1;
  }

  if config.file.is_none() {
    panic!("Expected file input");
  }

  config
}
