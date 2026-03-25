use std::collections::HashMap;

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
  pub current_block_label: Label,
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

  /// Create an unsealed block and return its label.
  pub fn create_block(&mut self) -> Label {
    let label = Label(self.label_counter);
    self.label_counter += 1;

    self.blocks.insert(label, BasicBlock::new(label));

    label
  }

  /// Add an immediate predecessor block label to the current block.
  pub fn add_pred_to_block(&mut self, pred_label: Label) {
    assert!(
      self.blocks.contains_key(&pred_label),
      "Attempted adding unknown predecessor block label {pred_label}."
    );
    self
      .blocks
      .get_mut(&self.current_block_label)
      .unwrap()
      .preds
      .push(pred_label);
  }

  /// Set the terminator instruction to the current block.
  pub fn set_block_terminator(&mut self, terminator: Instr) {
    self
      .blocks
      .get_mut(&self.current_block_label)
      .unwrap()
      .terminator = Some(terminator);
  }

  /// Add an instruction to the current block.
  pub fn add_instr_to_block(&mut self, instr: Instr) {
    self
      .blocks
      .get_mut(&self.current_block_label)
      .unwrap()
      .body
      .push(instr);
  }

  /// Seal the block with label `block_label` and resolve all incomplete phi nodes.
  pub fn seal_block(&mut self, block_label: Label) {
    assert!(
      self.blocks.contains_key(&block_label),
      "Attempted to seal block unknown label {block_label}."
    );

    let incomplete_phis: HashMap<Ident, Temp> = self
      .blocks
      .get_mut(&block_label)
      .unwrap()
      .incomplete_phis
      .drain()
      .collect();

    for (var_id, temp) in incomplete_phis {
      self.resolve_incompete_phi_in_block(var_id, temp, block_label);
    }

    self.blocks.get_mut(&block_label).unwrap().sealed = true;
  }

  /// Switch context from currently active block to block of given label.
  pub fn switch_to_block(&mut self, label: Label) {
    assert!(
      self.blocks.contains_key(&label),
      "Attemped switching to unknown block {label}."
    );
    self.current_block_label = label;
  }

  /// Create and return new temporary of a given type.
  pub fn create_temp(&mut self, typ: Typ) -> Temp {
    let temp: Temp = (self.temp_counter, typ);
    self.temp_counter += 1;
    temp
  }

  /// Record that variable `var_id` maps to `temp` in block with label `block_label`.
  pub fn write_variable(&mut self, var_id: &Ident, temp: Temp, block_label: Label) {
    assert!(
      self.blocks.contains_key(&block_label),
      "Attempted to write variable to unknown block label {block_label}."
    );

    self
      .latest_var_assignments
      .insert(var_id.clone(), temp.1.clone());

    self
      .blocks
      .get_mut(&block_label)
      .unwrap()
      .current_def
      .insert(var_id.to_string(), temp);
  }

  /// Get the temp assigned to a variable with block with label `block_label`.
  pub fn read_variable(&mut self, var_id: &Ident, block_label: Label) -> Temp {
    assert!(
      self.blocks.contains_key(&block_label),
      "Attempted to read variable from unknown block label {block_label}."
    );

    // variable already within block
    if let Some(temp) = self
      .blocks
      .get(&block_label)
      .and_then(|block| block.current_def.get(var_id))
    {
      return temp.clone();
    }

    // variable not in unsealed block, so creating an incomplete phi to bring it in
    if !self.blocks.get(&block_label).unwrap().sealed {
      let typ = self.infer_typ(var_id, block_label);
      let phi_temp = self.create_temp(typ);

      self
        .blocks
        .get_mut(&block_label)
        .unwrap()
        .incomplete_phis
        .insert(var_id.clone(), phi_temp.clone());
      return phi_temp;
    }

    // variable not in sealed block, so traversing all preds and creating phi node
    let preds = self.blocks.get(&block_label).unwrap().preds.clone();

    let temp = if preds.len() == 1 {
      self.read_variable(var_id, preds[0])
    } else {
      let typ = self.infer_typ(var_id, block_label);
      let phi_temp = self.create_temp(typ);

      self.write_variable(var_id, phi_temp.clone(), block_label);
      self.resolve_incompete_phi_in_block(var_id.clone(), phi_temp, block_label)
    };

    self.write_variable(var_id, temp.clone(), block_label);
    temp
  }

  /// Add operand to incomplete phi in block with label `block_label`.
  fn resolve_incompete_phi_in_block(
    &mut self,
    var_id: Ident,
    phi_temp: Temp,
    block_label: Label,
  ) -> Temp {
    assert!(
      self.blocks.contains_key(&block_label),
      "Attempted to resolve incomplete phi in unknown block label {block_label}."
    );

    let pred_block_labels = self.blocks.get(&block_label).unwrap().preds.clone();

    // get uses of var in predecessor blocks
    let srcs: Vec<(Label, Operand)> = pred_block_labels
      .iter()
      .map(|pred_block_label| {
        (
          *pred_block_label,
          Operand::Temp(self.read_variable(&var_id, *pred_block_label)),
        )
      })
      .collect();

    let block_body = &mut self.blocks.get_mut(&block_label).unwrap().body;

    if let Some(Instr::Phi { srcs: phi_srcs, .. }) = block_body
      .iter_mut()
      .find(|i| matches!(i, Instr::Phi { dest, .. } if dest.0 == phi_temp.0))
    {
      // resolve the incomplete phi with the predecessor operands
      *phi_srcs = srcs;
    } else {
      // phi not yet constructed, so inserting at the top of the block
      block_body.insert(
        0,
        Instr::Phi {
          dest: phi_temp.clone(),
          srcs,
        },
      );
    }

    phi_temp
  }

  /// Infer the type of a variable within block with label `block_label`.
  fn infer_typ(&self, var_id: &Ident, block_label: Label) -> Typ {
    assert!(
      self.blocks.contains_key(&block_label),
      "Attempted to infer type of variable {var_id} within unknown block label {block_label}."
    );

    if let Some(temp) = self
      .blocks
      .get(&block_label)
      .and_then(|block| block.current_def.get(var_id))
    {
      return temp.1.clone();
    }

    if let Some(typ) = self.latest_var_assignments.get(var_id) {
      return typ.clone();
    }

    // should never reach here if the semantic analysis and typechecker pass.
    unreachable!("Failed to ingfer type of variable {var_id} within block label {block_label}.");
  }
}
