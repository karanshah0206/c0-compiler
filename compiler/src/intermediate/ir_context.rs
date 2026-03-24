use std::collections::{HashMap, HashSet};

use crate::front::ast::{Ident, Typ};
use crate::intermediate::ir_asm::{Instr, Label, Operand, Temp};

/// A single basic block in the CFG.
pub struct BasicBlock {
  /// The label that marks the entry of this block.
  pub label: Label,
  /// The instructions within this block.
  pub body: Vec<Instr>,
  /// The instruction that terminates this block.
  pub terminator: Option<Instr>,
  /// The labels that are direct predecessors of this block in the CFG.
  pub preds: Vec<Label>,
  /// Is this blocked sealed (i.e., are all its predecessors known yet)?
  pub sealed: bool,
  /// The current SSA renamings for variables in this block.
  pub current_def: HashMap<Ident, Temp>,
  /// Phi nodes that are still awaiting information from predecessor blocks.
  pub incomplete_phis: HashMap<Ident, Temp>,
}

impl BasicBlock {
  /// Generate an empty, unsealed basic block.
  fn new(label: Label) -> Self {
    BasicBlock {
      label,
      body: Vec::new(),
      terminator: None,
      preds: Vec::new(),
      sealed: false,
      current_def: HashMap::new(),
      incomplete_phis: HashMap::new(),
    }
  }
}

/// Context for generating intermediate representation in SSA form (Braun et al. technique).
pub struct IRContext {
  /// Number of generated temps within context (i.e., id for next temp)
  temp_counter: usize,
  /// Number of generated labels within context (i.e., id for next label)
  label_counter: usize,
  /// Basic blocks within this context.
  blocks: HashMap<Label, BasicBlock>,
  /// Lable of the basic block currently being evaluated.
  current_block_label: Label,
  /// "Trivial" Phi nodes that were replaced by their single operand.
  /// A Phi node is trivial if it just references itself and one other value.
  trivial_phis: HashMap<Label, Temp>,
  /// Most recently assigned type in the currently evaluated block for a given variable.
  latest_var_assignments: HashMap<Ident, Typ>,
}

impl IRContext {
  /// Create an IR context with a (sealed, empty) entry block.
  pub fn new() -> Self {
    let entry_label = Label(0);
    let mut entry_block = BasicBlock::new(entry_label);
    entry_block.sealed = true; // no predecessors to the entry block.

    IRContext {
      temp_counter: 0,
      label_counter: 1,
      blocks: HashMap::from([(entry_label, entry_block)]),
      current_block_label: entry_label,
      trivial_phis: HashMap::new(),
      latest_var_assignments: HashMap::new(),
    }
  }
}
