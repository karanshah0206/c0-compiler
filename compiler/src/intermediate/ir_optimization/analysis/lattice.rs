use std::collections::HashMap;

use crate::front::ast::Typ;
use crate::intermediate::ir_asm::Operand;

/// Lattice is the set of known states of temporaries.
pub type Lattice = HashMap<usize, LatticeValue>;

/// Possible states for a value in data-flow analysis.
#[derive(Clone, PartialEq)]
pub enum LatticeValue {
  /// Value is undetermined.
  Undefined,
  /// Value is a known constant.
  Const((i64, Typ)),
  /// Value could be one of many.
  Overdefined,
}

/// Update the lattice through discovery of a new lattice value.
/// Returns `true` if the known state changes.
pub fn update_lattice(lattice: &mut Lattice, temp_id: usize, new: LatticeValue) -> bool {
  let old = lattice
    .get(&temp_id)
    .cloned()
    .unwrap_or(LatticeValue::Undefined);

  let merged = join_lattice(old.clone(), new);

  if merged != old {
    lattice.insert(temp_id, merged);
    true
  } else {
    false
  }
}

/// Combine two lattice values from control-flow paths and return resultant lattice value.
pub fn join_lattice(lhs: LatticeValue, rhs: LatticeValue) -> LatticeValue {
  match (lhs, rhs) {
    (LatticeValue::Undefined, value) | (value, LatticeValue::Undefined) => value,
    (LatticeValue::Const(l), LatticeValue::Const(r)) if l == r => LatticeValue::Const(l),
    _ => LatticeValue::Overdefined,
  }
}

/// Get the known lattice state of an operand.
pub fn get_lattice_value_of_operand(op: &Operand, lattice: &Lattice) -> LatticeValue {
  match op {
    Operand::Const(c) => LatticeValue::Const(c.clone()),
    Operand::Temp((id, _)) => lattice.get(id).cloned().unwrap_or(LatticeValue::Undefined),
  }
}
