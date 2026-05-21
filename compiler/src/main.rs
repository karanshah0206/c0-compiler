mod args;
mod common;
mod emit;
mod front;
mod intermediate;
mod llvm_back;
mod x86_back;

use std::{fs, process, thread, time};

use logos::Logos;

use crate::front::{lexer::Token, parser::parse, semantics};
use crate::intermediate::{ir_codegen, ir_optimize};
use crate::llvm_back::llvm::generate_llvm;
use crate::x86_back::{x86_codegen, x86_optimize, x86_regalloc};

fn main() {
  let config = args::parse_args();

  // Helper macro to time evaluating an expression
  macro_rules! time {
    ( $x:expr ) => {{
      let t1 = time::SystemTime::now();
      let result = $x;
      (result, t1.elapsed().unwrap())
    }};
  }

  let compiler_thread = thread::Builder::new()
    .stack_size(256 * 1024 * 1024)
    .spawn(move || {
      let header_str = if let Some(header_file) = config.header {
        fs::read_to_string(header_file).expect("Could not read header file.")
      } else {
        "".to_string()
      };
      let source_str =
        fs::read_to_string(config.source.clone().unwrap()).expect("Could not read source file.");

      // 1. Lexical analysis
      let (header_token_stream, header_lex_time) =
        time!(Token::lexer(&header_str).spanned().map(|(t, y)| (
          y.start,
          t.expect("Badly formatted header code."),
          y.end
        )));
      let (source_token_stream, source_lex_time) =
        time!(Token::lexer(&source_str).spanned().map(|(t, y)| (
          y.start,
          t.expect("Badly formatted source code."),
          y.end
        )));

      // 2. Syntactic analysis
      let (mut header_ast, header_parse_time) = time!(match parse(header_token_stream) {
        Ok(header_ast) => header_ast,
        Err(e) => {
          eprintln!("Syntax error in header file. {e}");
          return 1;
        }
      });
      let (mut source_ast, source_parse_time) = time!(match parse(source_token_stream) {
        Ok(source_ast) => source_ast,
        Err(e) => {
          eprintln!("Syntax error in source file. {e}");
          return 1;
        }
      });

      // 3. Semantic analysis
      let (symbol_table, sema_time) =
        time!(semantics::analyze_program(&mut header_ast, &mut source_ast));

      if config.dump_ast {
        for ast in &source_ast {
          println!("{ast}");
        }
      }

      if config.check {
        println!("Semantic analysis passes.");
        return 0;
      }

      // 4. Lower AST to IR
      let (mut program_ir, ir_gen_time) =
        time!(ir_codegen::munch_program(&source_ast, &symbol_table));

      if config.dump_ir {
        for (func_name, func_ir) in &program_ir {
          println!("{func_name}");
          for ir_instr in func_ir.linearize() {
            println!("\t{ir_instr}");
          }
        }
      }

      // 5. IR optimization
      let (_, ir_optimize_time) = time!(ir_optimize::optimize(
        &mut program_ir,
        config.optimizer_level,
        config.allow_unsafe
      ));

      if config.verbose {
        println!(
          "Lexing: {}us",
          header_lex_time.as_micros() + source_lex_time.as_micros()
        );
        println!(
          "Parsing: {}us",
          header_parse_time.as_micros() + source_parse_time.as_micros()
        );
        println!("Semantics: {}us", sema_time.as_micros());
        println!("IR Codegen: {}us", ir_gen_time.as_micros());
        println!("IR Optimization: {}us", ir_optimize_time.as_micros());
      }

      match config.target {
        args::EmitTarget::Abstract => {
          emit::emit_ir(config.source.unwrap(), program_ir, symbol_table).is_err() as i32
        }
        args::EmitTarget::X86_64 => {
          // 6. Register allocation
          let (coloring, regalloc_time) = time!(x86_regalloc::register_allocation(
            &mut program_ir,
            &symbol_table,
            config.optimizer_level
          ));

          // 7. x86 assembly generation
          let (mut x86_program, x86_time) = time!(x86_codegen::generate_assembly(
            &program_ir,
            coloring,
            &symbol_table,
            config.allow_unsafe
          ));

          // 8. x86 optimizations
          let (_, x86_optimization_time) = time!(x86_optimize::optimize(
            &mut x86_program,
            config.optimizer_level
          ));

          if config.verbose {
            println!("Register Allocation: {}us", regalloc_time.as_micros());
            println!("x86 Codegen: {}us", x86_time.as_micros());
            println!("x86 Optimization: {}us", x86_optimization_time.as_micros());
          }

          emit::emit_x86(config.source.unwrap(), x86_program).is_err() as i32
        }
        args::EmitTarget::Llvm => {
          let (llvm_str, codegen_time) = time!(generate_llvm(
            &header_ast,
            &source_ast,
            &program_ir,
            &symbol_table
          ));

          if config.verbose {
            println!("LLVM Codegen: {}us", codegen_time.as_micros());
          }

          emit::emit_llvm(config.source.unwrap(), llvm_str).is_err() as i32
        }
      }
    })
    .unwrap();

  process::exit(compiler_thread.join().expect("Compiler failed."))
}
