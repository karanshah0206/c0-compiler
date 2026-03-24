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
