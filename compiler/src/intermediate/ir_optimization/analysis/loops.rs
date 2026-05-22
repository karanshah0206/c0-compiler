use std::collections::{HashMap, HashSet};

use crate::front::ast::{BinOp, Typ};
use crate::intermediate::{
  ir_asm::{Instr, Label, Operand},
  ir_context::IRContext,
  ir_optimization::analysis::{cfg_helpers::*, dominator::DominatorTree},
};

/// Information about a natural loop in the control-flow graph.
pub struct NaturalLoop {
  /// Loop header block label.
  pub header: Label,
  /// All basic blocks that constitute the loop body, including header.
  pub body: HashSet<Label>,
  /// Source blocks of back-edges to the header.
  pub back_edges: Vec<Label>,
}

/// Discover all natural loops in the control-flow graph.
pub fn find_natural_loops(ctx: &IRContext, doms: &DominatorTree) -> Vec<NaturalLoop> {
  let predecessors = get_predecessors_for_all_blocks(&ctx);
  let mut by_header: HashMap<Label, Vec<Label>> = HashMap::new();

  for &label in ctx.get_blocks().keys() {
    if doms.dominator.contains_key(&label)
      && let Some(block) = ctx.get_blocks().get(&label)
    {
      for successor in get_successors_of_block(block) {
        if doms.does_dominate(successor, label) {
          by_header.entry(successor).or_default().push(label);
        }
      }
    }
  }

  by_header
    .into_iter()
    .map(|(header, back_edges)| {
      let mut body: HashSet<Label> = HashSet::from([header]);
      let mut stack: Vec<Label> = back_edges.clone();
      while let Some(label) = stack.pop() {
        if body.insert(label)
          && let Some(preds) = predecessors.get(&label)
        {
          for &pred in preds {
            if !body.contains(&pred) {
              stack.push(pred);
            }
          }
        }
      }
      NaturalLoop {
        header,
        body,
        back_edges,
      }
    })
    .collect()
}

/// Check if an instruction inside a loop's body is pure/idempotent.
pub fn is_loop_instr_pure(instr: &Instr, is_unsafe: bool) -> bool {
  let get_operand_typ = |op: Operand| -> Typ {
    match op {
      Operand::Const((_, t)) => t,
      Operand::Temp((_, t)) => t,
    }
  };

  !matches!(
    instr,
    Instr::Store { .. }
      | Instr::Call { .. }
      | Instr::TailCall { .. }
      | Instr::Throw(_)
      | Instr::Alloc { .. }
      | Instr::AllocArray { .. }
      | Instr::Return(_)
      | Instr::JumpTo(_)
      | Instr::JumpIf { .. }
      | Instr::Label(_)
  ) && !match instr {
    Instr::BinOp { op, dest, lhs, rhs } => {
      if matches!(dest.1, Typ::Pointer(_, _)) {
        if !is_unsafe
          && matches!(op, BinOp::Add)
          && matches!(
            get_operand_typ(lhs.clone()),
            Typ::Pointer(..) | Typ::Array(..)
          )
        {
          return true;
        }
        return false;
      }
      match op {
        BinOp::Div | BinOp::Mod => match rhs {
          Operand::Const((c, _)) => {
            *c == 0
              || (*c == -1
                && match lhs {
                  Operand::Const((l, _)) => *l == i32::MIN as i64,
                  Operand::Temp(_) => true,
                })
          }
          _ => true,
        },
        BinOp::Sal | BinOp::Sar => !matches!(rhs, Operand::Const((c, _)) if (0..=31).contains(c)),
        _ => false,
      }
    }
    Instr::Load { .. } => true,
    _ => false,
  }
}
