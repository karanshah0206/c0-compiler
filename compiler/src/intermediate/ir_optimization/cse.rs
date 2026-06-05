use std::collections::HashMap;

use crate::front::ast::{BinOp, Typ, UnOp};
use crate::intermediate::{
  ir_asm::{Instr, Label, Operand, Temp},
  ir_context::IRContext,
  ir_optimization::analysis::{alias::*, dominator::DominatorTree},
};

/// Value-number key for an SSA-defining instruction.
#[derive(Hash, PartialEq, Eq)]
enum ExprKey {
  /// Move from a source operand.
  Move(KeyOp),
  /// Unary operation.
  UnOp(UnOp, KeyOp),
  /// Binary operation.
  BinOp(BinOp, KeyOp, KeyOp),
}

/// Hashable/orderable encoding of an operand for value numbering.
#[derive(Hash, PartialEq, Eq, PartialOrd, Ord)]
enum KeyOp {
  /// Constant value (integer payload paied with a type tag).
  Const(i64, TypeTag),
  /// SSA temporary identified by an id.
  Temp(usize),
}

/// Compact representation of `Typ` suitable for hashing and sorting.
#[derive(Hash, PartialEq, Eq, PartialOrd, Ord)]
enum TypeTag {
  Int,
  Bool,
  Other,
}

impl From<&Typ> for TypeTag {
  fn from(t: &Typ) -> Self {
    match t {
      Typ::Int => TypeTag::Int,
      Typ::Bool => TypeTag::Bool,
      _ => TypeTag::Other,
    }
  }
}

/// Common subexpression elimination via dominator-tree value numbering.
pub fn cse(ctx: &mut IRContext) -> bool {
  let dominator_tree = DominatorTree::new(ctx);
  if dominator_tree.reachable.is_empty() {
    return false;
  }

  let alias_analysis = AliasInfo::new(ctx);
  let mut scopes: Vec<HashMap<ExprKey, Temp>> = Vec::new();
  let mut substitutions: HashMap<usize, Temp> = HashMap::new();
  let mut changed = false;

  traverse_dom_tree(
    ctx,
    dominator_tree.reachable[0],
    &dominator_tree,
    &alias_analysis,
    &mut scopes,
    &mut substitutions,
    &mut changed,
  );

  changed
}

/// Visit a block in dominator-tree order, performing value-numbering on its instructions.
fn traverse_dom_tree(
  ctx: &mut IRContext,
  block_label: Label,
  dominator_tree: &DominatorTree,
  alias_analysis: &AliasInfo,
  scopes: &mut Vec<HashMap<ExprKey, Temp>>,
  substitutions: &mut HashMap<usize, Temp>,
  changed: &mut bool,
) {
  scopes.push(HashMap::new());

  let body = std::mem::take(&mut ctx.get_blocks_mut().get_mut(&block_label).unwrap().body);
  let mut new_body = Vec::with_capacity(body.len());
  let mut load_table: HashMap<PointerTarget, Temp> = HashMap::new();

  for instr in body {
    let mut instr = rewrite_operands(instr, substitutions);

    match &instr {
      Instr::Load { dest, addr } => {
        if let Some(key) = match alias_analysis.target_of_operand(addr) {
          Some(pointer_target) => Some(pointer_target),
          None => match addr {
            Operand::Temp((id, _)) => Some(PointerTarget::Relative {
              base: *id,
              offset: 0,
            }),
            Operand::Const(_) => None,
          },
        } {
          if let Some(prev) = load_table.get(&key) {
            let canonical = resolve_canonical(prev.clone(), substitutions);
            substitutions.insert(dest.0, canonical.clone());
            *changed = true;
            new_body.push(Instr::Move {
              dest: dest.clone(),
              src: Operand::Temp(canonical),
            });
          } else {
            load_table.insert(key, dest.clone());
            new_body.push(instr);
          }
        } else {
          new_body.push(instr);
        }
        continue;
      }
      Instr::Store { addr, .. } => {
        invalidate_for_store(&mut load_table, addr, alias_analysis);
        new_body.push(instr);
        continue;
      }
      Instr::Call { .. } | Instr::TailCall { .. } => {
        invalidate_for_call(&mut load_table, alias_analysis);
        new_body.push(instr);
        continue;
      }
      _ => {}
    }

    if let Some(key) = expression_key(&instr)
      && let Some(dest) = match &instr {
        Instr::BinOp { dest, .. }
        | Instr::UnOp { dest, .. }
        | Instr::Move { dest, .. }
        | Instr::Load { dest, .. } => Some(dest.clone()),
        _ => None,
      }
    {
      if let Some(existing) = scope_lookup(scopes, &key) {
        let canonical = resolve_canonical(existing, substitutions);
        substitutions.insert(dest.0, canonical.clone());
        *changed = true;
        instr = Instr::Move {
          dest,
          src: Operand::Temp(canonical),
        };
      } else {
        scopes.last_mut().unwrap().insert(key, dest);
      }
    }

    new_body.push(instr);
  }

  if let Some(terminator) = ctx
    .get_blocks_mut()
    .get_mut(&block_label)
    .unwrap()
    .terminator
    .take()
  {
    let new_terminator = rewrite_operands(terminator, substitutions);
    ctx
      .get_blocks_mut()
      .get_mut(&block_label)
      .unwrap()
      .terminator = Some(new_terminator);
  }

  ctx.get_blocks_mut().get_mut(&block_label).unwrap().body = new_body;

  let children = dominator_tree
    .children
    .get(&block_label)
    .cloned()
    .unwrap_or_default();
  for child in children {
    traverse_dom_tree(
      ctx,
      child,
      dominator_tree,
      alias_analysis,
      scopes,
      substitutions,
      changed,
    );
  }

  scopes.pop();
}

/// Follow the substitution chain to its canonical temp.
fn resolve_canonical(mut temp: Temp, subst: &HashMap<usize, Temp>) -> Temp {
  while let Some(next) = subst.get(&temp.0) {
    if next.0 == temp.0 {
      break;
    }
    temp = next.clone();
  }
  temp
}

/// Invalidate cached loads that may be clobbered by a store through `addr`.
fn invalidate_for_store(
  load_table: &mut HashMap<PointerTarget, Temp>,
  addr: &Operand,
  alias: &AliasInfo,
) {
  let store_target = alias.target_of_operand(addr).or(match addr {
    Operand::Temp((id, _)) => Some(PointerTarget::Relative {
      base: *id,
      offset: 0,
    }),
    Operand::Const(_) => None,
  });

  match store_target {
    Some(PointerTarget::Class(store_class)) => {
      load_table.retain(|key, _| match key {
        PointerTarget::Class(load_class) => *load_class != store_class,
        // a class store reaches a relative load only if the alloc has escaped
        PointerTarget::Relative { .. } => !alias.escapes(store_class.alloc_id),
      });
    }
    Some(PointerTarget::Relative {
      base: store_base,
      offset: store_offset,
    }) => {
      load_table.retain(|key, _| match key {
        PointerTarget::Class(load_class) => !alias.escapes(load_class.alloc_id),
        PointerTarget::Relative {
          base: load_base,
          offset: load_offset,
        } => {
          if *load_base == store_base {
            *load_offset != store_offset
          } else {
            false
          }
        }
      });
    }
    None => {
      // store through a constant address: very rare, conservatively flush
      load_table.clear();
    }
  }
}

/// Invalidate cached loads that may be clobbered by a call. Non-escaping loads are preserved.
fn invalidate_for_call(load_table: &mut HashMap<PointerTarget, Temp>, alias: &AliasInfo) {
  load_table.retain(|key, _| match key {
    PointerTarget::Class(class) => !alias.escapes(class.alloc_id),
    PointerTarget::Relative { .. } => false,
  });
}

/// Build a value-number key for a instruction.
fn expression_key(instr: &Instr) -> Option<ExprKey> {
  let operand_key = |op: &Operand| -> KeyOp {
    match op {
      Operand::Const((value, typ)) => KeyOp::Const(*value, TypeTag::from(typ)),
      Operand::Temp((id, _)) => KeyOp::Temp(*id),
    }
  };

  match instr {
    Instr::Move { src, .. } => match src {
      Operand::Const(_) => None,
      Operand::Temp(_) => Some(ExprKey::Move(operand_key(src))),
    },
    Instr::UnOp { op, src, .. } => Some(ExprKey::UnOp(*op, operand_key(src))),
    Instr::BinOp { op, lhs, rhs, .. } => {
      let mut lhs_key = operand_key(lhs);
      let mut rhs_key = operand_key(rhs);
      if matches!(
        *op,
        BinOp::Add
          | BinOp::Mul
          | BinOp::And
          | BinOp::Or
          | BinOp::Xor
          | BinOp::CmpEq
          | BinOp::CmpNeq
          | BinOp::LAnd
          | BinOp::LOr
      ) && rhs_key < lhs_key
      {
        std::mem::swap(&mut lhs_key, &mut rhs_key);
      }
      Some(ExprKey::BinOp(*op, lhs_key, rhs_key))
    }
    _ => None,
  }
}

/// Replace each operand with its canonical temp following substitution chains.
fn rewrite_operands(mut instr: Instr, subst: &HashMap<usize, Temp>) -> Instr {
  let resolve = |op: &mut Operand| {
    if let Operand::Temp(temp) = op {
      let canonical = resolve_canonical(temp.clone(), subst);
      if canonical.0 != temp.0 {
        *op = Operand::Temp(canonical);
      }
    }
  };
  match &mut instr {
    Instr::BinOp { lhs, rhs, .. } => {
      resolve(lhs);
      resolve(rhs);
    }
    Instr::UnOp { src, .. } => resolve(src),
    Instr::JumpIf { pred, .. } => resolve(pred),
    Instr::Call { args, .. } | Instr::TailCall { args, .. } => {
      for arg in args.iter_mut() {
        resolve(arg);
      }
    }
    Instr::Return(Some(op)) => resolve(op),
    Instr::Phi { srcs, .. } => {
      for (_, op) in srcs.iter_mut() {
        resolve(op);
      }
    }
    Instr::Move { src, .. } => resolve(src),
    Instr::Load { addr, .. } => resolve(addr),
    Instr::Store { addr, src } => {
      resolve(addr);
      resolve(src);
    }
    Instr::Alloc { size, .. } => resolve(size),
    Instr::AllocArray { size, count, .. } => {
      resolve(size);
      resolve(count);
    }
    Instr::Label(_) | Instr::JumpTo(_) | Instr::Return(None) | Instr::Throw(_) => {}
  }
  instr
}

/// Walk the scope chain for an existing temp matching key.
fn scope_lookup(scopes: &[HashMap<ExprKey, Temp>], key: &ExprKey) -> Option<Temp> {
  for scope in scopes.iter().rev() {
    if let Some(temp) = scope.get(key) {
      return Some(temp.clone());
    }
  }
  None
}
