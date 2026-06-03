# C0 Compiler

`c0mpile` is a multi-phase optimizing compiler written in Rust that translates [C0](https://jamiemorgenstern.com/teaching/su-122/misc/c0-reference.pdf) (a type-safe, memory-safe C-like language) into System V x86-64 assembly or LLVM IR.

## Optimization Pipeline

The IR optimization sequence is:

- scalar replacement of aggregates (SROA)
- copy propagation
- sparse conditional constant propagation and constant folding
- common subexpression elimination
- loop-invariant code motion
- aggressive dead-code elimination
- tail-call elimination
- CFG simplification

Optimization level behavior:

- `-O0` (default) disables the optimization loop
- `-O1` runs optimization iteratively with a 12-second per-function cap
- `-O2` and higher run the same loop without that cap

After register allocation, the x86 backend also runs x86-specific control-flow simplification and strength reduction (peephole) passes.

## Backends

### Abstract IR

The `abs` target emits the compiler’s internal SSA-style IR as text. This is the best target if you want to see how your source was lowered and optimized without being distracted by machine code details.

### LLVM IR

The `llvm` target emits textual LLVM IR. This backend is useful if you want to inspect or integrate with LLVM tooling.

It lowers the checked program structure directly into LLVM types and instructions, including struct type declarations, function declarations, and calls to runtime helpers such as `abort` and `calloc` when needed.

### x86-64 Assembly

The `x86-64` target emits System V AMD64 GNU/AT&T assembly. This backend includes register allocation and x86-specific post-processing.

This is where the compiler’s runtime safety model is most visible. In safe mode, the backend emits traps for issues such as null pointer dereferences, invalid array accesses, and invalid shift counts. The trap paths route through small runtime labels that call `abort` or raise the appropriate failure behavior.

The generated assembly uses the C0 symbol names with an `_c0_` prefix for compiler-generated functions. Header-declared external functions keep their original names.

## Safety Model

C0 is type-safe and memory-safe. The native backend inserts runtime checks for memory and arithmetic assumptions that C0 programs are expected to respect.

The `-u` / `--unsafe` flag turns runtime safety checks in the native backend and lets certain optimizations assume well-behaved inputs. This is useful for performance experiments and for compiling code that is already known to satisfy the language’s runtime constraints.

The compiler still performs full static type checking regardless of `-u`. Unsafe mode changes runtime validation and optimization assumptions, not the core language rules.

## Build

1. Install [Rust](https://rust-lang.org), [GCC](https://gcc.gnu.org), and the [LLVM toolchain](https://llvm.org). We need GCC not for compiling the compiler itself, but because this project doesn't have its own linker, so we rely on GCC (which is why we have `wrapper.c`).
2. Clone this repository and `cd` into the root folder.
3. Run `make release`. That builds the Rust crate in release mode and copies the resulting executable to `bin/c0mpile` in the root directory.

## Run

```bash
c0mpile [flags] source.c0
```

Example compiling, linking, and running the executable on x86-64:

```bash
c0mpile -e x86-64 -O2 source.c0
gcc source.c0.s wrapper.c -o source.out
./source.out
```

Example compiling, linking, and running the executable via LLVM:

```bash
c0mpile -e llvm -O1 source.c0
llc source.c0.ll 
gcc source.c0.s wrapper.c -o source.out
./source.out
```

Useful flags:

| Flag | Meaning |
| --- | --- |
| `-e x86-64` | Emit System V AMD64 assembly |
| `-e llvm` | Emit LLVM IR |
| `-e abs` | Emit abstract IR (default) |
| `-l header.c0` | Load external declarations from a header file |
| `-O`, `-O1`, `-O2` | Enable optimization; `-O` defaults to level 1 |
| `-u`, `--unsafe` | Disable runtime safety checks in native code generation |
| `-t`, `--typecheck-only` | Check whether program is well-formed without compiling |

Output files are written next to the source file by appending the target extension to the source filename:

- `.s` for x86-64 assembly
- `.ll` for LLVM IR
- `.abs` for abstract IR
