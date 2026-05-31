use std::collections::{HashMap, HashSet};

use crate::intermediate::{
  ir_asm::{Instr, Label, Operand},
  ir_context::IRContext,
  ir_optimization::analysis::cfg_helpers::*,
};

/// Control-flow graph simplification.
pub fn cfg_simplification(ctx: &mut IRContext) -> bool {
  let mut changed = false;
  loop {
    let mut changed_inner = false;
    changed_inner |= cfg_simplification_pass(ctx);
    changed_inner |= merge_linear_block_chains(ctx);
    if !changed_inner {
      break;
    }
    changed = true;
  }
  changed
}

/// Reduce a phi node to a trivial move if possible.
pub fn simplify_trivial_phi(instr: &mut Instr) -> bool {
  let Instr::Phi { dest, srcs } = instr else {
    return false;
  };
  if srcs.len() != 1 {
    return false;
  }
  let src = srcs[0].1.clone();
  *instr = Instr::Move {
    dest: dest.clone(),
    src,
  };
  true
}

/// Remove conditional jumps from block terminators, if possible.
pub fn simplify_terminator(terminator: &mut Instr) -> bool {
  if let Instr::JumpIf { pred, holds, fails } = terminator {
    if let Operand::Const((value, _)) = pred {
      *terminator = Instr::JumpTo(if *value != 0 { *holds } else { *fails });
      return true;
    }
    if holds == fails {
      *terminator = Instr::JumpTo(*holds);
      return true;
    }
  }
  false
}

/// Remove unreachable blocks and simplify phi nodes and block terminators.
fn cfg_simplification_pass(ctx: &mut IRContext) -> bool {
  let mut changed = false;
  let blocks = ctx.get_blocks_mut();
  let reachable = get_reachable_labels(blocks);

  let before = blocks.len();
  blocks.retain(|label, _| reachable.contains(label));
  changed |= before != blocks.len();

  let labels: HashSet<Label> = blocks.keys().copied().collect();
  let mut preds_by_block: HashMap<Label, Vec<Label>> =
    HashMap::from_iter(labels.iter().map(|label| (*label, Vec::new())));
  let edges: Vec<(Label, Label)> = blocks
    .iter()
    .flat_map(|(label, block)| {
      get_successors_of_block(block)
        .into_iter()
        .map(|successor| (*label, successor))
    })
    .collect();

  for (predecessor, successor) in edges {
    if labels.contains(&successor) {
      preds_by_block
        .entry(successor)
        .or_default()
        .push(predecessor);
    }
  }

  for (label, block) in blocks.iter_mut() {
    let new_preds = preds_by_block.remove(label).unwrap_or_default();
    changed |= block.preds.len() != new_preds.len();
    block.preds = new_preds.clone();

    for instr in &mut block.body {
      if let Instr::Phi { srcs, .. } = instr {
        let old_len = srcs.len();
        srcs.retain(|(pred, _)| new_preds.contains(pred));
        changed |= srcs.len() != old_len;
        changed |= simplify_trivial_phi(instr);
      }
    }

    if let Some(terminator) = &mut block.terminator {
      changed |= simplify_terminator(terminator);
    }
  }

  changed
}

/// Merge straight chains of blocks in the control-flow graph.
fn merge_linear_block_chains(ctx: &mut IRContext) -> bool {
  let mut changed = false;

  loop {
    let merge_candidate = {
      let blocks = ctx.get_blocks();
      let mut potential_candidate: Option<(Label, Label)> = None;
      for (label, block) in blocks {
        let Some(Instr::JumpTo(successor)) = block.terminator else {
          continue;
        };

        // can't merge recursive loops
        if successor == *label || successor == Label(0) {
          continue;
        }

        let Some(successor_block) = blocks.get(&successor) else {
          continue;
        };

        // can't merge a non-sole dependency
        if successor_block.preds.len() != 1 || successor_block.preds[0] != *label {
          continue;
        }

        potential_candidate = Some((*label, successor));
        break;
      }
      potential_candidate
    };

    let Some((predecessor, successor)) = merge_candidate else {
      break;
    };

    let successor_block = ctx.get_blocks_mut().remove(&successor).unwrap();
    let (mut successor_body, successor_terminator) =
      (successor_block.body, successor_block.terminator);

    for instr in successor_body.iter_mut() {
      if let Instr::Phi { dest, srcs } = instr
        && srcs.len() == 1
      {
        let src = srcs[0].1.clone();
        *instr = Instr::Move {
          dest: dest.clone(),
          src,
        };
      }
    }

    // merge successor's body into predecessor's body
    let predecessor_block = ctx.get_blocks_mut().get_mut(&predecessor).unwrap();
    predecessor_block.body.extend(successor_body);
    predecessor_block.terminator = successor_terminator;

    // update predecessor in successor of successors, and any phi nodes
    for label in ctx
      .get_blocks()
      .get(&predecessor)
      .map(get_successors_of_block)
      .unwrap_or_default()
    {
      let Some(block) = ctx.get_blocks_mut().get_mut(&label) else {
        continue;
      };

      for succ_pred in block.preds.iter_mut() {
        if *succ_pred == successor {
          *succ_pred = predecessor;
        }
      }

      for instr in block.body.iter_mut() {
        if let Instr::Phi { srcs, .. } = instr {
          for (label, _) in srcs.iter_mut() {
            if *label == successor {
              *label = predecessor;
            }
          }
        }
      }
    }

    changed = true;
  }

  changed
}
