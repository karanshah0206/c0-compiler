use std::collections::{HashMap, HashSet};

use crate::common::symbol_table::SymbolTable;
use crate::front::ast::*;
use crate::intermediate::{
  ir_asm::{Exception, Instr, Operand},
  ir_codegen::ProgramIR,
};

/// An LLVM Function Signature.
struct FunctionSignature {
  /// Function return type.
  return_typ: Typ,
  /// Function parameter types in order.
  param_typs: Vec<Typ>,
}

/// Generate an emittable LLVM IR string.
pub fn generate_llvm(
  header_ast: &ProgramAST,
  source_ast: &ProgramAST,
  program_ir: &ProgramIR,
  symbol_table: &SymbolTable,
) -> String {
  let mut function_signatures: HashMap<Ident, FunctionSignature> = HashMap::new();

  collect_function_signatures(header_ast, &mut function_signatures);
  collect_function_signatures(source_ast, &mut function_signatures);

  let source_defined = source_ast
    .iter()
    .filter_map(|decl| {
      if let GlobalDeclaration::FDefn(_, name, _, _) = decl {
        Some(name)
      } else {
        None
      }
    })
    .collect::<HashSet<_>>();

  let mut out = String::new();
  out.push_str("; C0 Compiler\n\n");

  let header_defined = function_signatures
    .iter()
    .filter_map(|(func_name, func_sig)| {
      if source_defined.contains(func_name) {
        None
      } else {
        Some(format!(
          "declare {} @{}({})",
          llvm_type(&func_sig.return_typ),
          func_name,
          func_sig
            .param_typs
            .iter()
            .map(llvm_type)
            .collect::<Vec<_>>()
            .join(", ")
        ))
      }
    })
    .collect::<Vec<_>>();

  if !header_defined.is_empty() {
    out.push_str(&header_defined.join("\n"));
    out.push_str("\n\n");
  }

  let mut needs_abort = false;
  let mut aux_counter = 0usize;

  for (func_name, func_ir) in program_ir {
    let (return_type, params) = symbol_table.get_function_signature(func_name).unwrap();

    out.push_str(&format!(
      "define {} @_c0_{}({}) {{\n",
      llvm_type(&return_type),
      func_name,
      params
        .iter()
        .enumerate()
        .map(|(index, typ)| format!("{} %t{}", llvm_type(typ), index))
        .collect::<Vec<_>>()
        .join(", ")
    ));

    for ir_instr in func_ir.linearize() {
      out.push_str(&generate_instr(
        &ir_instr,
        &function_signatures,
        &source_defined,
        &return_type,
        &mut needs_abort,
        &mut aux_counter,
      ));
    }
    out.push_str("}\n\n");
  }

  if needs_abort
    && (!function_signatures.contains_key("abort") || source_defined.contains(&"abort".to_string()))
  {
    out.push_str("declare void @abort()\n");
  }

  out
}

/// Generate LLVM function signatures from functions in the source code.
fn collect_function_signatures(
  program: &ProgramAST,
  function_signatures: &mut HashMap<Ident, FunctionSignature>,
) {
  for g_decl in program {
    match g_decl {
      GlobalDeclaration::FDecl(ret_typ, name, params)
      | GlobalDeclaration::FDefn(ret_typ, name, params, ..) => {
        function_signatures.insert(
          name.clone(),
          FunctionSignature {
            return_typ: ret_typ.clone(),
            param_typs: params.iter().map(|(typ, _)| typ.clone()).collect(),
          },
        );
      }
      GlobalDeclaration::Typedef(..) => {}
    }
  }
}

/// Transform C0 type to LLVM type.
fn llvm_type(typ: &Typ) -> &'static str {
  match typ {
    Typ::Void => "void",
    Typ::Int => "i32",
    Typ::Bool => "i1",
    Typ::Typedef(_) => unreachable!("Unresolved typedefs found in LLVM backend."),
  }
}

/// Generate LLVM instructions from IR instructions.
fn generate_instr(
  instr: &Instr,
  function_signatures: &HashMap<Ident, FunctionSignature>,
  source_defined: &HashSet<&Ident>,
  return_type: &Typ,
  needs_abort: &mut bool,
  aux_counter: &mut usize,
) -> String {
  match instr {
    Instr::BinOp { op, dest, lhs, rhs } => match op {
      BinOp::LAnd | BinOp::LOr => {
        let lhs = emit_i1(lhs);
        let rhs = emit_i1(rhs);

        let llvm_op = if matches!(op, BinOp::LAnd) {
          "and"
        } else {
          "or"
        };
        format!("\t%t{} = {llvm_op} i1 {lhs}, {rhs}\n", dest.0)
      }
      BinOp::CmpEq | BinOp::CmpNeq | BinOp::Gt | BinOp::Lt | BinOp::Gte | BinOp::Lte => {
        let (lhs, lhs_cast) = cast_to_i32(lhs, aux_counter);
        let (rhs, rhs_cast) = cast_to_i32(rhs, aux_counter);

        let llvm_op = match op {
          BinOp::CmpEq => "eq",
          BinOp::CmpNeq => "ne",
          BinOp::Lt => "slt",
          BinOp::Gt => "sgt",
          BinOp::Lte => "sle",
          BinOp::Gte => "sge",
          _ => unreachable!(),
        };

        let mut out = String::new();
        out.push_str(&lhs_cast);
        out.push_str(&rhs_cast);
        out.push_str(&format!(
          "\t%t{} = icmp {llvm_op} i32 {lhs}, {rhs}\n",
          dest.0
        ));
        out
      }
      _ => {
        let lhs = emit_i32(lhs);
        let rhs = emit_i32(rhs);

        let llvm_op = match op {
          BinOp::Add => "add",
          BinOp::Sub => "sub",
          BinOp::Mul => "mul",
          BinOp::Div => "sdiv",
          BinOp::Mod => "srem",
          BinOp::And => "and",
          BinOp::Xor => "xor",
          BinOp::Or => "or",
          BinOp::Sal => "shl",
          BinOp::Sar => "ashr",
          _ => unreachable!(),
        };

        format!("\t%t{} = {llvm_op} i32 {lhs}, {rhs}\n", dest.0)
      }
    },
    Instr::UnOp { op, dest, src } => match op {
      UnOp::Neg => {
        let src = emit_i32(src);
        format!("\t%t{} = sub i32 0, {src}\n", dest.0)
      }
      UnOp::Not => {
        let src = emit_i32(src);
        format!("\t%t{} = xor i32 {src}, -1\n", dest.0)
      }
      UnOp::LNot => {
        let src = emit_i1(src);
        format!("\t%t{} = xor i1 {src}, true\n", dest.0)
      }
    },
    Instr::Label(label) => format!("L{}:\n", label.0),
    Instr::JumpTo(label) => format!("\tbr label %L{}\n", label.0),
    Instr::JumpIf { pred, holds, fails } => format!(
      "\tbr i1 {}, label %L{}, label %L{}\n",
      emit_i1(pred),
      holds.0,
      fails.0
    ),
    Instr::Call { dest, name, args } => {
      let function_signature = function_signatures.get(name).unwrap();
      let mut arg_strings = Vec::with_capacity(function_signature.param_typs.len());
      for (index, arg_typ) in function_signature.param_typs.iter().enumerate() {
        let arg = emit_operand_of_typ(&args[index], arg_typ);
        arg_strings.push(format!("{} {arg}", llvm_type(arg_typ)));
      }

      let callee = if source_defined.contains(name) {
        &format!("_c0_{name}")
      } else {
        name
      };

      if let Some(dest) = dest {
        format!(
          "\t%t{} = call {} @{}({})\n",
          dest.0,
          llvm_type(&function_signature.return_typ),
          callee,
          arg_strings.join(", ")
        )
      } else {
        format!(
          "\tcall {} @{}({})\n",
          llvm_type(&function_signature.return_typ),
          callee,
          arg_strings.join(", ")
        )
      }
    }
    Instr::Return(value) => match value {
      Some(value) => format!(
        "\tret {} {}\n",
        llvm_type(return_type),
        emit_operand_of_typ(value, return_type)
      )
      .to_string(),
      None => "\tret void\n".to_string(),
    },
    Instr::Throw(exception) => match exception {
      Exception::Abort => {
        *needs_abort = true;
        "\tcall void @abort()\n\tunreachable\n".to_string()
      }
    },
    Instr::Phi { dest, srcs } => {
      let phi_ops = srcs
        .iter()
        .map(|(label, op)| {
          let value = match op {
            Operand::Const(value) => emit_const_of_typ(*value, &dest.1),
            Operand::Temp((id, _)) => format!("%t{}", id),
          };
          format!("[ {}, %L{} ]", value, label.0)
        })
        .collect::<Vec<_>>()
        .join(", ");

      format!("\t%t{} = phi {} {}\n", dest.0, llvm_type(&dest.1), phi_ops)
    }
    Instr::Move { dest, src } => {
      let src = emit_operand_of_typ(src, &dest.1);
      match &dest.1 {
        Typ::Bool => format!("\t%t{} = or i1 {src}, false\n", dest.0).to_string(),
        Typ::Int => format!("\t%t{} = add i32 {src}, 0\n", dest.0).to_string(),
        Typ::Void => unreachable!("Typechecker erroneously allows operands to be of type void."),
        Typ::Typedef(typedef) => {
          unreachable!("Unresolved typedef {typedef} found in LLVM backend.")
        }
      }
    }
  }
}

/// Stringify an operand of a given type.
fn emit_operand_of_typ(op: &Operand, typ: &Typ) -> String {
  match typ {
    Typ::Bool => emit_i1(op),
    Typ::Int => emit_i32(op),
    Typ::Void => unreachable!("Typechecker erroneously allows operands to be of type void."),
    Typ::Typedef(typedef) => unreachable!("Unresolved typedef {typedef} found in LLVM backend."),
  }
}

/// Stringify an immediate of a given type.
fn emit_const_of_typ(value: i32, typ: &Typ) -> String {
  match typ {
    Typ::Bool => {
      if value == 0 {
        "false".to_string()
      } else {
        "true".to_string()
      }
    }
    Typ::Int => value.to_string(),
    Typ::Void => {
      unreachable!("Typechecker erroneously permits constants assigned to the void type.")
    }
    Typ::Typedef(typedef) => unreachable!("Unresolved typedef {typedef} found in LLVM backend."),
  }
}

/// Stringify a boolean operand.
fn emit_i1(op: &Operand) -> String {
  match op {
    Operand::Const(value) => {
      if *value == 0 {
        "false".to_string()
      } else {
        "true".to_string()
      }
    }
    Operand::Temp((id, typ)) => match typ {
      Typ::Bool => format!("%t{}", id),
      _ => {
        unreachable!("Typechecker erroneously allows non-bool expressions where unacceptable.")
      }
    },
  }
}

/// Stringify an integer operand.
fn emit_i32(op: &Operand) -> String {
  match op {
    Operand::Const(value) => value.to_string(),
    Operand::Temp((id, typ)) => match typ {
      Typ::Int => format!("%t{}", id),
      _ => unreachable!("Typechecker erroneously allows non-int expressions where unacceptable."),
    },
  }
}

/// Stringify an operand as an i32 (cast booleans into i32, ints remain unchanged)
fn cast_to_i32(op: &Operand, aux_counter: &mut usize) -> (String, String) {
  match op {
    Operand::Const(value) => (value.to_string(), String::new()),
    Operand::Temp((id, typ)) => match typ {
      Typ::Bool => {
        let temp_casted = format!("%aux{}", *aux_counter);
        *aux_counter += 1;
        (
          temp_casted.clone(),
          format!("\t{temp_casted} = zext i1 %t{id} to i32\n"),
        )
      }
      Typ::Int => (format!("%t{id}"), String::new()),
      Typ::Void => unreachable!("Cannot cast from void to boolean."),
      Typ::Typedef(typedef) => unreachable!("Unresolved typedef {typedef} found in LLVM backend."),
    },
  }
}
