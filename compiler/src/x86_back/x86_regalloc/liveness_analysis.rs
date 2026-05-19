use std::collections::{HashMap, HashSet};

use crate::intermediate::{
  ir_asm::{Instr, Label, Operand},
  ir_context::{BasicBlock, IRContext},
};

/// Liveness analysis result.
pub struct Liveness {
  #[allow(unused)]
  pub live_in: BlockLiveSets,
  pub live_out: BlockLiveSets,
}

/// Live set indexed by basic block label.
type BlockLiveSets = HashMap<Label, HashSet<usize>>;

/// Compute liveness analysis for every block in a function's IR.
pub fn analyze_liveness(ctx: &IRContext) -> Liveness {
  let blocks = ctx.get_blocks();

  let mut labels: Vec<Label> = blocks.keys().copied().collect();
  labels.sort_by_key(|label| label.0);

  let mut block_uses: BlockLiveSets = HashMap::new();
  let mut block_defines: BlockLiveSets = HashMap::new();

  for label in &labels {
    let block = blocks
      .get(label)
      .unwrap_or_else(|| panic!("Unknown block label {label} found in liveness analysis."));
    let (uses, defines) = analyze_block(block);
    block_uses.insert(*label, uses);
    block_defines.insert(*label, defines);
  }

  let mut successors: HashMap<Label, Vec<Label>> = HashMap::new();
  for label in &labels {
    let block = blocks
      .get(label)
      .unwrap_or_else(|| panic!("Unknown block label {label} found in liveness analysis."));
    successors.insert(
      *label,
      match block.terminator.as_ref() {
        Some(Instr::JumpTo(label)) => vec![*label],
        Some(Instr::JumpIf { holds, fails, .. }) => vec![*holds, *fails],
        _ => vec![],
      },
    );
  }

  let mut live_in: BlockLiveSets = labels
    .iter()
    .map(|&label| (label, HashSet::<usize>::new()))
    .collect();
  let mut live_out: BlockLiveSets = labels
    .iter()
    .map(|&label| (label, HashSet::<usize>::new()))
    .collect();

  let mut changed = true;
  while changed {
    changed = false;
    for &label in &labels {
      let mut new_out: HashSet<usize> = HashSet::new();
      if let Some(successor_list) = successors.get(&label) {
        for successor in successor_list {
          if let Some(successor_in) = live_in.get(successor) {
            for temp_id in successor_in {
              new_out.insert(*temp_id);
            }
          }
        }
      }

      let uses = block_uses.get(&label).unwrap();
      let defines = block_defines.get(&label).unwrap();
      let mut new_in: HashSet<usize> = uses.clone();
      for temp_id in &new_out {
        if !defines.contains(temp_id) {
          new_in.insert(*temp_id);
        }
      }

      if &new_in != live_in.get(&label).unwrap() {
        live_in.insert(label, new_in);
        changed = true;
      }

      if &new_out != live_out.get(&label).unwrap() {
        live_out.insert(label, new_out);
        changed = true;
      }
    }
  }

  Liveness { live_in, live_out }
}

/// If an instruction defines a temporary, get its temp_id.
pub fn get_defines(instr: &Instr) -> Option<usize> {
  match instr {
    Instr::Call {
      dest: Some(dest), ..
    }
    | Instr::Phi { dest, .. }
    | Instr::BinOp { dest, .. }
    | Instr::UnOp { dest, .. }
    | Instr::Move { dest, .. }
    | Instr::Load { dest, .. }
    | Instr::Alloc { dest, .. }
    | Instr::AllocArray { dest, .. } => Some(dest.0),
    _ => None,
  }
}

/// Get the temp_ids used by an instruction.
pub fn get_uses(instr: &Instr) -> Vec<usize> {
  let mut ids = Vec::new();

  let mut push = |op: &Operand| {
    if let Operand::Temp((id, _)) = op {
      ids.push(*id);
    }
  };

  match instr {
    Instr::BinOp { lhs, rhs, .. } => {
      push(lhs);
      push(rhs);
    }
    Instr::UnOp { src, .. } => push(src),
    Instr::JumpIf { pred, .. } => push(pred),
    Instr::Call { args, .. } => {
      for arg in args {
        push(arg);
      }
    }
    Instr::Return(Some(op)) => push(op),
    Instr::Move { src, .. } => push(src),
    Instr::Load { addr, .. } => push(addr),
    Instr::Store { addr, src } => {
      push(addr);
      push(src);
    }
    Instr::Alloc { size, .. } => push(size),
    Instr::AllocArray { size, count, .. } => {
      push(size);
      push(count);
    }
    Instr::Phi { srcs, .. } => {
      for (_, src) in srcs {
        push(src);
      }
    }
    Instr::Label(_) | Instr::JumpTo(_) | Instr::Return(None) | Instr::Throw(_) => {}
  }

  ids
}

/// Analyze uses-defines in a basic block.
fn analyze_block(block: &BasicBlock) -> (HashSet<usize>, HashSet<usize>) {
  let mut uses: HashSet<usize> = HashSet::new();
  let mut defines: HashSet<usize> = HashSet::new();

  let mut walk = |instr: &Instr| {
    for temp_id in get_uses(instr) {
      if !defines.contains(&temp_id) {
        uses.insert(temp_id);
      }
    }
    if let Some(temp_id) = get_defines(instr) {
      defines.insert(temp_id);
    }
  };

  for instr in &block.body {
    walk(instr);
  }
  if let Some(terminator) = &block.terminator {
    walk(terminator);
  }

  (uses, defines)
}
