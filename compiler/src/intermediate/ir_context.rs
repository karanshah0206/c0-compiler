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
  pub current_block_label: Label,
  /// "Trivial" Phi nodes to replace with their single operand.
  /// A Phi node is trivial if it just references itself or a single other value.
  trivial_phis: HashMap<usize, Temp>,
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

  /// Get the number of temporaries generated within the context.
  pub fn get_temps_count(&self) -> usize {
    self.temp_counter
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
      "Attempted to seal block with unknown label {block_label}."
    );

    let incomplete_phis: HashMap<Ident, Temp> = self
      .blocks
      .get(&block_label)
      .unwrap()
      .incomplete_phis
      .iter()
      .map(|(v, t)| (v.clone(), t.clone()))
      .collect();

    for (var_id, phi_temp) in incomplete_phis {
      self.resolve_incompete_phi_in_block(var_id, phi_temp, block_label);
    }

    self
      .blocks
      .get_mut(&block_label)
      .unwrap()
      .incomplete_phis
      .clear();

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
      return self.resolve_phi_temp(temp.clone());
    }

    // variable in unsealed block, so creating an incomplete phi to bring it in
    if !self.blocks.get(&block_label).unwrap().sealed {
      let typ = self.infer_typ(var_id, block_label);
      let phi_temp = self.create_temp(typ);

      self
        .blocks
        .get_mut(&block_label)
        .unwrap()
        .incomplete_phis
        .insert(var_id.clone(), phi_temp.clone());

      self.write_variable(var_id, phi_temp.clone(), block_label);
      return phi_temp;
    }

    // variable in sealed block, so traversing all preds and creating Phi node
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

  /// Apply all remaining trivial Phi replacements globally within the context.
  pub fn finalize_trivial_phis(&mut self) {
    if self.trivial_phis.is_empty() {
      return;
    }

    let trivial_phis = self.trivial_phis.clone();

    let resolve_temp = |mut temp: Temp| {
      loop {
        match trivial_phis.get(&temp.0) {
          Some(next_in_chain) => temp = next_in_chain.clone(),
          None => return temp,
        }
      }
    };

    let resolve_op = |op: &mut Operand| {
      if let Operand::Temp(temp) = op {
        *temp = resolve_temp(temp.clone());
      }
    };

    let resolve_instr = |instr: &mut Instr| match instr {
      Instr::Label(_) | Instr::JumpTo(_) | Instr::Throw(_) => {}
      Instr::BinOp { lhs, rhs, .. } => {
        resolve_op(lhs);
        resolve_op(rhs);
      }
      Instr::UnOp { src, .. } => resolve_op(src),
      Instr::Call { args, .. } => {
        for arg in args.iter_mut() {
          resolve_op(arg);
        }
      }
      Instr::Return(operand) => {
        if let Some(operand) = operand.as_mut() {
          resolve_op(operand);
        }
      }
      Instr::JumpIf { pred, .. } => resolve_op(pred),
      Instr::Phi { srcs, .. } => {
        for (_, operand) in srcs.iter_mut() {
          resolve_op(operand);
        }
      }
      Instr::Move { src, .. } => resolve_op(src),
    };

    for block in self.blocks.values_mut() {
      for val in block.current_def.values_mut() {
        *val = resolve_temp(val.clone());
      }

      block.body.retain(|instr| match instr {
        Instr::Phi { dest, .. } => !trivial_phis.contains_key(&dest.0),
        _ => true,
      });

      for instr in block.body.iter_mut() {
        resolve_instr(instr);
      }

      if let Some(terminator) = block.terminator.as_mut() {
        resolve_instr(terminator);
      }
    }
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
      .find(|instr| matches!(instr, Instr::Phi { dest, .. } if dest.0 == phi_temp.0))
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

    self.try_remove_trivial_phi(phi_temp)
  }

  /// Try removing phi node if trivial (i.e., self-referential or only one src) and return resolved temp.
  fn try_remove_trivial_phi(&mut self, phi_temp: Temp) -> Temp {
    let (phi_block_label, srcs): (Label, Vec<Temp>) = {
      let mut found: Option<(Label, Vec<(Label, Operand)>)> = None;

      for (block_label, block) in self.blocks.iter() {
        if let Some(Instr::Phi { srcs, .. }) = block
          .body
          .iter()
          .find(|instr| matches!(instr, Instr::Phi { dest, .. } if dest.0 == phi_temp.0))
        {
          found = Some((*block_label, srcs.clone()));
          break;
        }
      }

      match found {
        Some((block_label, srcs)) => {
          let filtered = srcs
            .iter()
            .filter_map(|(_, op)| {
              if let Operand::Temp(temp) = op {
                if temp.0 != phi_temp.0 {
                  Some(temp.clone())
                } else {
                  None
                }
              } else {
                None
              }
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
          (block_label, filtered)
        }
        None => return phi_temp,
      }
    };

    let resolved: Vec<Temp> = srcs
      .iter()
      .map(|temp| self.resolve_phi_temp(temp.clone()))
      .collect::<HashSet<_>>()
      .into_iter()
      .collect();

    if resolved.len() != 1 {
      return phi_temp;
    }

    let resolved_identical_temp = resolved.into_iter().next().unwrap();
    if let Some(block) = self.blocks.get_mut(&phi_block_label) {
      block
        .body
        .retain(|instr| !matches!(instr, Instr::Phi { dest, .. } if dest.0 == phi_temp.0));
    }

    self
      .trivial_phis
      .insert(phi_temp.0, resolved_identical_temp.clone());

    resolved_identical_temp
  }

  /// Resolve phi temp from its replacement chain (if any).
  fn resolve_phi_temp(&mut self, mut phi_temp: Temp) -> Temp {
    loop {
      match self.trivial_phis.get(&phi_temp.0) {
        Some(next_in_chain) => phi_temp = next_in_chain.clone(),
        None => return phi_temp,
      }
    }
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

  /// Transform CFG into linearized IR in reverse post-order from entry block.
  pub fn linearize(&self) -> Vec<Instr> {
    let mut linear_ir = Vec::new();
    for label in self.get_reachable_in_rpo() {
      let block = self.blocks.get(&label).unwrap();
      linear_ir.push(Instr::Label(block.label));
      linear_ir.extend(block.body.clone());
      if let Some(terminator) = &block.terminator {
        linear_ir.push(terminator.clone());
      }
    }
    linear_ir
  }

  /// Get labels of reachable blocks in reverse post-order entry block.
  fn get_reachable_in_rpo(&self) -> Vec<Label> {
    let mut visited: HashSet<Label> = HashSet::new();
    let mut order: Vec<Label> = Vec::new();
    self.reachable_generator_dfs(Label(0), &mut visited, &mut order);
    order.reverse();
    order
  }

  /// Helper to generate successor blocks in order when linearlizing IR.
  fn reachable_generator_dfs(
    &self,
    label: Label,
    visited: &mut HashSet<Label>,
    order: &mut Vec<Label>,
  ) {
    if visited.insert(label)
      && let Some(block) = self.blocks.get(&label)
    {
      for successor in self.get_successors_of_block(block) {
        self.reachable_generator_dfs(successor, visited, order);
      }
      order.push(block.label);
    }
  }

  /// Get labels of direct successors of given block.
  fn get_successors_of_block(&self, block: &BasicBlock) -> Vec<Label> {
    match &block.terminator {
      Some(Instr::JumpTo(label)) => vec![*label],
      Some(Instr::JumpIf { holds, fails, .. }) => {
        let mut successors = vec![*holds];
        successors.push(*fails);
        successors
      }
      _ => vec![],
    }
  }
}
