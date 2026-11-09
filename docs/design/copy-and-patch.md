# Design note: a copy-and-patch JIT from `vm` declarations

How the JIT built by `--jit-stencils` works, and why it was built that
way. For contributors to the compiler.

## Overview

`loimi build --jit-stencils` builds a baseline JIT into an interpreter,
generated from its `vm` declaration. `LOIMI_JIT=1` turns it on at run
time.

- **Stencils are the vm's own handlers**, compiled by clang with holes
  for operand fields and continuations. No stencil code is written by
  hand, and interpreters need no changes: Mini, MiniJS, Scheme and
  Prolog build with it as they are.
- **Coverage is complete.** Every op, family member and super gets a
  stencil, and every test of the four passes with the JIT forced on.
- **Speed:** 0.76–0.83 of the interpreter's time on Mini, MiniJS and
  Scheme (current numbers in [results.md](../results.md)), about 0.98 on
  Prolog (section 14).

The JIT removes dispatch and operand decoding. It does not remove the
loads and stores through the register file, the type checks, or the call
overhead (section 7).

Background: Xu & Kjolstad, "Copy-and-Patch Compilation" (OOPSLA 2021),
and CPython 3.13's JIT (`Tools/jit`), which also builds stencils with
clang.

The code: `src/jitgen.rs` (stencil IR), `build_stencils` in
`src/main.rs`, `src/elf.rs` (reading clang's object file), and
`src/jit_rt.c` (the run-time JIT).

## 1. Why Loimi handlers are nearly stencils already

In tail-call mode every op is a `preserve_none` function
`(ctx, state..., insn)` that ends in a `musttail` call through the
dispatch table, with the interpreter's state in fixed registers. A
copy-and-patch compiler needs exactly that: a fixed register interface
between templates, so their code can be pasted one after another, and a
place for each template to continue to.

So the stencil for an op is **its handler, specialized**. The
interpreter's state (`pc`, `regs`, `k`, `fp` for Mini) stays in the same
registers, and compiled code can enter and leave the interpreter at any
instruction boundary. No frame map, deoptimization metadata or state
reconstruction is needed.

## 2. From a handler to a stencil

`src/jitgen.rs` emits each op's handler a second time as
`__jit.op.NAME`, with the same body and these changes:

| Handler | Stencil |
|---|---|
| operand field decode `let raw = (insn >> s) & m` | a **hole**: `ptrtoint @"__jit.f.OFF.SHIFT.BITS.u"`, patched with the field's value |
| the instruction word `%insn` (passed to cold blocks and `on raise`) | the hole `__jit.w.0`, patched with the word |
| `next`, and any `jump pc + d` with a constant d | `musttail call @"__jit.n.D"`: a direct `jmp`, patched to the code for pc + d |
| `jump L` with L a label operand (`pc + it`) | `musttail call @"__jit.j.OFF.SHIFT.BITS"`, patched to the code for the label's target |
| any other `jump` (calls, returns) | the interpreter's dispatch, through the table |
| an outlined `cold { }` block | a tail call to the *interpreter's* cold function |
| `--vm-stats` counting | none |

**Holes** are external symbols whose *address* is the value, declared
`extern_weak dso_local` with `!absolute_symbol` range metadata. The
range lets LLVM use them as 32-bit immediates and displacements; the
`extern_weak` stops it assuming they are non-null. Unsigned fields are
patched pre-scaled by 8 (the size of a register slot), so `regs[a]`
folds into one instruction. Signed fields are patched with a bias, so
their range is non-negative too.

Mini's float add (`ADD_F`), as clang compiles the stencil:

```
movsd  __jit.f.0.16.8.u(%r14),%xmm0     ; R[b]
movsd  __jit.f.0.24.8.u(%r14),%xmm1     ; R[c]
movapd %xmm0,%xmm2
addsd  %xmm1,%xmm2
ucomisd %xmm2,%xmm2                     ; NaN: not both numbers
jp     slow
movsd  %xmm2,__jit.f.0.8.8.u(%r14)      ; R[a]
add    $0x4,%r13                        ; pc += 1
jmp    __jit.n.1                        ; the next instruction
```

The interpreter's handler does the same work, plus about six
instructions to decode the fields and four to dispatch.

**Direct jumps.** A `jump` target computed from pc and locals may be a
label. The stencil compares it with `pc + off + field` for each signed
field of the op, and on a match tail-calls `__jit.j.*`. Both sides come
from the same hole, so LLVM folds the compare and the jump becomes a
plain `jmp`. This covers `then` operands: Mini's `JLT a b; JMP L` jumps
straight to `L`'s code.

**Cold paths are exits.** An outlined cold block is a separate function
that re-decodes its operands from `insn`. The stencil passes it the
patched word and tail-calls the interpreter's copy, which ends in the
interpreter's dispatch. Inline `cold { }` blocks (such as `gc.collect`
before an allocation) stay in the stencil as calls.

**Superinstructions.** In a super's stencil, field holes and `next`
distances are counted from the head word, and LLVM optimizes across the
parts as in the interpreter. The later parts get stencils of their own
too, because jumps may land on them. This matters: with supers split
into parts, nbody took 0.48 s with the JIT against the interpreter's
0.46 s; with super stencils it takes 0.35 s.

## 3. Getting stencils out of clang

1. **The stencil module** is the program's IR, rewritten. Handlers,
   cold blocks, shared `-> never` functions and globals become
   `available_externally`: stencils use the interpreter's own by name.
   Other functions (*helpers*: `value.truthy`, the collector) stay
   `internal` and inline into stencils as into handlers; the JIT copies
   the ones still called. Functions no stencil reaches become
   declarations.
2. **clang** compiles it with `-O3 -fno-pic -ffunction-sections
   -fdata-sections`: one section per stencil, helper and constant.
3. **The ELF reader** (`src/elf.rs`) turns each relocation into a patch
   record: a hole (kind parsed from the symbol name), a program symbol,
   a helper call, a thread-local, or read-only data (copied). A stencil
   with an unsupported relocation is rejected and its op uses the exit
   stencil; no op of the four interpreters is rejected.
4. **The tables** (code bytes, patch records, helper code, the addresses
   of the program symbols used, read-only data) go into the program as
   IR constants.

Helpers are copied rather than shared with the interpreter by address:
taking their addresses changed how LLVM inlined them in the interpreter,
which cost a JIT build 5% with the JIT off.

Mini's 113 stencils were 21 KB of code, plus 17 KB of helpers. Stencils
add 1.6–2.3 s to an interpreter's build.

## 4. Interaction with the rest of the vm

Compiled code keeps the interpreter's state in the same registers and
memory, so most vm features work unchanged:

- **`sync`, `reload`, safepoints, root scanning, `raise`, `exit`** are
  the handlers' own code, compiled into the stencil.
- **Re-entry.** Nested runs (`Mini.call` from a sort comparator) reach
  compiled code through the dispatch table, like the outer run.
- **Inline caches** live in code words read through `pc`, and are shared
  by the interpreter and compiled code.
- **Quickening (`become`)** rewrites compiled code in place (section 10).
- **The verifier.** Only code built by `Vm.build_verified` is
  registered. Stencils rely on the same facts as the handlers, and the
  JIT checks that every continuation it patches lands on an instruction
  of the same code object.

Compiled code is never freed, so a return address into it never
dangles.

## 5. The run-time JIT, exits and re-entry

`src/jit_rt.c` is compiled into the program.

**Registry.** `Vm.build_verified` calls `loimi_jit_register(code, n)`.
Code objects are kept in a treap ordered by address (section 14). A new
object overlapping an old record means the old one was freed.

**Counting copies.** Handlers that make a jump the JIT counts or
intercepts (a call, a return, a label jump, a jump through a
`Vm.Resume`) or do a `become` are emitted twice: as in a default build,
and as a counting copy (`NAME.jc`). With `LOIMI_JIT=1` the runtime
points the dispatch table at the counting copies at startup. The
default copies' IR is byte-identical to a default build's, so a JIT
build with the JIT off is as fast as a default build. (Testing a flag in
every handler instead cost 5%.)

**Hot counters.** The counting copies decrement a 16-bit counter, in a
4096-entry table indexed by a hash of the target, on every call and
every backward label jump. Compiled code counts only calls. At zero,
`loimi_jit_hot(pc)` compiles the whole code object containing pc. The
default threshold is 50. This is LuaJIT's hot counting, applied to whole
functions.

**Compiling** is two passes: lay out one stencil copy per instruction,
then copy the stencils and apply the patch records. The JIT region is a
256 MiB `memfd` near the program (pc-relative calls into it must fit in
32 bits), mapped twice: executable where the code runs, and writable at
another address. Patching needs no `mprotect`, and no page is writable
and executable at once.

**Entering compiled code.** After compiling, each instruction word with
a stencil is replaced with an *enter word*, `ENTER | (index << 8)`.
`ENTER` is the highest free opcode, and its handler jumps to
`loimi_jit_entries[index]`. So the interpreter's dispatch enters
compiled code at any instruction: a call, a return, after an exit, at a
loop header (on-stack replacement for free), or from C. This is
LuaJIT's `JFUNC`/`JLOOP` patching, at every instruction. Words that
other instructions read (`then` operands, later words of a super) are
left alone.

Host code never sees enter words: in a JIT build the generated readers
of code words (`Vm.disasm`, the field getters, `Vm.verify`, ...) map a
word through `Vm.real_word`, an inline load from the runtime's
`loimi_jit_words` table. Handlers that read other instructions' words
get the real word the same way.

**Exits.** Compiled code leaves through the dispatch table: dynamic
jumps, cold paths, and instructions without a stencil (whose slot holds
the exit stencil). A `jump` to a computed pc that is none of the op's
labels (Mini's SWITCH) tests the target word first and goes straight to
its compiled code if it is an enter word. After an exit the interpreter
runs until it reaches an enter word. An exit costs one or two indirect
jumps, about what the interpreter pays per instruction anyway.

## 6. Results

Current numbers for all four interpreters are in
[results.md](../results.md). First measurements (2026-11-02, best of 5,
Scheme 3, interleaved, ±5% noise), geometric means of time ratios:

| | benchmarks | JIT off / interp | JIT / interp | range | JIT against references |
|---|---:|---:|---:|---:|---|
| Mini | 14 | 1.00 (0.99–1.02) | **0.77** | 0.59–0.97 | LuaJIT `-joff` 0.65, LuaJIT 2.3× slower |
| MiniJS | 14 | 1.00 (0.99–1.02) | **0.85** | 0.74–1.02 | node `--jitless` 0.45, QuickJS 0.35, node 3.9× slower |
| Scheme | 41 | 1.00 (0.94–1.06) | **0.79** | 0.50–1.84 | Guile 3 (JIT) 0.80, Guile without JIT 0.37, Gauche 0.23, Chibi 0.05 |

- The JIT gains most on numeric loops and calls. It gains little where
  the time is outside handlers: building strings, allocation, the GC,
  property lookup (MiniJS json 1.02).
- LuaJIT's trace compiler is 2–4× faster on Mini's numeric loops and
  calls; the JIT beats it only where allocation dominates.

**Compile time and size** (`LOIMI_JIT_STATS=1`): 250–700 ns per bytecode
instruction (Mini's interp: 931 instructions in 0.27 ms), cheap enough
to compile at the first call. Compiled code is 150–200 bytes per
instruction, because the stencils carry their type-check fallbacks
inline.

**Tests.** The golden tests `mini_jit_stencils`, `minijs_jit_stencils`,
`scheme_jit_stencils` and `prolog_jit_stencils` run every script and
conformance test with `LOIMI_JIT_HOT=1` (every function compiled at its
first call or loop), normally, under GC stress and with generational GC
verification.

## 7. Where the time goes

SIGPROF profiles with compiled code attributed to stencils
(`LOIMI_JIT_MAP`):

- **mandelbrot**: all samples in compiled code. A float op is about 9
  instructions, against 16 in its handler, but every value goes through
  the register file in memory and every op checks its operand types
  again. LuaJIT keeps the loop's variables in xmm registers with no type
  checks.
- **fib**: `CALL` 42%, `RET1` 18%. Call and return do the same work as
  in the interpreter; only the dispatch around them is gone.

Dispatch and decoding were 20–40% of the time in handler-bound code,
and copy-and-patch removes most of it. The next layer needs register
allocation and type specialization across instructions. Template
techniques against it gained nothing (sections 10, 12, 13).
Ahead-of-time traces over loops did remove it, and were removed
themselves ([speed-audit.md](speed-audit.md), item 14).

## 8. Limitations

- x86-64 Linux only, executables without PIE, tail-call dispatch.
- One vm per program, with 32-bit code words, the opcode in the low
  bits, one free opcode (for enter words), and no variants.
- Compiled code and registry records are never freed. A relocation that
  does not fit in the 256 MiB region fails that compile, and the code
  stays interpreted.

## 9. Recompile policy

After a rewrite that drops compiled code, a code object is compiled
again after 1/8 of the hot threshold, and given up on after 32
compiles. Most rewrites don't drop code (section 10).

## 10. Rewrites of compiled code

`become` calls `loimi_jit_become(word, opcode)`. `Vm.optimize` and
`Vm.deopt` bracket their rewrites with `loimi_jit_invalidate` and
`loimi_jit_rewritten`. For each rewritten instruction of compiled code,
the runtime uses the first of:

1. code it already has for the new word (a site going back and forth
   between members);
2. compiling the whole code object again, up to 32 compiles;
3. an out-of-line copy of the new op's stencil for that instruction;
4. dropping the code (out of memory only).

Old code is never overwritten. Instead, each instruction whose word can
change (`jitgen::rewritable_ops`) gets a 5-byte nop in front of its
code, its *home*, and every way into that code goes through the home.
Retargeting writes a `jmp` into the home. This is safe while old frames
are live: only homes are written, and no return address points into
one. A stencil that made the rewrite finishes as the old member, then
reaches the new code through the next home. So family members need not
be complete. A site retargeted more than 16 times is left to the
interpreter, because each write to running code costs a pipeline flush.

Against recompiling on every rewrite this is no faster (Mini 0.77 →
0.78 of the interpreter), but sites that keep changing no longer cost
whole recompiles, and no function is given up on because of quickening.
The double mapping matters: with two `mprotect` calls per retarget,
deltablue took 1.85× as long. The homes cost nothing measurable (0.997
without them).

`LOIMI_JIT_RECOMPILE=1` turns recompiles off, to test the other paths;
the golden JIT tests pass both ways.

Rejected: splitting stencils into hot and cold parts
(`-fsplit-machine-functions`): 1.01× the JIT time, nbody 13% slower.

## 11. Environment variables

At run time (`src/jit_rt.c`):

| Variable | Effect |
|---|---|
| `LOIMI_JIT=1` | turns the JIT on |
| `LOIMI_JIT_HOT=n` | hot threshold (default 50) |
| `LOIMI_JIT_STATS=1` | prints compile times, rewrite counts and objects registered |
| `LOIMI_JIT_DEBUG=1` | lists each compiled code object |
| `LOIMI_JIT_MAP=file` | writes address ranges of compiled code, for profilers |
| `LOIMI_JIT_RECOMPILE=n` | stops recompiling a code object after n compiles (for testing) |

At build time:

| Variable | Effect |
|---|---|
| `LOIMI_JIT_REPORT=1` | reports the stencils, and any rejected ones |
| `LOIMI_KEEP_STENCILS=dir` | keeps the stencil module and object |
| `LOIMI_JIT_VARIANTS=0` | turns off type variants (section 12) |
| `LOIMI_JIT_REGDELAY=n` | a latency probe (section 13) |

```sh
loimi build examples/mini/main.lm --release --jit-stencils -o mini-jit
LOIMI_JIT=1 LOIMI_JIT_STATS=1 ./mini-jit bench/mini/programs/fib.lua
```

## 12. Constant operands patched in

An operand kind may take a `const` clause ([language.md](../language.md)
§3.2):

```
operand kst = k[it] const proto.consts.data.ptr verify (it as u64) < proto.consts.data.len;
```

It promises that the pool cannot change once the code is verified; the
checker rejects any write through the pool's state register or address
taken in it. It costs the interpreter nothing. The JIT uses it in two
ways:

- **Values.** A read of a 64-bit const operand becomes the hole
  `__jit.c.*`, a 64-bit immediate patched with the constant (read with
  `movabs` in inline asm, because clang 19 sometimes truncated it to a
  32-bit immediate otherwise).
- **Variants.** An op with one const operand of a one-word `value` type
  gets a stencil per variant its patterns name (`ADDK.v.Int`,
  `ADDK.v.Num`), with the constant's tag known, so LLVM folds the tag
  tests. The runtime picks the variant from the constant.

Neither gains measurably (Mini 0.996, MiniJS 0.998, Scheme 1.003): a
constant load is an L1 hit and a type check of a constant is a
well-predicted branch. Variants grow Mini's stencils from 22 KB to 29 KB.

## 13. Guest registers in machine registers (experiment)

Not built. On mandelbrot's inner loop, edited by hand, all guest
registers in host registers would take 0.68 of the time; over the
suite, a latency probe (`LOIMI_JIT_REGDELAY`) projects at most 12%. A
one-register cache, built on branch `jit-regcache-experiment`, was 1.5%
slower by geometric mean.

## 14. Prolog (2026-11-04)

The Prolog interpreter (`examples/prolog`, a WAM) builds unchanged
(116 stencils). Two JIT changes came from it; before them the JIT build
took 1.11× the interpreter's time, and 48× on sieve:

- **Jumps through a `Vm.Resume` count like calls.** Prolog enters
  clauses that way, and they never got hot. On times10 the share of
  samples in compiled code went from 21% to 91%.
- **The registry is a treap.** Prolog registers a code object per
  asserted clause (700,000 on sieve), at addresses the collector reuses.
  A sorted array kept by `memmove` took 85% of sieve's time. The treap
  finds an overlapping record and inserts in one descent each.

The JIT build now takes **0.98** of the interpreter's time over 29
programs (0.83–1.16). Prolog's ops do much more work per dispatch, and
`backtrack` and PROCEED jump through one indirect jump each to many
more targets in compiled code than in the interpreter.

## 15. Placement of compiled code: the Scheme puzzle/array1 regression (2026-11-07)

The JIT packs stencil copies back to back, so a change to one stencil's
size moves all the code after it, and that alone can change a
loop-bound program's time by 10–20%. Found when a commit that shortened
some cold stencils by 5 bytes made Scheme's array1 1.07× slower with
identical hot stencils; JIT region start offsets 0–63 give array1
0.247–0.294 s, with a 32-byte period.

Rejected: padding copies around the JCC erratum (1.017, A/A 1.011) and
aligning loop heads to 32 or 64 bytes (0.992 and 0.999, A/A 0.995).

So a JIT A/B difference within ±10% on one short loop-bound program
should be rechecked with the JIT's code moved before it is taken as a
regression.
