use std::collections::{HashMap, HashSet};

use crate::intermediate::{
  ir_asm::{Instr, Label, Operand, Temp},
  ir_context::{BasicBlock, IRContext},
};

/// Get the destination temporary of an instruction, if any.
pub fn get_dest_temp_from_instruction(instr: &Instr) -> Option<Temp> {
  match instr {
    Instr::BinOp { dest, .. }
    | Instr::UnOp { dest, .. }
    | Instr::Phi { dest, .. }
    | Instr::Move { dest, .. }
    | Instr::Load { dest, .. }
    | Instr::Alloc { dest, .. }
    | Instr::AllocArray { dest, .. } => Some(dest.clone()),
    Instr::Call {
      dest: Some(dest), ..
    } => Some(dest.clone()),
    _ => None,
  }
}

/// Get all operands used within an instruction.
pub fn get_operands_from_instruction(instr: &Instr) -> Vec<&Operand> {
  match instr {
    Instr::BinOp { lhs, rhs, .. } => vec![lhs, rhs],
    Instr::Move { src, .. } | Instr::UnOp { src, .. } => vec![src],
    Instr::JumpIf { pred, .. } => vec![pred],
    Instr::Call { args, .. } | Instr::TailCall { args, .. } => args.iter().collect(),
    Instr::Return(Some(op)) => vec![op],
    Instr::Phi { srcs, .. } => srcs.iter().map(|(_, op)| op).collect(),
    Instr::Load { addr, .. } => vec![addr],
    Instr::Store { addr, src } => vec![addr, src],
    Instr::Alloc { size, .. } => vec![size],
    Instr::AllocArray { size, count, .. } => vec![size, count],
    Instr::Label(_) | Instr::JumpTo(_) | Instr::Return(None) | Instr::Throw(_) => vec![],
  }
}

/// Get a mapping from all block labels to all their respective predecessor labels.
pub fn get_predecessors_for_all_blocks(ctx: &IRContext) -> HashMap<Label, Vec<Label>> {
  let blocks = ctx.get_blocks();
  let mut preds_by_label: HashMap<Label, Vec<Label>> =
    blocks.keys().map(|label| (*label, Vec::new())).collect();

  for (label, block) in blocks {
    for successor in get_successors_of_block(block) {
      if let Some(predecessors) = preds_by_label.get_mut(&successor) {
        predecessors.push(*label);
      }
    }
  }

  preds_by_label
}

/// Get the successor labels for a given block.
pub fn get_successors_of_block(block: &BasicBlock) -> Vec<Label> {
  match &block.terminator {
    Some(Instr::JumpTo(label)) => vec![*label],
    Some(Instr::JumpIf { holds, fails, .. }) => vec![*holds, *fails],
    _ => vec![],
  }
}

/// Get the count of uses of all temporaries by their `temp_id`.
pub fn compute_uses_of_all_temps(ctx: &IRContext) -> HashMap<usize, usize> {
  let mut uses = HashMap::new();
  for block in ctx.get_blocks().values() {
    for instr in &block.body {
      for operand in get_operands_from_instruction(instr) {
        if let Operand::Temp((temp_id, _)) = operand {
          *uses.entry(*temp_id).or_insert(0) += 1;
        }
      }
    }
    if let Some(terminator) = &block.terminator {
      for operand in get_operands_from_instruction(terminator) {
        if let Operand::Temp((temp_id, _)) = operand {
          *uses.entry(*temp_id).or_insert(0) += 1;
        }
      }
    }
  }
  uses
}

/// Get labels that are reachable within the CFG.
pub fn get_reachable_labels(blocks: &HashMap<Label, BasicBlock>) -> HashSet<Label> {
  let mut reachable = HashSet::new();
  let mut stack = vec![Label(0)];
  while let Some(label) = stack.pop() {
    if reachable.insert(label)
      && let Some(block) = blocks.get(&label)
    {
      for successor in get_successors_of_block(block) {
        if blocks.contains_key(&successor) {
          stack.push(successor);
        }
      }
    }
  }
  reachable
}

/// Get basic block labels in CFG in reverse post-order.
pub fn get_block_labels_in_rpo(ctx: &IRContext) -> Vec<Label> {
  let mut visited: HashSet<Label> = HashSet::new();
  let mut labels = Vec::new();
  if ctx.get_blocks().contains_key(&Label(0)) {
    rpo_dfs_helper(ctx, Label(0), &mut visited, &mut labels);
  }
  labels.reverse();
  labels
}

/// Helper to build list of CFG basic block labels in reverse post-order.
fn rpo_dfs_helper(
  ctx: &IRContext,
  label: Label,
  visited: &mut HashSet<Label>,
  labels: &mut Vec<Label>,
) {
  if visited.insert(label)
    && let Some(block) = ctx.get_blocks().get(&label)
  {
    for successor in get_successors_of_block(block) {
      rpo_dfs_helper(ctx, successor, visited, labels);
    }
  }
  labels.push(label);
}
