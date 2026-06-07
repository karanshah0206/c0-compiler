use std::{
  cmp::Ordering,
  collections::{HashMap, HashSet},
};

use crate::intermediate::ir_context::IRContext;
use crate::x86_back::{
  x86_asm::X86Reg,
  x86_regalloc::{interference_graph::*, liveness_analysis::*, spill::*, *},
};

/// Max number of allocator iterations to run.
const MAX_ITERATIONS: usize = 3;

/// Status of each move edge during coalescing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MoveStatus {
  /// Move edge is currently in consideration for coalescing.
  Worklist,
  /// One of the move edge's nodes might re-enter the worklist.
  Active,
  /// Move edge was coalesced.
  Coalesced,
  /// Move edge cannot be coalesced because endpoints interfere.
  Constrained,
  /// Move edge appears impossible to coalesce.
  Fronzen,
}

/// Abstraction for running a full cycle of interference graph-based
/// register allocation with iterated register coalescing.
struct Allocator {
  /// Number of assignable register.
  k: usize,
  /// Interference graph.
  graph: InterferenceGraph,

  /// Set of nodes that are precolored.
  precolored: HashSet<Node>,
  /// The initial set of nodes coming in from the interference graph.
  initial: HashSet<Node>,
  /// Set of nodes that to be simplified.
  simplify_worklist: HashSet<Node>,
  /// Set of nodes that appear impossible to merge.
  freeze_worklist: HashSet<Node>,
  /// Set of nodes marked as potential spills.
  spill_worklist: HashSet<Node>,
  /// Stack to record allocation order during simplification.
  select_stack: Vec<Node>,

  /// Set of nodes that have been spilt.
  spilt_nodes: HashSet<Node>,
  /// Set of nodes that have been coalesced.
  coalesced_nodes: HashSet<Node>,
  /// Set of nodes that have been assigned a color.
  colored_nodes: HashSet<Node>,

  /// Processing status of each move indexed on the move edge's ID.
  move_status: Vec<MoveStatus>,
  /// IDs of move edges in consideration for coalescing.
  worklist_moves: HashSet<usize>,
  /// Set of move edge IDs that were deferred but may be coalescible.
  active_moves: HashSet<usize>,

  /// Alias mapping for coalesced nodes.
  alias: HashMap<Node, Node>,
  /// Color mapping for processed nodes.
  color: HashMap<Node, Color>,
}

impl Allocator {
  /// Generate a fresh allocator from an interference graph.
  fn new(graph: InterferenceGraph) -> Self {
    let mut allocator = Allocator {
      k: X86Reg::allocatable().len(),
      graph,
      precolored: HashSet::new(),
      initial: HashSet::new(),
      simplify_worklist: HashSet::new(),
      freeze_worklist: HashSet::new(),
      spill_worklist: HashSet::new(),
      select_stack: Vec::new(),
      spilt_nodes: HashSet::new(),
      coalesced_nodes: HashSet::new(),
      colored_nodes: HashSet::new(),
      move_status: Vec::new(),
      worklist_moves: HashSet::new(),
      active_moves: HashSet::new(),
      alias: HashMap::new(),
      color: HashMap::new(),
    };

    for _ in 0..allocator.graph.moves.len() {
      allocator.move_status.push(MoveStatus::Worklist);
    }

    for reg in X86Reg::allocatable() {
      let node = Node::Reg(reg);
      allocator.precolored.insert(node);
      allocator.color.insert(node, register_to_color(reg));
    }

    let mut temp_nodes: Vec<Node> = allocator
      .graph
      .adj
      .keys()
      .copied()
      .filter(|n| matches!(n, Node::Temp(_)))
      .collect();
    temp_nodes.sort_unstable();

    for n in temp_nodes {
      allocator.initial.insert(n);
    }

    allocator
  }

  /// Simplify/coalesce/freeze/spill until convergence and then allocate colors.
  fn run(&mut self) {
    self.make_worklists();

    loop {
      if !self.simplify_worklist.is_empty() {
        self.simplify();
      } else if !self.worklist_moves.is_empty() {
        self.coalesce();
      } else if !self.freeze_worklist.is_empty() {
        self.freeze();
      } else if !self.spill_worklist.is_empty() {
        self.select_spill_node();
      } else {
        break;
      }
    }

    self.perform_coloring();
  }

  /// Move a low-degree, non-move-related node into the select stack.
  fn simplify(&mut self) {
    let node = match self.simplify_worklist.iter().min().copied() {
      Some(node) => node,
      None => return,
    };

    self.simplify_worklist.remove(&node);
    self.select_stack.push(node);
    let mut neighbors: Vec<Node> = self.get_interfering_nodes_with(node).into_iter().collect();
    neighbors.sort_unstable();

    for neighbor in neighbors {
      self.decrement_node_degree(neighbor);
    }
  }

  /// Try coalescing nodes on a move edge from the worklist.
  fn coalesce(&mut self) {
    let move_id = match self.worklist_moves.iter().min().copied() {
      Some(move_id) => move_id,
      None => return,
    };
    self.worklist_moves.remove(&move_id);

    let MoveEdge { src, dest } = self.graph.moves[move_id];
    let src = self.resolve_node_alias(src);
    let dest = self.resolve_node_alias(dest);

    let (u, v) = if self.is_precolored(src) {
      (src, dest)
    } else {
      (dest, src)
    };

    if u == v {
      self.move_status[move_id] = MoveStatus::Coalesced;
      self.update_worklists(u);
      return;
    }

    if self.is_precolored(v) || self.graph.does_interfere(u, v) {
      self.move_status[move_id] = MoveStatus::Constrained;
      self.update_worklists(u);
      self.update_worklists(v);
      return;
    }

    let can_coalesce = if self.is_precolored(u) {
      self.check_george_criterion(v, u)
    } else {
      self.check_briggs_criterion(u, v)
    };

    if can_coalesce {
      self.move_status[move_id] = MoveStatus::Coalesced;
      self.merge_nodes(u, v);
      self.update_worklists(u);
    } else {
      self.move_status[move_id] = MoveStatus::Active;
      self.active_moves.insert(move_id);
    }
  }

  /// Pop a low-degree move-related node and freeze all its moves.
  fn freeze(&mut self) {
    let node = match self.freeze_worklist.iter().min().copied() {
      Some(node) => node,
      None => return,
    };

    self.freeze_worklist.remove(&node);
    self.simplify_worklist.insert(node);
    self.freeze_moves_for_node(node);
  }

  /// Spill a node from the spill worklist that has the lowest spill cost.
  fn select_spill_node(&mut self) {
    let mut best: Option<(Node, f64)> = None;
    for &node in &self.spill_worklist {
      let degree = self.get_node_degree(node) as f64;
      let temp_id = match node {
        Node::Temp(temp_id) => temp_id,
        _ => continue,
      };
      let weight = self.graph.spill_weight.get(&temp_id).copied().unwrap_or(1.);
      let priority = weight / degree.max(1.);

      best = match best {
        None => Some((node, priority)),
        Some((best_node, best_priority)) => {
          let better = priority
            .total_cmp(&best_priority)
            .then_with(|| node.cmp(&best_node))
            == Ordering::Less;
          if better {
            Some((node, priority))
          } else {
            Some((best_node, best_priority))
          }
        }
      };
    }

    if let Some((node, _)) = best {
      self.spill_worklist.remove(&node);
      self.simplify_worklist.insert(node);
      self.freeze_moves_for_node(node);
    }
  }

  /// Pop nodes from the select stack and assign them a color/mark as potential spill.
  fn perform_coloring(&mut self) {
    while let Some(node) = self.select_stack.pop() {
      let mut colors: HashSet<Color> = (1..=self.k).collect();

      let neighbors = self.graph.adj.get(&node).cloned().unwrap_or_default();
      for neighbor in neighbors {
        let neighbor = self.resolve_node_alias(neighbor);
        if neighbor == node {
          continue;
        }

        if let Some(color) = self.color.get(&neighbor) {
          colors.remove(color);
        }
      }

      if colors.is_empty() {
        self.spilt_nodes.insert(node);
      } else {
        let mut colors: Vec<Color> = colors.into_iter().collect();
        colors.sort_unstable();

        if let Node::Temp(temp_id) = node {
          let weight = self.graph.spill_weight.get(&temp_id).copied().unwrap_or(0.);
          let prefer_callee = weight > 5.;
          colors.sort_by_key(|color| {
            let reg = color_to_register(*color);
            let is_callee = X86Reg::callee_saved().contains(&reg);
            if prefer_callee == is_callee { 0 } else { 1 }
          });
        }

        let color = colors[0];
        self.color.insert(node, color);
        self.colored_nodes.insert(node);
      }
    }

    let mut coalesced: Vec<Node> = self.coalesced_nodes.iter().copied().collect();
    coalesced.sort_unstable();
    for node in coalesced {
      let alias_resolved_node = self.resolve_node_alias(node);
      if let Some(&color) = self.color.get(&alias_resolved_node) {
        self.color.insert(node, color);
      } else if self.spilt_nodes.contains(&alias_resolved_node) {
        self.spilt_nodes.insert(node);
      }
    }
  }

  /// Check if a node is already precolored.
  fn is_precolored(&self, node: Node) -> bool {
    self.precolored.contains(&node)
  }

  /// Get the degree of a node in the interference graph.
  /// Precolored nodes have an unbounded degree (set to `usize::MAX`)
  fn get_node_degree(&self, node: Node) -> usize {
    if self.is_precolored(node) {
      usize::MAX
    } else {
      *self.graph.degree.get(&node).unwrap_or(&0)
    }
  }

  /// Get the set of nodes that interfere with a given node.
  fn get_interfering_nodes_with(&self, node: Node) -> HashSet<Node> {
    self
      .graph
      .adj
      .get(&node)
      .cloned()
      .unwrap_or_default()
      .into_iter()
      .filter(|o| !self.select_stack.contains(o) && !self.coalesced_nodes.contains(o))
      .collect()
  }

  /// Get the move-edge IDs that are still alive associated with a node.
  fn get_node_moves(&self, node: Node) -> HashSet<usize> {
    self
      .graph
      .move_list
      .get(&node)
      .cloned()
      .unwrap_or_default()
      .into_iter()
      .filter(|o| self.active_moves.contains(o) || self.worklist_moves.contains(o))
      .collect()
  }

  /// Get the `temp_id`s of nodes that have been marked for spill.
  fn get_spilt_temp_ids(&self) -> Vec<usize> {
    let mut temp_ids: Vec<usize> = self
      .spilt_nodes
      .iter()
      .filter_map(|node| match node {
        Node::Temp(temp_id) => Some(*temp_id),
        _ => None,
      })
      .collect();
    temp_ids.sort_unstable();
    temp_ids
  }

  /// Resolve a node's alias chain to get to its concrete representation.
  fn resolve_node_alias(&self, mut node: Node) -> Node {
    while self.coalesced_nodes.contains(&node) {
      node = *self
        .alias
        .get(&node)
        .expect("Coalesced node missing alias.");
    }
    node
  }

  /// Check if a given node has any live move-edges.
  fn does_node_have_moves(&self, node: Node) -> bool {
    !self.get_node_moves(node).is_empty()
  }

  /// Initialize the simplify, spill, and freeze worklists from interference graph.
  fn make_worklists(&mut self) {
    let mut initial: Vec<Node> = self.initial.drain().collect();
    initial.sort_unstable();
    for node in initial {
      let degree = self.get_node_degree(node);

      if degree >= self.k {
        self.spill_worklist.insert(node);
      } else if self.does_node_have_moves(node) {
        self.freeze_worklist.insert(node);
      } else {
        self.simplify_worklist.insert(node);
      }
    }

    for i in 0..self.graph.moves.len() {
      self.worklist_moves.insert(i);
    }
  }

  /// Decrement the degree of a node after an interfering neighbor is removed.
  fn decrement_node_degree(&mut self, node: Node) {
    if self.is_precolored(node) {
      return;
    }

    let degree = self.get_node_degree(node);
    if degree == 0 || degree == usize::MAX {
      return;
    }
    self.graph.degree.insert(node, degree - 1);
    if degree == self.k {
      let mut nodes: Vec<Node> = self.get_interfering_nodes_with(node).into_iter().collect();
      nodes.sort_unstable();
      nodes.push(node);

      for n in nodes {
        let mut moves: Vec<usize> = self.get_node_moves(n).into_iter().collect();
        moves.sort_unstable();
        for mov in moves {
          if self.active_moves.contains(&mov) {
            self.active_moves.remove(&mov);
            self.worklist_moves.insert(mov);
          }
        }
      }

      self.spill_worklist.remove(&node);
      if self.does_node_have_moves(node) {
        self.freeze_worklist.insert(node);
      } else {
        self.simplify_worklist.insert(node);
      }
    }
  }

  /// Evaluate Brigg's criterion for coalescing two non-interfering nodes.
  /// Check that if nodes `u` and `v` are merged, the merged node has fewer than `k` neighbors of degree >= k.
  fn check_briggs_criterion(&self, u: Node, v: Node) -> bool {
    let mut interfering: HashSet<Node> = self.get_interfering_nodes_with(u);
    interfering.extend(self.get_interfering_nodes_with(v));
    let high_degree_count = interfering
      .iter()
      .filter(|node| self.get_node_degree(**node) >= self.k)
      .count();
    high_degree_count < self.k
  }

  /// Evaluate George's criterion for coalescing when one of the nodes is precolored.
  /// Check that all neighbors of node `u` either interfere with node `v` or have degree < k.
  fn check_george_criterion(&self, u: Node, v: Node) -> bool {
    for node in self.get_interfering_nodes_with(u) {
      if !(self.get_node_degree(node) < self.k
        || self.is_precolored(node)
        || self.graph.does_interfere(node, v))
      {
        return false;
      }
    }
    true
  }

  /// Add an interference edge between nodes `u` and `v`, updating their degrees if non-precolored.
  fn add_edge(&mut self, u: Node, v: Node) {
    if u == v || self.graph.does_interfere(u, v) {
      return;
    }

    self.graph.adj_set.insert((u, v));
    self.graph.adj_set.insert((v, u));
    self.graph.adj.entry(u).or_default().insert(v);
    self.graph.adj.entry(v).or_default().insert(u);
    if !self.is_precolored(u) {
      *self.graph.degree.entry(u).or_insert(0) += 1;
    }
    if !self.is_precolored(v) {
      *self.graph.degree.entry(v).or_insert(0) += 1;
    }
  }

  /// If a node is low-degree and non-move related, move it to simplify worklist.
  fn update_worklists(&mut self, node: Node) {
    if self.is_precolored(node) {
      return;
    }

    let degree = self.get_node_degree(node);
    if degree < self.k && !self.does_node_have_moves(node) {
      self.freeze_worklist.remove(&node);
      self.simplify_worklist.insert(node);
    }
  }

  /// Merge two nodes as part of coalescing.
  fn merge_nodes(&mut self, u: Node, v: Node) {
    if self.freeze_worklist.contains(&v) {
      self.freeze_worklist.remove(&v);
    } else {
      self.spill_worklist.remove(&v);
    }
    self.coalesced_nodes.insert(v);
    self.alias.insert(v, u);

    let v_moves = self.graph.move_list.remove(&v).unwrap_or_default();
    self.graph.move_list.entry(u).or_default().extend(v_moves);

    let mut neighbors: Vec<Node> = self
      .graph
      .adj
      .get(&v)
      .cloned()
      .unwrap_or_default()
      .into_iter()
      .filter(|o| !self.coalesced_nodes.contains(o))
      .collect();
    neighbors.sort_unstable();
    for neighbor in neighbors {
      self.add_edge(neighbor, u);
      self.decrement_node_degree(neighbor);
    }

    if !self.is_precolored(u)
      && self.get_node_degree(u) >= self.k
      && self.freeze_worklist.contains(&u)
    {
      self.freeze_worklist.remove(&u);
      self.spill_worklist.insert(u);
    }
  }

  /// Freeze all live moves for a given node.
  fn freeze_moves_for_node(&mut self, node: Node) {
    let mut moves: Vec<usize> = self.get_node_moves(node).into_iter().collect();
    moves.sort_unstable();

    for move_id in moves {
      let MoveEdge { src, dest } = self.graph.moves[move_id];
      let src = self.resolve_node_alias(src);
      let dest = self.resolve_node_alias(dest);
      let other = if self.resolve_node_alias(node) == src {
        dest
      } else {
        src
      };

      if self.active_moves.contains(&move_id) {
        self.active_moves.remove(&move_id);
      } else {
        self.worklist_moves.remove(&move_id);
      }
      self.move_status[move_id] = MoveStatus::Fronzen;

      if !self.is_precolored(other)
        && self.get_node_moves(other).is_empty()
        && self.get_node_degree(other) < self.k
      {
        self.freeze_worklist.remove(&other);
        self.simplify_worklist.insert(other);
      }
    }
  }

  /// Build and return the final vector mapping `temp_id`s to colors/spill.
  fn build_coloring(&self, temps_count: usize) -> Vec<Color> {
    let mut coloring = vec![UNCOLORED; temps_count];

    for (node, &color) in &self.color {
      if let Node::Temp(temp_id) = *node
        && temp_id < temps_count
      {
        coloring[temp_id] = color;
      }
    }

    for temp_id in self.get_spilt_temp_ids() {
      if temp_id < temps_count {
        coloring[temp_id] = SPILL;
      }
    }

    coloring
  }
}

/// Perform register allocation on a function and return coloring.
pub fn allocate_function(ctx: &mut IRContext, params_count: usize) -> Vec<Color> {
  for _ in 0..MAX_ITERATIONS {
    let liveness = analyze_liveness(ctx);
    let graph = interference_graph::build(ctx, &liveness, params_count);
    let mut allocator = Allocator::new(graph);
    allocator.run();

    let spilt = allocator.get_spilt_temp_ids();
    if spilt.is_empty() {
      return allocator.build_coloring(ctx.get_temps_count());
    }

    rewrite_spill(ctx, &spilt, &allocator.graph.temp_types.clone());
  }

  let liveness = analyze_liveness(ctx);
  let graph = interference_graph::build(ctx, &liveness, params_count);
  let mut allocator = Allocator::new(graph);
  allocator.run();

  allocator.build_coloring(ctx.get_temps_count())
}
