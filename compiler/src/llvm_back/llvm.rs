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

  let struct_defs = collect_struct_definitions(header_ast, source_ast);
  if !struct_defs.is_empty() {
    out.push_str(&struct_defs.join("\n"));
    out.push_str("\n\n");
  }

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
  let mut needs_malloc = false;
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
        &mut needs_malloc,
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

  if needs_malloc
    && (!function_signatures.contains_key("malloc")
      || source_defined.contains(&"malloc".to_string()))
  {
    out.push_str("declare ptr @malloc(i64)\n");
  }

  out
}

/// Generate LLVM struct type definitions.
fn collect_struct_definitions(header_ast: &ProgramAST, source_ast: &ProgramAST) -> Vec<String> {
  let mut seen = HashSet::new();
  let mut defs = Vec::new();

  for program in [header_ast, source_ast] {
    for decl in program {
      if let GlobalDeclaration::SDefn(struct_id, fields) = decl
        && seen.insert(struct_id.clone())
      {
        let fields = fields
          .iter()
          .map(|(typ, _)| llvm_type(typ))
          .collect::<Vec<_>>()
          .join(", ");
        defs.push(format!("%struct.{struct_id} = type {{ {fields} }}"));
      }
    }
  }

  defs
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
      GlobalDeclaration::SDefn(..) => {}
      GlobalDeclaration::SDecl(..) | GlobalDeclaration::TDefn(..) => {}
    }
  }
}

/// Transform C0 type to LLVM type.
fn llvm_type(typ: &Typ) -> &'static str {
  match typ {
    Typ::Void => "void",
    Typ::Int => "i32",
    Typ::Bool => "i1",
    Typ::Null | Typ::Pointer(..) | Typ::Array(..) | Typ::Struct(..) => "ptr",
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
  needs_malloc: &mut bool,
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
        if is_ptr_like(&operand_type(lhs)) || is_ptr_like(&operand_type(rhs)) {
          let llvm_op = match op {
            BinOp::CmpEq => "eq",
            BinOp::CmpNeq => "ne",
            _ => unreachable!("Pointer comparison only supports equality and inequality."),
          };

          return format!(
            "\t%t{} = icmp {llvm_op} ptr {}, {}\n",
            dest.0,
            emit_ptr(lhs),
            emit_ptr(rhs)
          );
        }

        if matches!(operand_type(lhs), Typ::Bool) && matches!(operand_type(rhs), Typ::Bool) {
          let llvm_op = match op {
            BinOp::CmpEq => "eq",
            BinOp::CmpNeq => "ne",
            BinOp::Lt => "slt",
            BinOp::Gt => "sgt",
            BinOp::Lte => "sle",
            BinOp::Gte => "sge",
            _ => unreachable!(),
          };

          return format!(
            "\t%t{} = icmp {llvm_op} i1 {}, {}\n",
            dest.0,
            emit_i1(lhs),
            emit_i1(rhs)
          );
        }

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
        if matches!(op, BinOp::Add | BinOp::Sub) {
          let lhs_typ = operand_type(lhs);
          let rhs_typ = operand_type(rhs);

          if is_ptr_like(&lhs_typ) || is_ptr_like(&rhs_typ) {
            let (ptr_op, offset_op, negate) = if is_ptr_like(&lhs_typ) && is_int_like(&rhs_typ) {
              (lhs, rhs, matches!(op, BinOp::Sub))
            } else if matches!(op, BinOp::Add) && is_int_like(&lhs_typ) && is_ptr_like(&rhs_typ) {
              (rhs, lhs, false)
            } else {
              unreachable!("Unsupported pointer arithmetic in LLVM lowering.")
            };

            let (offset, mut prefix) = cast_to_i64(offset_op, aux_counter);
            let offset = if negate {
              let neg_aux = format!("%aux{}", *aux_counter);
              *aux_counter += 1;
              prefix.push_str(&format!("\t{neg_aux} = sub i64 0, {offset}\n"));
              neg_aux
            } else {
              offset
            };

            return format!(
              "{prefix}\t%t{} = getelementptr inbounds i8, ptr {}, i64 {offset}\n",
              dest.0,
              emit_ptr(ptr_op)
            );
          }
        }

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
    Instr::TailCall { name, args } => {
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

      if function_signature.return_typ == Typ::Void {
        format!(
          "\ttail call {} @{}({})\n\tret void\n",
          llvm_type(&function_signature.return_typ),
          callee,
          arg_strings.join(", ")
        )
      } else {
        let tail_result = format!("%aux{}", *aux_counter);
        *aux_counter += 1;
        format!(
          "\t{tail_result} = tail call {} @{}({})\n\tret {} {tail_result}\n",
          llvm_type(&function_signature.return_typ),
          callee,
          arg_strings.join(", "),
          llvm_type(return_type),
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
            Operand::Const((value, _)) => emit_const_of_typ(*value, &dest.1),
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
        Typ::Null | Typ::Pointer(..) | Typ::Array(..) | Typ::Struct(..) => {
          format!("\t%t{} = select i1 true, ptr {src}, ptr null\n", dest.0)
        }
        Typ::Void => unreachable!("Typechecker erroneously allows operands to be of type void."),
        Typ::Typedef(typedef) => {
          unreachable!("Unresolved typedef {typedef} found in LLVM backend.")
        }
      }
    }
    Instr::Load { .. } | Instr::Store { .. } | Instr::Alloc { .. } | Instr::AllocArray { .. } => {
      match instr {
        Instr::Load { dest, addr } => {
          format!(
            "\t%t{} = load {}, ptr {}\n",
            dest.0,
            llvm_type(&dest.1),
            emit_ptr(addr)
          )
        }
        Instr::Store { addr, src } => {
          format!(
            "\tstore {} {}, ptr {}\n",
            llvm_type(&operand_type(src)),
            emit_operand_of_typ(src, &operand_type(src)),
            emit_ptr(addr)
          )
        }
        Instr::Alloc { dest, size } => {
          *needs_malloc = true;
          format!(
            "\t%t{} = call ptr @malloc(i64 {})\n",
            dest.0,
            cast_to_i64(size, aux_counter).0
          )
        }
        Instr::AllocArray { dest, size, count } => {
          *needs_malloc = true;
          let (size, mut prefix): (String, String) = cast_to_i64(size, aux_counter);
          let (count, count_prefix): (String, String) = cast_to_i64(count, aux_counter);
          prefix.push_str(&count_prefix);
          let bytes_aux = format!("%aux{}", *aux_counter);
          *aux_counter += 1;
          prefix.push_str(&format!("\t{bytes_aux} = mul i64 {size}, {count}\n"));
          format!(
            "{prefix}\t%t{} = call ptr @malloc(i64 {bytes_aux})\n",
            dest.0
          )
        }
        _ => unreachable!(),
      }
    }
  }
}

/// Stringify an operand of a given type.
fn emit_operand_of_typ(op: &Operand, typ: &Typ) -> String {
  match typ {
    Typ::Bool => emit_i1(op),
    Typ::Int => emit_i32(op),
    Typ::Null | Typ::Pointer(..) | Typ::Array(..) | Typ::Struct(..) => emit_ptr(op),
    Typ::Void => unreachable!("Typechecker erroneously allows operands to be of type void."),
    Typ::Typedef(typedef) => unreachable!("Unresolved typedef {typedef} found in LLVM backend."),
  }
}

/// Stringify an immediate of a given type.
fn emit_const_of_typ(value: i64, typ: &Typ) -> String {
  match typ {
    Typ::Bool => {
      if value == 0 {
        "false".to_string()
      } else {
        "true".to_string()
      }
    }
    Typ::Int => value.to_string(),
    Typ::Null | Typ::Pointer(..) | Typ::Array(..) | Typ::Struct(..) => "null".to_string(),
    Typ::Void => {
      unreachable!("Typechecker erroneously permits constants assigned to the void type.")
    }
    Typ::Typedef(typedef) => unreachable!("Unresolved typedef {typedef} found in LLVM backend."),
  }
}

/// Stringify a boolean operand.
fn emit_i1(op: &Operand) -> String {
  match op {
    Operand::Const((value, _)) => {
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
    Operand::Const((value, _)) => value.to_string(),
    Operand::Temp((id, typ)) => match typ {
      Typ::Int => format!("%t{}", id),
      _ => unreachable!("Typechecker erroneously allows non-int expressions where unacceptable."),
    },
  }
}

/// Stringify an operand as an i32 (cast booleans into i32, ints remain unchanged)
fn cast_to_i32(op: &Operand, aux_counter: &mut usize) -> (String, String) {
  match op {
    Operand::Const((value, _)) => (value.to_string(), String::new()),
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
      _ => unreachable!("Bad typecast to boolean in LLVM code generation."),
    },
  }
}

/// Stringify an operand as an i64.
fn cast_to_i64(op: &Operand, aux_counter: &mut usize) -> (String, String) {
  match op {
    Operand::Const((value, _)) => (value.to_string(), String::new()),
    Operand::Temp((id, typ)) => match typ {
      Typ::Bool => {
        let temp_casted = format!("%aux{}", *aux_counter);
        *aux_counter += 1;
        (
          temp_casted.clone(),
          format!("\t{temp_casted} = zext i1 %t{id} to i64\n"),
        )
      }
      Typ::Int => {
        let temp_casted = format!("%aux{}", *aux_counter);
        *aux_counter += 1;
        (
          temp_casted.clone(),
          format!("\t{temp_casted} = sext i32 %t{id} to i64\n"),
        )
      }
      _ => unreachable!("Bad typecast to i64 in LLVM code generation."),
    },
  }
}

/// Get the type of an operand.
fn operand_type(op: &Operand) -> Typ {
  match op {
    Operand::Const((_, typ)) | Operand::Temp((_, typ)) => typ.clone(),
  }
}

/// Check whether a type should be treated as a pointer/reference in LLVM lowering.
fn is_ptr_like(typ: &Typ) -> bool {
  matches!(
    typ,
    Typ::Null | Typ::Pointer(..) | Typ::Array(..) | Typ::Struct(_)
  )
}

/// Check whether a type is integer-like for pointer arithmetic.
fn is_int_like(typ: &Typ) -> bool {
  matches!(typ, Typ::Int | Typ::Bool)
}

/// Stringify a pointer-like operand.
fn emit_ptr(op: &Operand) -> String {
  match op {
    Operand::Const((value, _)) => {
      if *value == 0 {
        "null".to_string()
      } else {
        format!("inttoptr (i64 {value} to ptr)")
      }
    }
    Operand::Temp((id, _)) => format!("%t{id}"),
  }
}
