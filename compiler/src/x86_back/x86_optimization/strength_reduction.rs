use crate::x86_back::x86_asm::{
  self, LeaAddr,
  Width::{self, *},
  X86Instr::{self, *},
  X86Operand::{self, *},
  X86Reg, X86WReg,
};

/// Local x86_64 strength-reduction rewrites via peephole.
pub fn strength_reduction(instrs: &mut Vec<X86Instr>) -> bool {
  let mut changed = false;
  let mut rewritten = Vec::with_capacity(instrs.len());
  let mut i = 0;

  let is_memory_like = |op: X86Operand| -> bool { matches!(op, Stack(_) | Memory(_)) };
  let supports_lea_width = |width: Width| -> bool { matches!(width, W32 | W64) };

  while i < instrs.len() {
    // duplicate cmp/test
    if i + 1 < instrs.len() {
      match (&instrs[i], &instrs[i + 1]) {
        (Cmp(a1, b1), Cmp(a2, b2)) | (Test(a1, b1), Test(a2, b2)) if a1 == a2 && b1 == b2 => {
          changed = true;
          i += 1;
          continue;
        }
        _ => {}
      }
    }

    // move simplifications
    if i + 1 < instrs.len()
      && let (Mov(src, Register(tmp1)), Mov(Register(tmp2), dest)) = (&instrs[i], &instrs[i + 1])
      && tmp1 == tmp2
      && (X86WReg::scratch(W64).register == tmp1.register
        || X86WReg::scratch2(W64).register == tmp1.register)
      && (!is_memory_like(*src) || !is_memory_like(*dest))
    {
      rewritten.push(Mov(*src, *dest));
      changed = true;
      i += 2;
      continue;
    }

    if i + 1 < instrs.len()
      && let (Mov(Register(src), Register(dst)), Add(Immediate(imm), Register(add_dst))) =
        (&instrs[i], &instrs[i + 1])
      && dst == add_dst
      && src.width == dst.width
      && supports_lea_width(dst.width)
      && i32::try_from(imm.value).is_ok()
      && flags_dead_after(instrs, i + 1)
    {
      rewritten.push(Lea(
        LeaAddr {
          base: Some(src.register),
          index: None,
          scale: 1,
          disp: imm.value as i32,
        },
        *dst,
      ));
      changed = true;
      i += 2;
      continue;
    }

    if i + 1 < instrs.len()
      && let (Mov(Register(src), Register(dst)), Add(Register(idx), Register(add_dst))) =
        (&instrs[i], &instrs[i + 1])
      && dst == add_dst
      && src.width == dst.width
      && idx.width == dst.width
      && supports_lea_width(dst.width)
      && flags_dead_after(instrs, i + 1)
    {
      let index_reg = if idx.register == dst.register {
        src.register
      } else {
        idx.register
      };
      rewritten.push(Lea(
        LeaAddr {
          base: Some(src.register),
          index: Some(index_reg),
          scale: 1,
          disp: 0,
        },
        *dst,
      ));
      changed = true;
      i += 2;
      continue;
    }

    if i + 1 < instrs.len()
      && let (Mov(Register(src), Register(dst)), Sal(Some(Immediate(imm)), Register(shift_dst))) =
        (&instrs[i], &instrs[i + 1])
      && dst == shift_dst
      && src.width == dst.width
      && supports_lea_width(dst.width)
      && let Some(scale) = match imm.value {
        1 => Some(2),
        2 => Some(4),
        3 => Some(8),
        _ => None,
      }
      && flags_dead_after(instrs, i + 1)
    {
      rewritten.push(Lea(
        LeaAddr {
          base: None,
          index: Some(src.register),
          scale,
          disp: 0,
        },
        *dst,
      ));
      changed = true;
      i += 2;
      continue;
    }

    if let IMul(Some(Immediate(imm)), Register(src), Register(dst)) = &instrs[i]
      && src.width == dst.width
      && supports_lea_width(dst.width)
      && flags_dead_after(instrs, i)
      && let Some(lea_addr) = mul_const_to_lea(src.register, imm.value)
    {
      rewritten.push(Lea(lea_addr, *dst));
      changed = true;
      i += 1;
      continue;
    }

    // eliminate inconsequential move
    if i + 1 < instrs.len()
      && let (Mov(src1, Register(dst1)), Mov(src2, Register(dst2))) = (&instrs[i], &instrs[i + 1])
      && dst1 == dst2
      && matches!(*src1, Register(_) | Immediate(_))
      && !(match *src2 {
        Register(r) => r.register == dst1.register,
        Memory(m) => m.base == dst1.register,
        Stack(_) | Immediate(_) => false,
      })
    {
      changed = true;
      i = 1;
      continue;
    }

    // simple local optimizations
    match &instrs[i] {
      Mov(src, dst) if src == dst => changed = true,
      Mov(Immediate(x86_asm::Immediate { value: 0, .. }), Register(reg)) => {
        rewritten.push(Xor(Register(*reg), Register(*reg)));
        changed = true;
      }
      Cmp(Immediate(x86_asm::Immediate { value: 0, .. }), Register(reg)) => {
        rewritten.push(Test(Register(*reg), Register(*reg)));
        changed = true;
      }
      Add(Immediate(x86_asm::Immediate { value: 0, .. }), _)
      | Sub(Immediate(x86_asm::Immediate { value: 0, .. }), _)
        if flags_dead_after(instrs, i) =>
      {
        changed = true;
      }
      _ => rewritten.push(instrs[i].clone()),
    }

    i += 1;
  }

  if changed {
    *instrs = rewritten;
  }
  changed
}

/// Check if any instruction after `i` reads set flags.
fn flags_dead_after(instrs: &[X86Instr], i: usize) -> bool {
  let mut i = i + 1;
  while i < instrs.len() {
    let instr = &instrs[i];
    // block boundary
    if matches!(instr, Label(_) | Jmp(_) | Call(_) | Ret) {
      return true;
    }
    // reads flags
    if matches!(
      instr,
      Jne(_)
        | Je(_)
        | Jl(_)
        | Jg(_)
        | Jle(_)
        | Sete(_)
        | Setne(_)
        | Setl(_)
        | Setg(_)
        | Setle(_)
        | Setge(_)
    ) {
      return false;
    }
    // writes flags
    if matches!(
      instr,
      Add(_, _)
        | Sub(_, _)
        | IMul(_, _, _)
        | And(_, _)
        | Xor(_, _)
        | Or(_, _)
        | Sal(_, _)
        | Sar(_, _)
        | Neg(_)
        | Cmp(_, _)
        | Test(_, _)
    ) {
      return true;
    }
    i += 1;
  }
  true
}

/// Helper to try rewriring an `imul` as a `lea`.
fn mul_const_to_lea(src: X86Reg, k: i64) -> Option<LeaAddr> {
  match k {
    2 => Some(LeaAddr {
      base: None,
      index: Some(src),
      scale: 2,
      disp: 0,
    }),
    3 => Some(LeaAddr {
      base: Some(src),
      index: Some(src),
      scale: 2,
      disp: 0,
    }),
    4 => Some(LeaAddr {
      base: None,
      index: Some(src),
      scale: 4,
      disp: 0,
    }),
    5 => Some(LeaAddr {
      base: Some(src),
      index: Some(src),
      scale: 4,
      disp: 0,
    }),
    8 => Some(LeaAddr {
      base: None,
      index: Some(src),
      scale: 8,
      disp: 0,
    }),
    9 => Some(LeaAddr {
      base: Some(src),
      index: Some(src),
      scale: 8,
      disp: 0,
    }),
    _ => None,
  }
}
