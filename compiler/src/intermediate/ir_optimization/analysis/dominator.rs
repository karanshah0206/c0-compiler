use std::collections::HashMap;

use crate::intermediate::{
  ir_asm::Label, ir_context::IRContext, ir_optimization::analysis::cfg_helpers::*,
};

/// Dominator tree for a function's control-flow graph.
pub struct DominatorTree {
  /// Immediate dominator mappings.
  pub dominator: HashMap<Label, Label>,
  /// Children of basic blocks.
  pub children: HashMap<Label, Vec<Label>>,
  /// Blocks that are reachable from the entry block in reverse post-order.
  pub reachable: Vec<Label>,
}

impl DominatorTree {
  /// Compute and return the dominator tree from a function's IR.
  pub fn new(ctx: &IRContext) -> Self {
    let rpo = get_block_labels_in_rpo(ctx);
    let rpo_index: HashMap<Label, usize> = rpo
      .iter()
      .enumerate()
      .map(|(index, label)| (*label, index))
      .collect();
    let label_preds_map = get_predecessors_for_all_blocks(ctx);

    let mut dominators: HashMap<Label, Option<Label>> =
      rpo.iter().map(|label| (*label, None)).collect();
    if let Some(&entry_label) = rpo.first() {
      dominators.insert(entry_label, Some(entry_label));
    }

    let mut changed = true;
    while changed {
      changed = false;

      for &label in rpo.iter().skip(1) {
        let mut new_dominator: Option<Label> = None;
        for &pred in label_preds_map
          .get(&label)
          .map(|predecessors| predecessors.as_slice())
          .unwrap_or(&[])
        {
          if !rpo_index.contains_key(&pred) || dominators.get(&pred).copied().flatten().is_none() {
            continue;
          }
          new_dominator = Some(match new_dominator {
            Some(existing) => get_nearest_common_dominator(existing, pred, &dominators, &rpo_index),
            None => pred,
          });
        }

        if let Some(dominator) = new_dominator
          && dominators.get(&label).copied().flatten() != Some(dominator)
        {
          dominators.insert(label, Some(dominator));
          changed = true;
        }
      }
    }

    let dominators: HashMap<Label, Label> = dominators
      .into_iter()
      .filter_map(|(dom, sub)| sub.map(|some_sub| (dom, some_sub)))
      .collect();

    let mut children: HashMap<Label, Vec<Label>> = dominators
      .keys()
      .map(|label| (*label, Vec::new()))
      .collect();

    for (&block_label, &dominator) in &dominators {
      if block_label != dominator {
        children.entry(dominator).or_default().push(block_label);
      }
    }

    DominatorTree {
      dominator: dominators,
      children,
      reachable: rpo,
    }
  }

  /// Check if block labelled `a` dominates block labelled `b`.
  pub fn does_dominate(&self, a: Label, b: Label) -> bool {
    let mut current = b;
    loop {
      if current == a {
        return true;
      }
      let parent = match self.dominator.get(&current) {
        Some(p) => *p,
        None => return false,
      };
      if parent == current {
        return current == a;
      }
      current = parent;
    }
  }
}

/// Get the nearest common dominator block of two blocks.
fn get_nearest_common_dominator(
  mut a: Label,
  mut b: Label,
  dominators: &HashMap<Label, Option<Label>>,
  rpo_index: &HashMap<Label, usize>,
) -> Label {
  while a != b {
    while rpo_index[&a] > rpo_index[&b] {
      a = dominators[&a].unwrap();
    }
    while rpo_index[&b] > rpo_index[&a] {
      b = dominators[&b].unwrap();
    }
  }
  a
}
