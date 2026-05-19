use std::{
  cmp::min,
  collections::{HashMap, HashSet},
};

use crate::front::ast::{BinOp, Typ};
use crate::intermediate::{
  ir_asm::{Instr, Label, Operand},
  ir_context::{BasicBlock, IRContext},
};
use crate::x86_back::{
  x86_asm::{Width::*, X86Reg, X86WReg},
  x86_regalloc::liveness_analysis::*,
};

/// A node in the interference graph.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Node {
  /// Pre-colored temporary goes directly to register.
  Reg(X86Reg),
  /// Uncolored temporary identified via its numeric id.
  Temp(usize),
}

/// A move edge candidate for coalescing.
pub struct MoveEdge {
  pub src: Node,
  pub dest: Node,
}

/// Per-temporary type information.
pub type TempTypes = HashMap<usize, Typ>;

/// Interference graph generated using analysis.
/// Also stores heuristics for better coalesing and spilling.
pub struct InterferenceGraph {
  /// Mapping from each node to its interfering nodes.
  pub adj: HashMap<Node, HashSet<Node>>,
  /// All interfering nodes in a tuple-set for fast lookup.
  pub adj_set: HashSet<(Node, Node)>,
  /// Number of interfering nodes for each node.
  pub degree: HashMap<Node, usize>,
  /// Edges generated due to move instructions.
  pub moves: Vec<MoveEdge>,
  /// List of move ids that a node participates in.
  pub move_list: HashMap<Node, HashSet<usize>>,
  /// Type of each temporary.
  pub temp_types: TempTypes,
  /// Spill cost heuristic per temporary.
  pub spill_weight: HashMap<usize, f64>,
}

impl InterferenceGraph {
  /// Generate a fresh, empty interference graph.
  fn new() -> Self {
    InterferenceGraph {
      adj: HashMap::new(),
      adj_set: HashSet::new(),
      degree: HashMap::new(),
      moves: Vec::new(),
      move_list: HashMap::new(),
      temp_types: HashMap::new(),
      spill_weight: HashMap::new(),
    }
  }

  /// Check if two nodes interference.
  pub fn does_interfere(&self, u: Node, v: Node) -> bool {
    self.adj_set.contains(&(u, v))
  }

  /// Add a new node (if it doesn't already exist) to the interference graph.
  fn add_node(&mut self, node: Node) {
    self.adj.entry(node).or_default();
    self.degree.entry(node).or_insert(0);
    self.move_list.entry(node).or_default();
  }

  /// Add an edge between two nodes (if it doesn't already exist), avoiding self-loops.
  fn add_edge(&mut self, u: Node, v: Node) {
    if u == v || self.adj_set.contains(&(u, v)) {
      return;
    }

    self.adj_set.insert((u, v));
    self.adj_set.insert((v, u));
    self.adj.entry(u).or_default();
    self.adj.entry(v).or_default();
    *self.degree.entry(u).or_insert(0) += 1;
    *self.degree.entry(v).or_insert(0) += 1;
  }
}

/// Build interference graph using function's IR and liveness analysis.
pub fn build(ctx: &IRContext, liveness: &Liveness, params_count: usize) -> InterferenceGraph {
  let mut graph = InterferenceGraph::new();

  for reg in X86Reg::allocatable() {
    graph.add_node(Node::Reg(reg));
  }
  for id in 0..params_count {
    graph.add_node(Node::Temp(id));
  }

  let blocks = ctx.get_blocks();
  let mut block_labels: Vec<Label> = blocks.keys().copied().collect();
  block_labels.sort_by_key(|label| label.0);

  for &label in &block_labels {
    let block = blocks.get(&label).unwrap();
    for instr in &block.body {
      get_temp_types(instr, &mut graph.temp_types);
    }
    if let Some(terminator) = &block.terminator {
      get_temp_types(terminator, &mut graph.temp_types);
    }
  }

  let block_weights = approximate_block_weights(ctx);

  for label in block_labels {
    let block = blocks.get(&label).unwrap();
    let mut live = liveness.live_out.get(&label).cloned().unwrap_or_default();

    let weight = *block_weights.get(&label).unwrap_or(&1.);

    if let Some(terminator) = &block.terminator {
      process_instruction(terminator, &mut live, &mut graph, weight);
    }
    for instruction in block.body.iter().rev() {
      process_instruction(instruction, &mut live, &mut graph, weight);
    }
  }

  let arg_regs = X86Reg::call_argument();
  for i in 0..params_count {
    for j in (i + 1)..params_count {
      graph.add_edge(Node::Temp(i), Node::Temp(j));
    }
    if i < arg_regs.len() {
      for j in (i + 1)..arg_regs.len() {
        graph.add_edge(Node::Temp(i), Node::Reg(arg_regs[j]));
      }
    }
  }

  graph
}

/// Add interference edges from definitions to current live set.
fn process_instruction(
  instr: &Instr,
  live: &mut HashSet<usize>,
  graph: &mut InterferenceGraph,
  weight: f64,
) {
  let def_temp_id = get_defines(instr);
  let move_src = if let Instr::Move {
    src: Operand::Temp((id, _)),
    ..
  } = instr
  {
    Some(*id)
  } else {
    None
  };

  let clobbers = get_clobbered_regs(instr);
  if !clobbers.is_empty() {
    let mut clobber_temps: HashSet<usize> = live.iter().copied().collect();
    for temp_id in get_uses(instr) {
      clobber_temps.insert(temp_id);
    }
    for &temp_id in &clobber_temps {
      if Some(temp_id) == def_temp_id {
        continue;
      }
      for &reg in &clobbers {
        graph.add_edge(Node::Temp(temp_id), Node::Reg(reg));
      }
    }
  }

  if let (Some(def), Some(src)) = (def_temp_id, move_src) {
    let move_id = graph.moves.len();

    graph.moves.push(MoveEdge {
      src: Node::Temp(src),
      dest: Node::Temp(def),
    });

    graph.add_node(Node::Temp(src));
    graph.add_node(Node::Temp(def));

    graph
      .move_list
      .entry(Node::Temp(def))
      .or_default()
      .insert(move_id);
    graph
      .move_list
      .entry(Node::Temp(src))
      .or_default()
      .insert(move_id);
  }

  if let Some(def) = def_temp_id {
    graph.add_node(Node::Temp(def));
    *graph.spill_weight.entry(def).or_insert(0.) += weight;
    for &temp_id in live.iter() {
      if temp_id == def {
        continue;
      }
      if Some(temp_id) == move_src {
        continue;
      }
      graph.add_edge(Node::Temp(temp_id), Node::Temp(def));
    }
  }

  if let Instr::BinOp {
    op: BinOp::Sub,
    rhs: Operand::Temp((rhs_id, _)),
    ..
  } = instr
  {
    if let Some(def) = def_temp_id
      && def != *rhs_id
    {
      graph.add_edge(Node::Temp(*rhs_id), Node::Temp(def));
    }
  }

  if let Some(def) = def_temp_id {
    live.remove(&def);
  }

  for temp_id in get_uses(instr) {
    *graph.spill_weight.entry(temp_id).or_insert(0.) += weight;
    graph.add_node(Node::Temp(temp_id));
    live.insert(temp_id);
  }
}

/// Get registers clobbered by an instruction.
fn get_clobbered_regs(instr: &Instr) -> Vec<X86Reg> {
  match instr {
    Instr::Call { .. }
    | Instr::TailCall { .. }
    | Instr::Alloc { .. }
    | Instr::AllocArray { .. } => X86Reg::caller_saved()
      .into_iter()
      .filter(|r| X86Reg::allocatable().contains(r))
      .collect(),
    Instr::BinOp { op: BinOp::Div, .. } | Instr::BinOp { op: BinOp::Mod, .. } => vec![
      X86WReg::quotient(W64).register,
      X86WReg::modulo(W64).register,
    ],
    Instr::BinOp {
      op: BinOp::Sal,
      rhs,
      ..
    }
    | Instr::BinOp {
      op: BinOp::Sar,
      rhs,
      ..
    } if !matches!(rhs, Operand::Const(_)) => vec![X86WReg::shift().register],
    _ => vec![],
  }
}

/// Determine the type of all temporaries referenced in an instruction.
fn get_temp_types(instr: &Instr, temp_types: &mut TempTypes) {
  let walk_op = |op: &Operand, types: &mut TempTypes| {
    if let Operand::Temp((id, t)) = op {
      types.entry(*id).or_insert_with(|| t.clone());
    }
  };
  let walk_dest = |dest: &(usize, Typ), types: &mut TempTypes| {
    types.entry(dest.0).or_insert_with(|| dest.1.clone());
  };

  match instr {
    Instr::BinOp { dest, lhs, rhs, .. } => {
      walk_dest(dest, temp_types);
      walk_op(lhs, temp_types);
      walk_op(rhs, temp_types);
    }
    Instr::UnOp { dest, src, .. } => {
      walk_dest(dest, temp_types);
      walk_op(src, temp_types);
    }
    Instr::Move { dest, src } => {
      walk_dest(dest, temp_types);
      walk_op(src, temp_types);
    }
    Instr::Load { dest, addr } => {
      walk_dest(dest, temp_types);
      walk_op(addr, temp_types);
    }
    Instr::Store { addr, src } => {
      walk_op(addr, temp_types);
      walk_op(src, temp_types);
    }
    Instr::Alloc { dest, size } => {
      walk_dest(dest, temp_types);
      walk_op(size, temp_types);
    }
    Instr::AllocArray { dest, size, count } => {
      walk_dest(dest, temp_types);
      walk_op(size, temp_types);
      walk_op(count, temp_types);
    }
    Instr::Call { dest, args, .. } => {
      if let Some(d) = dest {
        walk_dest(d, temp_types);
      }
      for a in args {
        walk_op(a, temp_types);
      }
    }
    Instr::TailCall { args, .. } => {
      for arg in args {
        walk_op(arg, temp_types);
      }
    }
    Instr::Return(Some(op)) => walk_op(op, temp_types),
    Instr::JumpIf { pred, .. } => walk_op(pred, temp_types),
    Instr::Phi { dest, srcs } => {
      walk_dest(dest, temp_types);
      for (_, op) in srcs {
        walk_op(op, temp_types);
      }
    }
    _ => {}
  }
}

/// Determine execution weight per-block via loop nesting depth heuristic (10^depth).
fn approximate_block_weights(ctx: &IRContext) -> HashMap<Label, f64> {
  let blocks = ctx.get_blocks();

  let mut depth: HashMap<Label, usize> = blocks.keys().map(|&label| (label, 0)).collect();
  let mut on_stack: HashSet<Label> = HashSet::new();
  let mut visited: HashSet<Label> = HashSet::new();

  fn dfs(
    label: Label,
    blocks: &HashMap<Label, BasicBlock>,
    visited: &mut HashSet<Label>,
    on_stack: &mut HashSet<Label>,
    depth: &mut HashMap<Label, usize>,
  ) {
    if !visited.insert(label) {
      return;
    }

    on_stack.insert(label);
    let block = match blocks.get(&label) {
      Some(block) => block,
      None => {
        on_stack.remove(&label);
        return;
      }
    };

    let successors = match &block.terminator {
      Some(Instr::JumpTo(label)) => vec![*label],
      Some(Instr::JumpIf { holds, fails, .. }) => vec![*holds, *fails],
      _ => vec![],
    };

    for successor in successors {
      if on_stack.contains(&successor) {
        *depth.entry(successor).or_insert(0) += 1;
        *depth.entry(label).or_insert(0) += 1;
      } else {
        dfs(successor, blocks, visited, on_stack, depth);
      }
    }

    on_stack.remove(&label);
  }

  if blocks.contains_key(&Label(0)) {
    dfs(Label(0), blocks, &mut visited, &mut on_stack, &mut depth);
  }

  let mut weights: HashMap<Label, f64> = HashMap::new();
  for (label, depth) in depth {
    let capped = min(depth, 10) as i32;
    weights.insert(label, 10f64.powi(capped));
  }

  weights
}
