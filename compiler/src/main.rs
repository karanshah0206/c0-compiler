mod args;

fn main() {
  let config = args::parse_args();

  // Testing configurations
  println!("Verbose: {}", config.verbose);
  println!("Check: {}", config.check);
  println!("Optimizer level: {}", config.optimizer_level);
  println!(
    "Target: {}",
    match config.target {
      args::EmitTarget::Abstract => "abstract",
      args::EmitTarget::X86_64 => "x86-64",
    }
  );
  println!("File: {}", config.file.unwrap_or("No file provided".to_string()));
}
