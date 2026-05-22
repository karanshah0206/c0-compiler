use std::{
  collections::{HashMap, HashSet},
  iter, mem,
};

use crate::intermediate::{
  ir_asm::{Instr, Label, Operand, Temp},
  ir_context::IRContext,
  ir_optimization::analysis::{cfg_helpers::*, dominator::DominatorTree, loops::*},
};

/// Loop-invariant code motion.
/// Hoists pure, non-trapping instructions out of the loop's body.
pub fn licm(ctx: &mut IRContext, is_unsafe: bool) -> bool {
  let dominator_tree = DominatorTree::new(ctx);
  if dominator_tree.reachable.is_empty() {
    return false;
  }

  let mut loops = find_natural_loops(ctx, &dominator_tree);
  if loops.is_empty() {
    return false;
  }

  // using body size to approximate loops depth, attempting to process inner-most first
  loops.sort_by_key(|l| l.body.len());

  let mut changed = false;
  for l in &loops {
    if !ctx.get_blocks().contains_key(&l.header) {
      continue;
    }
    let preheader = match ensure_preheader(ctx, l) {
      Some(p) => p,
      None => continue,
    };
    changed |= hoist_loop_invariant(ctx, l, preheader, is_unsafe);
  }
  changed
}

/// Ensure that a loop has one, unique non-loop preheader block.
/// Creates a preheader if doesn't exist and rewrires phi nodes accordingly.
fn ensure_preheader(ctx: &mut IRContext, l: &NaturalLoop) -> Option<Label> {
  let header_predecessors = ctx.get_blocks().get(&l.header)?.preds.clone();

  let non_loop_predecessors: Vec<Label> = header_predecessors
    .iter()
    .copied()
    .filter(|p| !l.body.contains(p))
    .collect();

  if non_loop_predecessors.is_empty() {
    return None;
  }
  if non_loop_predecessors.len() == 1 {
    let pre = non_loop_predecessors[0];
    let pre_block = ctx.get_blocks().get(&pre).unwrap();
    let pre_successors = get_successors_of_block(pre_block);
    if pre_successors.len() == 1 && pre_successors[0] == l.header {
      return Some(pre);
    }
  }

  let loop_predecessors: Vec<Label> = header_predecessors
    .iter()
    .copied()
    .filter(|p| l.body.contains(p))
    .collect();

  Some(create_preheader(
    ctx,
    l.header,
    &non_loop_predecessors,
    &loop_predecessors,
  ))
}

/// Create a new preheader for a loop and redirect all non-loop edges through it.
/// Merge header phi sources from those edges with a phi in the preheader as required.
fn create_preheader(
  ctx: &mut IRContext,
  header: Label,
  non_loop_predecessors: &[Label],
  loop_predecessors: &[Label],
) -> Label {
  let preheader = ctx.create_block();
  {
    let block = ctx.get_blocks_mut().get_mut(&preheader).unwrap();
    block.terminator = Some(Instr::JumpTo(header));
    block.preds = non_loop_predecessors.to_vec();
    block.sealed = true;
  }

  // update predecessor block terminators to go to loop preheader
  for &pred_label in non_loop_predecessors {
    let pred_block = ctx.get_blocks_mut().get_mut(&pred_label).unwrap();
    match &mut pred_block.terminator {
      Some(Instr::JumpTo(l)) if *l == header => *l = preheader,
      Some(Instr::JumpIf { holds, fails, .. }) => {
        if *holds == header {
          *holds = preheader;
        }
        if *fails == header {
          *fails = preheader;
        }
      }
      _ => {}
    }
  }

  /// (original idx, dest temp, sources from non-loop preds, sources from loop preds).
  type PhiView = (usize, Temp, Vec<(Label, Operand)>, Vec<(Label, Operand)>);

  let phi_views: Vec<PhiView> = ctx
    .get_blocks()
    .get(&header)
    .unwrap()
    .body
    .iter()
    .enumerate()
    .filter_map(|(i, instr)| match instr {
      Instr::Phi { dest, srcs } => {
        let non_loop_sources: Vec<_> = srcs
          .iter()
          .filter(|(l, _)| non_loop_predecessors.contains(l))
          .cloned()
          .collect();
        let loop_sources = srcs
          .iter()
          .filter(|(l, _)| !non_loop_predecessors.contains(l))
          .cloned()
          .collect();
        Some((i, dest.clone(), non_loop_sources, loop_sources))
      }
      _ => None,
    })
    .collect();

  let mut new_preheader_phis: Vec<Instr> = Vec::new();
  let mut header_phi_rewrites: Vec<(usize, Vec<(Label, Operand)>)> = Vec::new();

  for (idx, dest, non_loop_srcs, loop_srcs) in phi_views {
    let merged_op = if non_loop_srcs.len() <= 1 {
      non_loop_srcs
        .first()
        .map(|(_, op)| op.clone())
        .unwrap_or_else(|| Operand::Const((0, dest.1.clone())))
    } else {
      let first_src = non_loop_srcs[0].1.clone();
      if non_loop_srcs.iter().all(|(_, op)| op == &first_src) {
        first_src
      } else {
        let merged_temp = ctx.create_temp(dest.1.clone());
        new_preheader_phis.push(Instr::Phi {
          dest: merged_temp.clone(),
          srcs: non_loop_srcs,
        });
        Operand::Temp(merged_temp)
      }
    };

    let mut new_srcs = vec![(preheader, merged_op)];
    new_srcs.extend(loop_srcs);
    header_phi_rewrites.push((idx, new_srcs));
  }

  let header_block = ctx.get_blocks_mut().get_mut(&header).unwrap();
  for (idx, new_srcs) in header_phi_rewrites {
    if let Some(Instr::Phi { srcs, .. }) = header_block.body.get_mut(idx) {
      *srcs = new_srcs;
    }
  }

  ctx.get_blocks_mut().get_mut(&header).unwrap().preds = iter::once(preheader)
    .chain(loop_predecessors.iter().copied())
    .collect();

  if !new_preheader_phis.is_empty() {
    let pre_block = ctx.get_blocks_mut().get_mut(&preheader).unwrap();
    let mut body = mem::take(&mut pre_block.body);
    let mut prepended = new_preheader_phis;
    prepended.append(&mut body);
    pre_block.body = prepended;
  }

  preheader
}

/// Greedily hoist instructions that are invariant to the loop itself into the preheader.
/// Iterates to convergence, resolving newly invariant def chains.
fn hoist_loop_invariant(
  ctx: &mut IRContext,
  l: &NaturalLoop,
  preheader: Label,
  is_unsafe: bool,
) -> bool {
  let mut hoisted_temps: HashSet<usize> = HashSet::new();
  let mut changed = false;

  loop {
    let mut defs = HashMap::new();
    for block in ctx.get_blocks().values() {
      for (i, instr) in block.body.iter().enumerate() {
        if let Some(dest) = get_dest_temp_from_instruction(instr) {
          defs.insert(dest.0, (block.label, i));
        }
      }
    }

    let mut did_hoist = false;

    for &label in l.body.iter() {
      let body = mem::take(&mut ctx.get_blocks_mut().get_mut(&label).unwrap().body);
      let mut kept = Vec::with_capacity(body.len());
      let mut to_hoist: Vec<Instr> = Vec::new();

      for instr in body {
        if matches!(&instr, Instr::Phi { .. })
          || !is_loop_instr_pure(&instr, is_unsafe)
          || !are_operands_invariant(&instr, &l.body, &defs, &hoisted_temps)
        {
          kept.push(instr);
        } else {
          if let Some(dest) = match &instr {
            Instr::BinOp { dest, .. }
            | Instr::UnOp { dest, .. }
            | Instr::Move { dest, .. }
            | Instr::Load { dest, .. } => Some(dest.clone()),
            _ => None,
          } {
            hoisted_temps.insert(dest.0);
          }

          to_hoist.push(instr);
          did_hoist = true;
          changed = true;
        }
      }

      ctx.get_blocks_mut().get_mut(&label).unwrap().body = kept;

      if !to_hoist.is_empty() {
        let pre = ctx.get_blocks_mut().get_mut(&preheader).unwrap();
        let mut existing = mem::take(&mut pre.body);
        existing.extend(to_hoist);
        pre.body = existing;
      }
    }

    if !did_hoist {
      break;
    }
  }

  changed
}

/// Check whether all operands to an instruction in the loop body are invariant to the loop itself.
fn are_operands_invariant(
  instr: &Instr,
  body: &HashSet<Label>,
  defs: &HashMap<usize, (Label, usize)>,
  hoisted: &HashSet<usize>,
) -> bool {
  for operand in get_operands_from_instruction(instr) {
    if let Operand::Temp((temp_id, _)) = operand
      && !hoisted.contains(temp_id)
    {
      match defs.get(&temp_id) {
        Some((def_block, _)) if body.contains(def_block) => return false,
        _ => {}
      }
    }
  }
  true
}
