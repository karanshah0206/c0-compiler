use std::collections::{HashMap, HashSet};

use crate::x86_back::x86_asm::X86Instr::{self, *};

/// Apply local x86 control-flow simplifications until no more changes are found.
pub fn simplify_control_flow(instrs: &mut Vec<X86Instr>) -> bool {
  let mut changed = false;
  changed |= merge_consecutive_labels(instrs);
  changed |= jump_threading(instrs);
  changed |= remove_redundant_jumps(instrs);
  changed |= branch_inversion(instrs);
  changed |= eliminate_dead_code(instrs);
  changed
}

/// Collapse runs of adjacent labels into a single canonical label.
fn merge_consecutive_labels(instrs: &mut Vec<X86Instr>) -> bool {
  let mut changed = false;
  let mut alias_to_primary: HashMap<String, String> = HashMap::new();
  let mut previous_label: Option<String> = None;

  for instr in instrs.iter() {
    if let Label(label) = instr {
      if let Some(prev) = &previous_label {
        alias_to_primary.insert(label.clone(), prev.clone());
      } else {
        previous_label = Some(label.clone());
      }
    } else {
      previous_label = None;
    }
  }

  if !alias_to_primary.is_empty() {
    let mut rewritten = Vec::with_capacity(instrs.len());
    for instr in instrs.iter() {
      match instr {
        Label(label) if alias_to_primary.contains_key(label) => changed = true,
        _ => rewritten.push(rewrite_jump_targets(instr.clone(), &alias_to_primary)),
      }
    }
    *instrs = rewritten;
  }

  changed
}

/// Retarget labels that immediately branch to another label.
fn jump_threading(instrs: &mut Vec<X86Instr>) -> bool {
  let mut changed = false;

  let mut jump_map: HashMap<String, String> = HashMap::new();
  let mut i = 0;
  while i < instrs.len() {
    if let Label(label) = &instrs[i] {
      let mut j = i + 1;
      while j < instrs.len() && matches!(instrs[j], Label(_)) {
        j += 1;
      }
      if j < instrs.len()
        && let Jmp(target) = &instrs[j]
      {
        jump_map.insert(label.clone(), target.clone());
      }
    }
    i += 1;
  }

  let mut label_to_target: HashMap<String, String> = HashMap::new();
  for (label, _) in jump_map.clone() {
    let resolved = resolve_label(label.clone(), &jump_map);
    if resolved != label {
      label_to_target.insert(label, resolved);
    }
  }

  if !label_to_target.is_empty() {
    let mut rewritten = Vec::with_capacity(instrs.len());
    for instr in instrs.iter() {
      let new_instr = rewrite_jump_targets(instr.clone(), &label_to_target);
      changed |= match (instr, &new_instr) {
        (Jmp(a), Jmp(b))
        | (Jne(a), Jne(b))
        | (Je(a), Je(b))
        | (Jl(a), Jl(b))
        | (Jg(a), Jg(b))
        | (Jle(a), Jle(b)) => a != b,
        _ => false,
      };
      rewritten.push(new_instr);
    }
    *instrs = rewritten;
  }

  changed
}

/// Remove jumps whose target is the next reachable label.
fn remove_redundant_jumps(instrs: &mut Vec<X86Instr>) -> bool {
  let mut changed = false;

  let check_is_redundant = |instrs: &[X86Instr], i: usize, target: &String| -> bool {
    let mut i = i + 1;
    while i < instrs.len() {
      if let Label(label) = &instrs[i] {
        if label == target {
          return true;
        }
        i += 1;
        continue;
      }
      return false;
    }
    false
  };

  let mut rewritten = Vec::with_capacity(instrs.len());
  let mut i = 0;
  while i < instrs.len() {
    match &instrs[i] {
      Jmp(target) | Jne(target) | Je(target) | Jl(target) | Jg(target) | Jle(target)
        if check_is_redundant(instrs, i, target) =>
      {
        changed = true;
        i += 1;
      }
      _ => {
        rewritten.push(instrs[i].clone());
        i += 1;
      }
    }
  }
  *instrs = rewritten;

  changed
}

/// Invert simple conditional branches to avoid an extra unconditional jump.
fn branch_inversion(instrs: &mut Vec<X86Instr>) -> bool {
  let mut changed = false;

  let get_inverse_condition = |instrs: &[X86Instr], i: usize| -> Option<X86Instr> {
    let cond = &instrs[i];
    let jump = &instrs[i + 1];
    let after = &instrs[i + 2];

    let (cond_target, flipped_builder): (&str, fn(String) -> X86Instr) = match cond {
      Jne(t) => (t.as_str(), Je),
      Je(t) => (t.as_str(), Jne),
      Jg(t) => (t.as_str(), Jle),
      Jle(t) => (t.as_str(), Jg),
      _ => return None,
    };

    let Jmp(else_target) = jump else {
      return None;
    };

    let Label(fallthrough_label) = after else {
      return None;
    };

    if cond_target == fallthrough_label {
      Some(flipped_builder(else_target.clone()))
    } else {
      None
    }
  };

  let mut rewritten = Vec::with_capacity(instrs.len());
  let mut i = 0;
  while i < instrs.len() {
    if i + 2 < instrs.len()
      && let Some(flip) = get_inverse_condition(instrs, i)
    {
      rewritten.push(flip);
      changed = true;
      i += 2;
      continue;
    }
    rewritten.push(instrs[i].clone());
    i += 1;
  }
  *instrs = rewritten;

  changed
}

/// Drop unreachable instructions and unreferenced labels.
fn eliminate_dead_code(instrs: &mut Vec<X86Instr>) -> bool {
  let mut changed = false;

  // remove unreachable blocks
  let mut rewritten = Vec::with_capacity(instrs.len());
  let mut can_emit = true;
  for instr in instrs.iter() {
    match instr {
      Label(_) => {
        can_emit = true;
        rewritten.push(instr.clone());
      }
      _ if can_emit => {
        let terminates = matches!(instr, Jmp(_) | Ret);
        rewritten.push(instr.clone());
        if terminates {
          can_emit = false;
        }
      }
      _ => changed = true,
    }
  }
  *instrs = rewritten;

  // delete unreferenced labels
  let mut referenced = HashSet::new();
  for instr in instrs.iter() {
    match instr {
      Jmp(label) | Jne(label) | Je(label) | Jl(label) | Jg(label) | Jle(label) => {
        referenced.insert(label.clone());
      }
      _ => {}
    }
  }
  let current = instrs.len();
  instrs.retain(|instr| match instr {
    Label(label) => referenced.contains(label),
    _ => true,
  });
  if instrs.len() != current {
    changed = true;
  }

  changed
}

/// Helpder to rewrite branch targets through a label replacement map.
fn rewrite_jump_targets(instr: X86Instr, replacements: &HashMap<String, String>) -> X86Instr {
  match instr {
    Jmp(label) => Jmp(resolve_label(label, replacements)),
    Jne(label) => Jne(resolve_label(label, replacements)),
    Je(label) => Je(resolve_label(label, replacements)),
    Jl(label) => Jl(resolve_label(label, replacements)),
    Jg(label) => Jg(resolve_label(label, replacements)),
    Jle(label) => Jle(resolve_label(label, replacements)),
    other => other,
  }
}

/// Helper that follows label replacements until a stable target is reached.
fn resolve_label(mut label: String, replacements: &HashMap<String, String>) -> String {
  let mut seen = HashSet::new();
  while let Some(next) = replacements.get(&label) {
    if !seen.insert(label.clone()) {
      break;
    }
    label = next.clone();
  }
  label
}
