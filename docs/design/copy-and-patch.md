# Design note: a copy-and-patch JIT from `vm` declarations

Status: **prototype**, behind flags (`loimi build --jit-stencils`, run
with `LOIMI_JIT=1`), x86-64 Linux, tail-call dispatch, one vm per
program. It runs all three example interpreters: Mini
(`examples/mini`), MiniJS and Scheme, none of them changed for it
(phase 4 marks their constant operands `const`).
Written 2026-11-02 to answer open question 6 of the
[roadmap](../roadmap.md); section 9 is the second phase (MiniJS and
Scheme, no cost with the JIT off, faster stencil builds), section 10
the third (rewrites of compiled code, hot and cold stencil parts:
neither faster), section 11 sketches immutable operand kinds, and
section 12 is the fourth phase, which implements them (no measurable
gain). Section 13 measures keeping guest registers in machine registers
(worth little over the suite; recommendation: stop the template-JIT
line).

> Handlers are separate functions with a fixed register interface, which
> is close to what a copy-and-patch template compiler needs. Can `vm`
> declarations generate a baseline JIT with no additional source?

**Answer: yes.** The prototype generates its stencils from Mini's
existing handlers. It needs no change to Mini's source and no stencil
code written by hand. It compiles every op (113 stencils, supers
included). It passes every Mini script and conformance test with the
JIT forced on at the first call or backward jump.

Against Mini's interpreter, it takes 32% off fib, 11% off sieve and
41% off mandelbrot, and 22% by geometric mean over the 14 benchmarks
(23% after phase 2; MiniJS 15%, Scheme 21%; section 9).
It is faster than LuaJIT's interpreter on all 14. It is still 2.3×
slower than LuaJIT with its trace JIT on (2.4–3.5× on those three). A
baseline JIT removes dispatch and operand decoding. It does not remove
the loads and stores through the register file, the type checks, or
the call overhead. Results, profiles and the recommendation are in
sections 6–8.

Background: Xu & Kjolstad, "Copy-and-Patch Compilation" (OOPSLA 2021),
and CPython 3.13's JIT (`Tools/jit`), which builds stencils with clang
from C templates with holes, and copies and patches them at run time.

## 1. Why Loimi handlers are nearly stencils already

In tail-call mode (`lowering.md` §3) every op is a `preserve_none`
function `(ctx, state..., insn)` that ends in a `musttail` call through
the dispatch table. The interpreter's state lives in fixed registers
across those calls. A copy-and-patch template compiler needs exactly
that: a fixed register interface between templates, so that template
code can be pasted one after another, and a place for each template to
continue to.

So the stencil for an op is **its handler, specialized**. The
interpreter's state (`pc`, `regs`, `k`, `fp` for Mini) stays in the same
registers, and compiled code can enter and leave the interpreter at any
instruction boundary. Section 5 shows how much that simplifies.

## 2. From a handler to a stencil

`src/jitgen.rs` emits each op's handler a second time as
`__jit.op.NAME`, with the same body (the same typed tree) and these
changes:

| Handler | Stencil |
|---|---|
| operand field decode `let raw = (insn >> s) & m` | a **hole**: `ptrtoint @"__jit.f.OFF.SHIFT.BITS.u"`, patched with the field's value |
| the instruction word `%insn` (passed to cold blocks and `on raise`) | the hole `__jit.w.0`, patched with the word |
| `next`, and any `jump pc + d` with a constant d | `musttail call @"__jit.n.D"`: a direct `jmp`, patched to the code for pc + d |
| `jump L` with L a label operand (`pc + it`) | `musttail call @"__jit.j.OFF.SHIFT.BITS"`, patched to the code for the label's target |
| any other `jump` (calls, returns) | the interpreter's dispatch, through the table |
| an outlined `cold { }` block | a tail call to the *interpreter's* cold function, unchanged |
| `--vm-stats` counting | none |

**Holes** are external symbols whose *address* is the value. They are
declared `extern_weak dso_local` with `!absolute_symbol` metadata giving
their range, so that LLVM:

- uses them as 32-bit immediates and displacements (`R_X86_64_32`,
  `R_X86_64_32S`). Without a range, a 64-bit `movabs` would be needed;
- does not assume they are non-null. For an ordinary global LLVM folded
  `LOADBOOL`'s `b != 0` to `true`; `extern_weak` stops that.

Unsigned fields are patched pre-scaled by 8 (the size of a register
slot) and decoded as `lshr exact h, 3`. A register access `regs[a]` then
folds into one instruction, `movsd hole(%r14), %xmm0`, where the
interpreter decodes the field with a shift and a mask first. Signed
fields are patched with a bias of 2^(bits-1), so their range is
non-negative too.

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
instructions to decode the three fields from the word, and four to
dispatch: load the next word, extract the opcode, load the handler from
the table, and jump indirectly.

**Direct jumps.** A `jump` target is classified from the typed tree:
`pc + constant` is a known distance; something computed from pc and
locals may be a label. For the second kind the stencil compares the
target with `pc + off + field` for each signed field of the op (its
label candidates), and on a match tail-calls `__jit.j.*`. Both sides
are built from the same hole, so LLVM folds the compare to true where
the target is that label, and the jump is a plain `jmp`. This covers
`then` operands: Mini's compare-and-branch ops (`JLT a b; JMP L`) jump
straight to `L`'s code. Anything else (`jump p.code` in `CALL`,
`jump fp.resume` in `RET`) goes through the dispatch table. That is
where compiled code exits to the interpreter, and how it enters other
compiled code (section 5).

**Cold paths stay out of line.** An outlined cold block is already a
separate function with the handler's signature, which re-decodes its
operands from the `insn` register. The stencil passes the patched word
and tail-calls the interpreter's copy. The cold path then ends in the
interpreter's own dispatch: **every cold path is an exit to the
interpreter**. Inline `cold { }` blocks (unit-typed, such as
`gc.collect` before an allocation) stay in the stencil as calls.

**Superinstructions** get stencils too. A super's handler is a sequence
of parts. In the stencil, each part's fields are holes numbered from the
head word (`__jit.f.1.8.8.u` is a field of the word after the head), and
a part's `next` becomes `__jit.n.D` with D counted from the head. LLVM
still optimizes across the parts, as in the interpreter: Mini's float
supers (`MUL_F_ADD_F`, `GETFIELD_MUL_F`, ...) and `#[forward]` supers
keep a value in a register between two ops. The words of a super's
later parts still get stencils of their own, because jumps may land on
them (`redo` and quickening run them alone as well). This mattered: with
supers split into their parts, nbody was 4% *slower* with the JIT than
the interpreter (0.48 s against 0.46 s). With super stencils it is 25%
faster (0.35 s).

## 3. Getting stencils out of clang

`main.rs` (`build_stencils`) and `src/elf.rs`:

1. **The stencil module** is the program's IR, rewritten:
   - globals and handler-convention functions (cold blocks, `on raise`,
     shared `-> never` functions, other handlers) become
     `available_externally dso_local`. Stencils refer to them *by name*:
     they use the interpreter's own.
   - other functions (*helpers*: `value.truthy`, `table_set`, the
     collector) stay `internal`. They inline into stencils as they do
     into handlers. The ones a stencil still calls are compiled into the
     object, and the JIT copies them once (step 3).
   - functions no stencil reaches become declarations, and `main` and
     exports become `available_externally`, so clang compiles only what
     the stencils use.

   Thread-locals become `localexec`. The stencils, the exit stencil and
   the hole declarations are appended.

   The prototype first made helpers `available_externally` too, so
   stencils called the interpreter's copies through `loimi_jit_syms`.
   Taking their addresses there changed how LLVM inlined them *in the
   interpreter*: in a JIT build `value.table_set` was inlined into
   `SETINDEXK` and `hash_set` was not, and the reverse in a default
   build. That was most of the 5% that a JIT build cost with the JIT off.
2. `clang -O3 -fno-pic -ffunction-sections -fdata-sections -c`: one
   section per stencil, helper, and constant.
3. **The ELF reader** (about 300 lines of Rust, no crates) walks the
   section headers, symbol table and `.rela.text.__jit.op.*` sections.
   Each relocation becomes a patch record of one of these kinds:
   - a hole (`__jit.w`, `__jit.f`, `__jit.n`, `__jit.j`, parsed from the
     symbol name);
   - a program symbol (cold functions, `vm.Mini.table`, globals, libc);
   - a helper: a function of the object (a call). Helpers' own
     relocations are read the same way, and they make one block of
     helper code. A helper must not use holes;
   - a thread-local (`R_X86_64_TPOFF32`);
   - read-only data of the object (float constants, `.rodata.cst*`),
     which is copied. A helper's jump table is data with relocations: it
     is copied too, and patched once. Stencils have no jump tables
     (`"no-jump-tables"`).

   Relocation types are `R_X86_64_64`, `PC32`, `PLT32`, `32`, `32S` and
   `TPOFF32`. A stencil with any other relocation is rejected, and so is
   one that refers to another stencil, to data with relocations, or to
   a helper that is rejected. Its op then uses the exit stencil. No op of
   Mini, MiniJS or Scheme is rejected.
4. **The tables** go into the program as IR constants: the code bytes,
   the patch records, a per-opcode stencil index, the helper code and
   its patch records, `loimi_jit_syms` (the addresses of the program
   symbols the stencils use), the read-only data, and a function
   computing each thread-local's offset from the thread pointer
   (`llvm.threadlocal.address` minus `%fs:0`).

Mini's 113 stencils are 21 KB of code, plus 26 helpers (17 KB).
Build times are in section 9.

A stencil whose fast path ends in `jmp __jit.n.D` as its *last*
instruction has the jump dropped when the next instruction's code
follows. In practice LLVM places the fast path first and the cold
blocks after it, so the jump is usually in the middle. It stays a
direct `jmp` (section 7; splitting stencils into hot and cold parts
did not pay, section 10).

## 4. Interaction with the rest of the vm

- **State, `sync`, `reload`, GC.** Compiled code keeps the interpreter's
  state in the same registers and memory, so `sync` blocks, safepoints,
  root scanning and `reload` after re-entrant calls are the handlers'
  own code, compiled into the stencil. No native frame is live across a
  safepoint except those of plain calls from a stencil (`gc.collect` in
  an inline cold block), which return into it. Compiled code is never
  freed, so a return address into it never dangles.
- **`raise`, `exit`.** Unchanged: `raise` tail-calls the vm's
  `on raise` with the patched word, and `exit` returns from the chain of
  tail calls to the entry, as from a handler.
- **Re-entry.** Nested runs (`Mini.call` from `table.sort`'s comparator)
  enter through `jump p.code` and the dispatch table, and reach compiled
  code the same way the outer run does. The sort and re-entry tests
  pass with the JIT forced on.
- **Inline caches** live in code words after the instruction and are
  read through `pc`, which compiled code keeps up to date. They work
  unchanged, and the interpreter and compiled code share them.
- **Quickening (`become`).** The JIT compiles the opcodes it finds, so
  it compiles quickened members once they are hot. In a JIT build,
  `become` in a stencil or counting copy calls
  `loimi_jit_become(word, opcode)`, which makes the rewrite. If the code
  object is compiled, the runtime also updates its compiled code: it
  compiles the code object again at once, or points the instruction at
  code it already has (section 10). `Vm.optimize` and `Vm.deopt`
  (generated) bracket their rewrites with `loimi_jit_invalidate` and
  `loimi_jit_rewritten`, so `refuse` (re-fusing supers after quickening)
  never sees an enter word. Instructions that can be rewritten get a
  5-byte prefix in front of their code, which the runtime turns into a
  jump to the new code. Old code that is still running therefore reaches
  the new member the next time it gets to that instruction; only the
  copy that made the rewrite finishes as the old member. Members need
  not be complete, as long as the runtime does not give up on the code
  object (it does only when out of memory).
- **The verifier.** Only code built by `Vm.build_verified` is registered
  with the JIT. Stencils rely on the facts the handlers rely on (operands
  in range; label targets are instruction boundaries), and the JIT
  checks that every continuation it patches lands on an instruction of
  the same code object. Enter words use an opcode no op has, so a
  verified program cannot contain one, and the verifier would reject a
  copy of patched code.

## 5. The run-time JIT, exits and re-entry

`src/jit_rt.c` (about 500 lines of C, compiled into the program):

- **Registry.** `Vm.build_verified` calls `loimi_jit_register(code, n)`.
  Code objects are kept sorted by address. A new object overlapping an
  old record means the collector freed the old one, so the record is
  dropped.
- **Counting copies.** Handlers and shared `-> never` functions that make
  a jump the JIT counts or intercepts (a call, a return, a label jump) or
  a `become` are emitted twice in a JIT build: as in a default build, and
  as a counting copy (`NAME.jc`) with the hooks below
  (`jitgen::counted_fns`; also those that tail-call such a function
  directly). With `LOIMI_JIT=1` the runtime points the dispatch table
  at the counting copies at startup. Without it, the interpreter is the
  default build's code. The copies go last in the module and number
  their own cold blocks and strings, so the default copies' IR is
  byte-identical to a default build's. (The prototype tested a flag
  before counting, in every handler; that cost 5%.)
- **Hot counters.** With the JIT on, the interpreter
  decrements a 16-bit counter in a 4096-entry table, indexed by the hash
  of the target. It does this on every `jump` to the start of a verified
  code object (a call: `jump p.code`) and on every backward label jump
  (a loop). Compiled code counts only calls. Returns never count: a
  caller gets hot through its own calls and loops. At zero, the handler
  tail-calls `Vm.jit_hot`, which calls `loimi_jit_hot(pc)`. That
  function finds the code object containing pc and compiles it whole
  (default threshold 50, `LOIMI_JIT_HOT`). This is LuaJIT's hot
  counting (`hotcall` in function headers, `hotloop` in loops), applied
  to whole functions. Calls and returns that reach an enter word jump
  straight to its compiled code, without counting and without the enter
  handler.
- **Helpers and the exit.** At the first compile, the runtime copies the
  helper code next to the stencils' read-only data and patches it, and
  makes one copy of the exit stencil.
- **Compiling** a code object is two passes. The first walks the
  instructions (`opinfo`: per opcode, the size and stencil to use) and
  lays out one stencil copy per instruction. The second copies the
  stencils and applies the patch records. The region is `mmap`ped near
  the program (pc-relative calls into it must fit in 32 bits). It is a
  `memfd` mapped twice: executable where the code runs, and writable
  at another address, where the runtime writes it. Patching therefore
  needs no `mprotect`, and no page is writable and executable at once.
- **Entering compiled code.** After compiling, each instruction word that
  has a stencil is replaced with `ENTER | (index << 8)`. `ENTER` is the
  highest free opcode, and its table slot holds `Vm.jit_enter`, a
  generated handler that loads `loimi_jit_entries[index]` and jumps
  there. The interpreter's dispatch therefore enters compiled code at
  any such instruction: a call to a compiled function (`jump p.code`), a
  return into one (`jump fp.resume`), the first instruction after an
  exit, a loop header (on-stack replacement for free), and an entry from
  C (`Mini.run`, `Mini.call`). This is LuaJIT's `JFUNC`/`JLOOP` patching,
  at every instruction. Words that other instructions read are left
  alone: a `then` operand word, and the later words of a super, whose
  head may still run in the interpreter.
- **Exits.** Compiled code leaves through the dispatch table: dynamic
  jumps, cold paths, and instructions without a stencil (their slot holds
  the exit stencil, `musttail tbl[opcode(*pc)]`). A continuation that
  is not an instruction goes to the shared exit copy. It is never taken:
  for example, MiniJS's `SWITCHI` jumps to `pc + 1 + i`, which the
  stencil compares with `pc + field` for each signed field, and a field
  that is not a label gives a target outside the code. The prototype
  failed the whole compile instead. The interpreter then
  runs until it dispatches to an enter word. An exit costs one or two
  indirect jumps, about what the interpreter pays per instruction anyway.

No frame map, deoptimization metadata or state reconstruction is
needed, because compiled code and the interpreter share one frame
layout and one register assignment. A baseline JIT whose stencils kept
values in machine registers across instructions would need all three.

## 6. Results

Seconds, best of 5, interleaved (each binary in turn), on the shared
8-core machine (load average 1.4–2.0; about ±5% noise).
*interpreter*: Mini's default build. *JIT off*: the `--jit-stencils`
build without `LOIMI_JIT`. *JIT*: the same with `LOIMI_JIT=1` (threshold
50). LuaJIT 2.1 as in `bench/mini/RESULTS.md`. Every run printed the
expected output.

| Benchmark | interpreter | JIT off | JIT | LuaJIT -joff | LuaJIT | JIT/interp | JIT/-joff | JIT/LuaJIT |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| **fib** | 1.029 | 1.129 | 0.703 | 1.125 | 0.199 | 0.68 | 0.62 | 3.5 |
| **sieve** | 0.743 | 0.806 | 0.659 | 1.000 | 0.273 | 0.89 | 0.66 | 2.4 |
| **mandelbrot** | 0.693 | 0.742 | 0.410 | 0.783 | 0.159 | 0.59 | 0.52 | 2.6 |
| binary_trees | 0.999 | 1.003 | 0.857 | 1.563 | 1.140 | 0.86 | 0.55 | 0.75 |
| closures | 0.483 | 0.490 | 0.468 | 0.936 | 0.795 | 0.97 | 0.50 | 0.59 |
| fannkuch | 1.229 | 1.299 | 0.981 | 1.430 | 0.313 | 0.80 | 0.69 | 3.1 |
| hash_strings | 1.053 | 1.116 | 0.802 | 1.046 | 0.236 | 0.76 | 0.77 | 3.4 |
| interp | 1.022 | 1.144 | 0.737 | 1.094 | 0.373 | 0.72 | 0.67 | 2.0 |
| nbody | 0.463 | 0.468 | 0.346 | 0.517 | 0.090 | 0.75 | 0.67 | 3.8 |
| oo_methods | 0.781 | 0.812 | 0.614 | 1.018 | 0.192 | 0.79 | 0.60 | 3.2 |
| queens | 0.918 | 0.963 | 0.714 | 0.943 | 0.508 | 0.78 | 0.76 | 1.4 |
| sort | 0.849 | 0.910 | 0.659 | 0.871 | 0.322 | 0.78 | 0.76 | 2.0 |
| spectral_norm | 1.132 | 1.160 | 0.792 | 1.163 | 0.074 | 0.70 | 0.68 | 10.7 |
| string_build | 0.951 | 0.944 | 0.920 | 0.974 | 0.568 | 0.97 | 0.94 | 1.6 |
| geometric mean | | 1.05× interp | | | | **0.78** | **0.66** | **2.3** |

- **The JIT takes 22% off by geometric mean** (3–41%). It is faster
  than LuaJIT's interpreter on all 14 benchmarks (0.50–0.94 of its
  time; Mini's interpreter alone wins 11 of 14). LuaJIT's trace
  compiler is still 2.3× faster by geometric mean: 2–4× on numeric
  loops and calls, and 10× on spectral_norm, where it inlines the
  `A(i, j)` call. The JIT beats it only where allocation dominates
  (binary_trees, closures).
- **There is little gain where the work is not in handlers.**
  string_build (3%) spends its time building and interning strings,
  closures (3%) in `CLOSURE`'s allocation and the GC, and sieve (11%)
  partly in table growth (`array_grow` and `array_append`: 16% of its
  profile). closures was 15% faster in an earlier build that also
  counted returns, and that difference held over repeated runs. It
  appears to be a layout effect, not understood.
- **JIT off costs 5% by geometric mean** (up to 12% on interp, 10% on
  fib). Phase 2 brought this to 0 (section 9): the cause was mostly
  helpers' addresses taken for the stencils, which changed LLVM's
  inlining in the interpreter, and partly the flag test before counting.
- **Default builds are unaffected.** Without `--jit-stencils` the IR
  for Mini is byte-identical to the IR from before the prototype.

**Compile time** (`LOIMI_JIT_STATS=1`, which includes the first
compile's `mmap`): 7–30 µs per function, or 250–700 ns per bytecode
instruction. fib's one function (12 instructions) takes 8 µs and
mandelbrot's main chunk (83 instructions) 21 µs. interp compiles 19
functions (931 instructions) in 0.27 ms in total. That is cheap enough
to compile at the first call. Compiled code is large, though: 150–200
bytes per instruction (188 KB for interp's 931), against 4 bytes of
bytecode. The stencils carry their type-check fallbacks inline, after
the fast path.

**Correctness.** The new golden test `mini_jit_stencils` runs all Mini
scripts and the 120 conformance tests (normally, under GC stress, and
with generational GC verification) with `LOIMI_JIT_HOT=1`. Every
function is therefore compiled at its first call or loop, before
quickening, and compiled again after each invalidation. Over the suite
that is 726 compiles and 436 invalidations. No op exits to the
interpreter for lack of a stencil.

## 7. Where the time goes

SIGPROF profiles, with compiled code attributed to stencils
(`LOIMI_JIT_MAP`):

- **mandelbrot**: 100% of samples are in compiled code: `JGTKN` 23%,
  `MUL_F` 20%, `ADD_F` 19%, `FORLOOP` 14%. A float op is now about 9
  instructions (two loads from `regs`, the operation, the NaN test, the
  store, `pc += 1`, a `jmp`), against about 16 in its handler. What is
  left, a template JIT cannot remove. Every value goes through the
  register file in memory, so `zi = 2.0 * zr * zi + ci` is a chain of
  store-to-load forwardings, and every op checks its operand types
  again. LuaJIT's trace keeps `zr`, `zi`, `zrq` and `ziq` in xmm
  registers and has no type checks inside the loop.
- **fib**: `CALL` 42%, `RET1` 18%, then the compare, `ADDI` and `ADD`.
  Call and return do the same work as in the interpreter (frame push,
  clearing the callee's registers, stack checks, frame pop); only the
  dispatch around them is gone.
- **sieve**: `SETINDEXK` 36%, `FORLOOP` 24%, `GETINDEX` 10%, and 16% in
  the table runtime.
- **nbody**, before super stencils: 10% of samples were in
  `Vm.jit_enter`. Inline-cache misses (`GETFIELD` on tables of different
  shapes) take the cold path, which exits to the interpreter's cold
  function, whose `next` re-enters compiled code through the enter
  word: two extra indirect jumps per miss.

Mini's interpreter is already tuned: supers, quickening,
compare-and-branch ops, NaN-tagged float fast paths. Even so,
**dispatch and operand decoding were 20–40% of the time in
handler-bound code, and copy-and-patch removes most of that.** It does
not touch the next layer: memory traffic through the register file,
repeated type checks, and call overhead. LuaJIT's remaining 2–4× comes
from there, and removing it needs register allocation and type
specialization across instructions. That is an optimizing compiler,
not a template one.

What the prototype leaves on the table, each worth a few percent at
most (by inspection, not measured):

- Most stencils end their fast path with `jmp __jit.n.D` *in the
  middle*, with the cold blocks after it, so every op takes one taken
  direct jump. Splitting stencils into hot and cold parts (as CPython
  3.14 does) was tried in phase 3 and measured no faster (section 10).
- `pc` is kept current in its register (`add $4` per op). Keeping it
  as a hole constant and materializing it only on exits would save
  that, but cache-field accesses would then need 64-bit addresses.
- Cold paths exit to the interpreter (above). Stencil copies of cold
  blocks whose `next` continues in compiled code would fix that.
- `k[it]` is still a load. The constant pool cannot change after
  verification, but Loimi has no way to say so (section 8). Phase 4 added
  the way and patched constants in: no measurable gain (section 12).

## 8. Recommendation

**Pursue it after 1.0, as an opt-in second tier, but do not present it
as a substitute for an optimizing JIT.**

- It answers the open question: the stencils come from the `vm`
  declaration with no additional source. The interpreter author writes
  nothing. Every op, family member and super is covered, including
  inline caches, `then` operands, `cold` blocks, `sync`, `raise`,
  re-entry, and GC stress.
- The gain is real but modest: 22% by geometric mean, over an
  interpreter that already beats LuaJIT's hand-written one. That is
  about as much as the whole second phase of interpreter tuning in
  `bench/mini/RESULTS.md` bought. It costs about 1,200 lines of
  compiler and runtime, and the build takes half as long again.
- The design extends to a full version without new concepts, because
  compiled code and the interpreter share one state layout: exits and
  re-entry are just dispatch.

What a full version needs:

1. **Language.**
   - *Immutable operands*: a way to say that what an operand kind reads
     cannot change after verification (`operand kst = const k[it]`).
     The JIT could then patch a constant's value in as a 64-bit
     immediate instead of loading it, and fold type checks on it
     (`ADDK_F` with a known float needs no check on its second
     operand). This is the first step to type specialization, and the
     only `vm` syntax change this work suggests (sketch in section 11).
     Done in phase 4, as a `const` clause (section 12). On its own it
     measured no faster.
   - ~~*Complete members*~~: since phase 3, old compiled code follows a
     rewrite at the rewritten instruction (section 10), so members need
     not be complete.
2. **Compiler.**
   - ~~Build the stencil module from what the handlers reach~~ (done in
     phase 2: only reachable functions are compiled).
   - Cold continuations that return into compiled code. (Hot and cold
     stencil parts, so that fast paths fall through, were tried in
     phase 3: no faster.)
   - ~~An interpreter that does not count when the JIT is off~~ (done in
     phase 2, with counting copies of the handlers that jump).
   - Other targets: AArch64 relocations (`ADRP`/`ADD`, 26-bit `B`/`BL`,
     `MOVZ`/`MOVK` for 64-bit holes), Mach-O and COFF, and
     position-independent programs (a per-region GOT, as in CPython).
   - Several vms per program (per-vm runtime tables).
3. **Runtime.**
   - Free compiled code on invalidation, and when the collector frees a
     code object. Compiled code holds no frames except plain helper
     calls, so freeing at the next safepoint is safe. Tie the registry
     to the collector (a sweeper hook) rather than noticing reuse at
     the next registration.
   - Threads: the enter-word rewrite is one word store, which fits the
     planned atomic rewrites under `#[threads(shared_code)]` (open
     question 4). Compiled code is thread-agnostic: thread-locals are
     patched as offsets from the thread pointer.
   - W^X under hardened runtimes (dual mapping instead of `mprotect`).
4. **Beyond copy-and-patch.** Loimi knows more than CPython's templates
   do: operand kinds say which fields name register slots. The compiler
   could therefore generate stencil variants that take or leave a slot's
   value in a fixed machine register, caching the most recently written
   register as CPython 3.14 caches the top of its stack. That would
   start on the memory-traffic layer of section 7, and is the next
   experiment if this tier is built. Measured in section 13: a third at
   best on mandelbrot, 1.5% slower over the suite as built.

## 9. Phase 2: MiniJS, Scheme, and no cost when off

The second phase asked whether the result holds beyond Mini, and
removed the prototype's worst costs.

**MiniJS and Scheme needed no change.** Both built with
`--jit-stencils` as they were, and passed all their tests with the JIT
forced on (threshold 1) in all three GC modes, before any compiler
change. All their ops get stencils (MiniJS 134, Scheme 88), none exits
to the interpreter for lack of one, and none is rejected. The vm
features Mini did not use were already handled by the generic design:
MiniJS's `-> never` helpers (`branch`, `call_slow`, `enter`) are shared
functions that stencils tail-call, exceptions are `raise` to `on raise`
(whose `jump` to the handler is ordinary dispatch), re-entry and
`reload` are the handlers' own code, and Scheme's escaping `call/cc` is
plain vm code. New golden tests `minijs_jit_stencils` and
`scheme_jit_stencils` run every script and conformance test that way,
as `mini_jit_stencils` does.

One thing failed silently: MiniJS's `SWITCHI` (`jump pc + 1 + i`) made
the JIT give up on functions with a `switch` (json: 2 of 22). Fixed in
the runtime (section 5, *Exits*).

**Changes** (compiler and runtime only):

- *No cost with the JIT off.* Counting copies of the handlers that jump
  (section 5) and helper code copied from the stencil object (section
  3). The handlers' IR in a JIT build is now byte-identical to a
  default build's, and so is their machine code, except a few bytes in
  some cold blocks.
- *Faster stencil builds.* Only functions that stencils reach are
  compiled in the stencil module.
- *Quickening.* After an invalidation a code object is compiled again
  after 1/8 of the threshold, and given up on after 32 compiles, not 8.
  deltablue gave up on 12 functions before (0.76 s → 0.74 s).

**Results.** Best of 5 (Scheme: 3), interleaved, same machine and
caveats as section 6. Geometric means of the time ratios; *JIT off* is
the `--jit-stencils` build without `LOIMI_JIT`.

| | benchmarks | JIT off / interp | JIT / interp | range | JIT against references |
|---|---:|---:|---:|---:|---|
| Mini | 14 | 1.00 (0.99–1.02) | **0.77** | 0.59–0.97 | LuaJIT `-joff` 0.65, LuaJIT 2.3× slower |
| MiniJS | 14 | 1.00 (0.99–1.02) | **0.85** | 0.74–1.02 | node `--jitless` 0.45, QuickJS 0.35, node 3.9× slower |
| Scheme | 41 | 1.00 (0.94–1.06) | **0.79** | 0.50–1.84 | Guile 3 (JIT) 0.80, Guile without JIT 0.37, Gauche 0.23, Chibi 0.05 |

- **JIT off is within noise** of a default build for all three (it was
  1.05 for Mini in phase 1).
- These runs predate the last two changes (pruned stencil module,
  helper jump tables). A best-of-3 re-run after them, without the
  references, gave 0.77, 0.86 and 0.77 with the JIT and 1.00 with it
  off.
- **Mini** improved a little over phase 1 (0.78 → 0.77): sieve 0.89 →
  0.87, closures 0.97 → 0.85. Helper calls from stencils no longer go
  through functions the interpreter had to keep out of line.
- **MiniJS** gains least, 15%. Its time is more in property lookup,
  allocation and strings (json 1.02, strings 0.97, raytrace 0.95) than
  in dispatch. Numeric and call-heavy code gains as in Mini (nbody and
  closures 0.74, awfy_micro 0.78). deltablue has 237 invalidations in
  301 compiles: its polymorphic sites quicken, and each `become` drops
  the whole code object's compiled code.
- **Scheme** gains 21%, most on calls and arithmetic loops (sum 0.50,
  ntakl 0.54, sumfp 0.52, puzzle 0.65), least on list and string
  programs. With the JIT it is faster than Guile 3's JIT by geometric
  mean (without it, Scheme takes 1.02× Guile's time). One outlier is unexplained: `string`
  (25 ms) takes 45 ms with the JIT, but only when exactly its three
  loop functions are compiled (threshold 20–60); with thresholds 1–10 or
  100+ it takes 24 ms. Its time is in copying large strings and the
  collector, not in compiled code.

**Build time.** Stencils now add 1.6 s to Mini's build (was 2.5 s),
2.3 s to MiniJS's (was 3.5 s) and 1.6 s to Scheme's (was 2.0 s).

**What remains** (in order of value):

1. *Quickening invalidates whole code objects.* Done in phase 3
   (section 10), but it was not what held MiniJS back.
2. *Hot/cold stencil parts.* Tried in phase 3 (section 10): no faster.
3. The limitations below, of which several vms per program, variants,
   and AArch64 are the ones a real interpreter would hit first.

### Limitations of the prototype

The prototype supports x86-64 Linux executables (non-PIE) with
tail-call dispatch only. It needs one vm per program, 32-bit code words
with the opcode in the low bits, one free opcode, and no variants.
Compiled code and registry records are never freed. The JIT region is
256 MiB at a fixed hint address within ±2 GiB of the program; a
relocation that does not fit fails that compile, and the code stays
interpreted.

### Reproducing

```sh
loimi build examples/mini/main.lm --release --jit-stencils -o mini-jit
LOIMI_JIT=1 LOIMI_JIT_STATS=1 ./mini-jit bench/mini/programs/fib.lua
```

`LOIMI_JIT_HOT=n` sets the threshold, `LOIMI_JIT_DEBUG=1` lists each
compiled code object, and `LOIMI_JIT_MAP=file` writes address ranges
for profilers. `LOIMI_JIT_RECOMPILE=n` stops recompiling a code object
after n compiles (rewrites then retarget instructions; for testing). At
build time, `LOIMI_JIT_REPORT=1` reports the stencils (and any rejected
ones), `LOIMI_KEEP_STENCILS=dir` keeps the stencil module and object,
and `LOIMI_JIT_SPLIT=1` splits stencils into hot and cold parts
(section 10).

## 10. Phase 3: rewrites of compiled code, hot and cold parts

Phase 2 named two things to try next: keep compiled code across
quickening (deltablue had 237 invalidations in 301 compiles), and let
fast paths fall through. Both are done. Neither made the JIT faster.

**Rewrites of compiled code.** A `become` used to restore every word of
the code object and drop its compiled code; the code was compiled again
after 1/8 of the threshold. Now `become` calls
`loimi_jit_become(word, opcode)` and `Vm.optimize`/`Vm.deopt` bracket
their rewrites with `loimi_jit_invalidate`/`loimi_jit_rewritten`. The
runtime picks, per rewritten instruction:

1. code it already has for the new word (the copy made at compile time,
   or one made earlier: a site going back and forth between members);
2. otherwise, compiling the whole code object again at once, up to 32
   compiles;
3. after that, an out-of-line copy of the new op's stencil for that
   instruction.

If none applies (out of memory or budget), it drops the code as before.
The alternatives considered:

- *Restore one word and recompile at the next safe point.* The hot
  counters are not a safe point for this: compiled-to-compiled calls do
  not count, so a function called only from compiled code would keep
  its stale code. Recompiling at the rewrite is safe, as it turns out
  (below), and simpler.
- *Re-patch the instruction's code in place.* Stencil copies differ in
  size, and a stencil's own branches may target its first bytes, so the
  old copy cannot be overwritten. The JIT puts a 5-byte nop in front of
  the code of each instruction whose word can change (ops that can
  `become`, supers and their first parts, computed by
  `jitgen::rewritable_ops`). Every way into that instruction's code goes
  through this *home*: direct jumps, falling through from the previous
  copy, and (via `loimi_jit_entries`) enter words. Retargeting writes a
  `jmp` there and updates the entry. A recompile also points each such
  home of the old code at the new code.
- *Compile family members together* so a `become` only changes which
  stencil runs. That is option 1 with all copies made up front. It
  would cost code for members that never run.

Why this is safe, including re-entry and invalidation while frames of
the code are live: compiled code is never freed, and only the 5-byte
homes of old code are written, which no return address points into.
A stencil that calls `loimi_jit_become` (or a helper that re-enters the
interpreter, which rewrites) returns into its unchanged body and
finishes as the old member, then continues at a home, which leads to
the new code. GC is not involved (compiled code holds no references).
deltablue has a store site that alternates between `SETPROP_ADD` and
`SETPROP` on every miss, 150,000 times in one run. With two `mprotect`
calls per retarget, deltablue took 1.85× as long. So the JIT region is
now one memory object mapped twice (executable, and writable at another
address), and patching needs no system call. Writing code the processor
is running still costs a pipeline flush, so a site retargeted more than
16 times is left to the interpreter (its home jumps to the exit
stencil).

Correctness: the golden JIT tests (all scripts and conformance tests,
three GC modes, threshold 1) pass as before, and pass again with
`LOIMI_JIT_RECOMPILE=1`, which turns off recompiles so that every
rewrite takes the retarget, out-of-line copy and interpreter paths.
Over MiniJS's tests that is 1,256 recompiles normally, and 1,402
retargets (1,188 new copies, 212 reuses, 2 sites left to the
interpreter) with recompiles off; no run dropped its code.

**Hot and cold stencil parts.** With `LOIMI_JIT_SPLIT=1` at build time,
stencils get a profile entry count, and clang's
`-fsplit-machine-functions` moves blocks it thinks never run to a
`.text.split` section. `elf.rs` joins the two parts (a new patch kind
for references between them), and the runtime lays out each code
object's cold parts after all its fast paths. Only about a third of the
stencils are split (36 of Mini's 112). The other stencils' cold blocks
are small tail calls to cold functions, which the splitter leaves in
place. A split stencil's fast path does end in its `jmp` to the next
instruction, which the JIT drops. It is off by default because it
measured no faster: 1.01× the JIT time without it for both Mini and
MiniJS. Mini's nbody was 13% slower, apparently because some blocks
moved out of line are not rare. A taken direct `jmp` to the next
instruction seems to cost little.

**Results.** Best of 5 (Scheme: 3), interleaved, all built with
`--align-functions=64` (the noise between builds of the same code is
otherwise several percent). *Before*: phase 2; *after*: this phase.
Geometric means of time against the interpreter (default build):

| | JIT off, before | JIT off, after | JIT, before | JIT, after |
|---|---:|---:|---:|---:|
| Mini (14) | 1.00 | 1.00 | 0.77 | 0.78 |
| MiniJS (14) | 1.00 | 1.00 | 0.85 | 0.86 |
| Scheme (41) | 1.00 | 1.00 | 0.77 | 0.78 |

- **No gain.** deltablue never drops its code now (579 recompiles, 351
  retargets to existing code, 88 out-of-line copies, 6 sites left to
  the interpreter), against 237 invalidations before, and runs in the
  same time (0.88 s against 0.87 s). The invalidations cost little: a
  dropped function ran in the interpreter for about 7 calls before it
  was compiled again, which is short next to the run. Recompiling costs
  little too (deltablue: 647 compiles in 4 ms). What limits
  MiniJS is still the work inside handlers: property lookup, allocation
  and strings (section 9).
- **It costs about 1%** with the JIT on, in all three (Mini 0.772 →
  0.777, MiniJS 0.847 → 0.861, Scheme 0.765 → 0.776). That is within
  the noise of one run, but every run was on the slow side, and Scheme
  barely quickens, so the cost is static. Suspects, not separated: the
  nop in front of every rewritable instruction, and the doubly mapped
  region, which cannot use transparent huge pages (the old private
  mapping could). Other code is unchanged.
- **What it buys** is robustness rather than speed. Old code reaches the
  new member at the rewritten instruction, so members need not be
  complete (section 8). Sites that keep changing no longer cost whole
  recompiles, and no function is given up on because of quickening.
- Default builds: the IR of all three interpreters is byte-identical to
  phase 2's (std paths normalized). JIT off: within noise.

## 11. Sketch: immutable operand kinds (implemented in section 12)

Section 7 left `k[it]` as a load. This sketches what the language and
the JIT would need to patch constants in.

**Syntax.** An operand kind whose place cannot change once the code is
verified:

```
operand kst = const k[it] verify (it as u64) < proto.consts.data.len;
```

**What the checker enforces.**

- The place's base (`k`) is a state register that is fixed per code
  object: it is set only from the code object's own data (MiniJS's
  `CALL` sets `k = p.consts.data.ptr`, and a return restores `fp.k`).
  The checker already knows which state an operand kind reads.
- No handler, helper or exported function stores through a `const`
  place. The pool is ordinary memory, so this is a promise of the vm
  author, checked as far as the types allow, like `verify` clauses.
- `const` adds nothing to the interpreter: the handlers are unchanged.

**What the JIT does with it.**

1. *Values as holes.* A `const` operand read becomes a 64-bit hole
   `__jit.c.OFF.SHIFT.BITS` (an `R_X86_64_64` immediate, `movabs`),
   patched with the constant's bits at compile time. The runtime needs
   the pool's address per code object: `Vm.build_verified` passes it to
   `loimi_jit_register`, evaluated from the operand kind's base in the
   verify context. `LOADK` becomes a store of an immediate, and
   `ADDK_F` adds an immediate with no load.
2. *Types as stencil variants.* A hole's value is unknown when clang
   compiles the stencil, so the type check on it stays. To fold it, the
   generator emits one variant per tag the handler distinguishes (the
   arms of the `match` or `if let` on that operand), each with the tag
   assumed (`llvm.assume`), for example `ADDK_F.k=Num`. The runtime
   picks the variant from the constant's tag. Variants are opt-in per
   op or per operand kind, to bound the number of stencils.
3. *Heap constants.* A pointer patched into code keeps its object
   alive only while the pool does. That holds as long as compiled code
   is dropped with its code object (the collector hook of section 8),
   and the collector does not move objects. A moving collector would
   need relocation records for the patched pointers, or the load kept
   for pointer constants.

**Expected gain.** Constant operands sit in hot loops (mandelbrot's
`MULK`, `ADDK` and `JGTKN`; Scheme's literal operands). Each loses a
load and, with variants, a tag check: a few percent where they are
hot, by analogy with section 7, not measured. It is also the first
step towards type specialization, which is where the remaining gap to
LuaJIT is (section 7).

## 12. Phase 4: constant operands patched in

Phase 4 implemented section 11 and measured it. **It works and costs
nothing, but it gains nothing measurable**: 0.3–0.7% on Mini, 0.2% on
MiniJS, none on Scheme (1.003), all within noise.

**Language.** An operand kind takes a `const` clause, beside `verify` and
`display` (`language.md` §3.2):

```
operand kst = k[it] const proto.consts.data.ptr verify (it as u64) < proto.consts.data.len;
```

The body must be `S[it]` for a pointer state register `S`. The clause is
an expression of the verify context, of `S`'s type: the value `S` has
while code verified against that context runs. That part is a promise of
the vm author, like a `verify` clause. The checker enforces the rest
within the vm: no handler, vm function, cold block or `on raise` may
write through `S` (the operand itself, `k[i] = x`, `*(k + i) = x`) or
take an address in it (`&k[i]`). A vm has at most one pool. Mini,
MiniJS and Scheme mark their `kst` kinds `const`; nothing else in them
changed. The tests are `tests/run/vm_const_operand.lm` and five
`tests/fail/vm_const_*.lm`.

**Interpreter.** `const` adds nothing to the handlers. A default build of
an interpreter that opts in has the same IR as before, plus one unused
function (`Vm__const_pool`, which type-checks the clause); LLVM drops it.
Interpreters that do not opt in have byte-identical IR (std paths and
panic strings normalized).

**JIT.** `Vm.build_verified` passes the pool to `loimi_jit_register`.

1. *Values.* In a stencil, a read of a const operand (`k[c]`, and also
   `*((k + c.raw) as *f64)`, which Mini's float members use) of a 64-bit
   type becomes the hole `__jit.c.OFF.SHIFT.BITS.u`, an `R_X86_64_64`
   immediate patched with the constant's bits. The hole is read with
   `movabs` in inline asm: as a plain symbol address, clang 19
   sometimes stored it with a sign-extended 32-bit immediate
   (`movq $hole, mem`) whatever its `!absolute_symbol` range, and those
   compiles failed. `LOADK` is now `movabs` and a store; `MULK_F` is
   `movabs`, `movq` to xmm, `mulsd` with the register operand.
2. *Variants.* An op with one const operand of a one-word `value` type
   gets a stencil per variant its handler's patterns name (`ADDK.v.Int`,
   `ADDK.v.Num`), from the same handler. In a variant, the constant is
   rebuilt as `(bits & payload mask) | tag`, so LLVM knows its tag bits
   and folds the tag tests on it; for the unboxed float of a nanbox it is
   `llvm.assume(bits < limit)`. The runtime picks a variant from the
   constant (`loimi_jit_variants`: mask and value per variant). Mini gets
   50 variants (163 stencils), MiniJS 48, Scheme none (its only const
   operand is `LOADK`'s). `LOIMI_JIT_VARIANTS=0` at build time turns them
   off. `ADDK.v.Int` loses the test of the constant: 14 instructions to
   the `jmp` instead of 17. What it cannot fold are tests written as bit
   tricks: `JGTKN`'s integer test `(a ^ bias | b ^ bias) < 1 << 48`
   stays in `JGTKN.v.Num`, because LLVM does not carry `b < limit`
   through the xor.

Correctness: the golden JIT tests (all scripts and conformance tests,
three GC modes, threshold 1, with and without recompiles) pass for all
three, with the constants patched in and variants chosen.

**Results.** Best of 5 (Scheme: 3), interleaved, `--align-functions=64`,
same machine (load average about 3). Geometric means:

| | JIT before / interp | JIT, constants / JIT before | JIT, constants and variants / JIT before | JIT off / interp |
|---|---:|---:|---:|---:|
| Mini (14) | 0.770 | 0.997 (0.993 in a second run) | 0.996 | 0.998 |
| MiniJS (14) | 0.858 | | 0.998 | 0.997 |
| Scheme (41) | 0.778 | 1.003 (only `LOADK` is const) | (no variants) | not measured |

- Per benchmark, Mini moves by up to ±5% with variants (mandelbrot and
  hash_strings 0.95, interp and oo_methods 1.03), about the noise of
  one run. The JIT-off column is the opted-in build without
  `LOIMI_JIT`.
- **Why so little.** A constant load is an L1 hit off a register that
  is already live (`k`), and out-of-order execution hides its latency;
  the `movabs` that replaces it is one instruction too, and cannot be
  folded into an arithmetic operand as the load was. A type check of a
  constant is a well-predicted branch. Neither is on the critical path:
  section 7's limits are the register file in memory, the checks of
  *register* operands, and calls. Variants also make code larger (Mini's
  stencils grow from 22 KB to 29 KB).
- **Phase 3's 1% (task 0).** Neither suspect is measurable on Mini. With
  the JIT on, against the shipped design: no rewrite prefixes 0.997, a
  private anonymous mapping (transparent huge pages allowed) 1.001, both
  0.989 (best of 5, 14 benchmarks). Left as is: the prefixes are what
  makes rewrites cheap, and the double mapping is what makes them need
  no system call.

**Recommendation.** Keep `const` operands: they are a small, checked
language feature with no cost, and the place type specialization would
start from. But patching constants is not a speedup for a baseline JIT
of an interpreter that is already tuned. The remaining gap to LuaJIT
needs values in machine registers and types known across instructions
(section 8, item 4), not cheaper constants.

## 13. Experiment: guest registers in machine registers

Section 8 (item 4) named this as the next experiment: keep register-file
values in machine registers across instructions in compiled code. This
section measures how much that can buy. **Answer: up to about a third
on a loop that lives in registers, but little over the suite, and a
template JIT gets almost none of it.** The one-register cache built here
is 1.5% *slower* by geometric mean.

**Upper bound, by hand.** The JIT's compiled code for mandelbrot's inner
loop (words 31–44 of the main chunk, five super stencils, `JGTKN`,
`FORLOOP`) was dumped from the running program and run on its own in a
harness, against hand-written versions of the same loop. Every version
keeps the stencils' structure (the same type checks, `pc` updates, one
taken `jmp` per stencil, cold code between them) unless noted. Best of
3, the same 1300×1300 picture, Skylake (i7-6700):

| inner loop | seconds | of the JIT's |
|---|---:|---:|
| the JIT's code, copied byte for byte | 0.42 | 1.00 |
| all 11 guest registers in host registers, spilled at exits | 0.285 | 0.68 |
| the same, write-through (results also stored) | 0.32 | 0.76 |
| only the previous op's result in a register, write-through | 0.33 | 0.79 |
| registers, and no type checks, `pc` updates or jumps | 0.235 | 0.56 |

The harness's first row matches the benchmark (0.41–0.45 s; mandelbrot
spends 97% of its samples in this loop). The loop is a chain of
dependent float operations, and every hop through the register file is
a store-to-load forwarding (4–5 cycles on this machine). Most of the
gain comes from the hops between consecutive operations, so a cache of
one register, CPython 3.14-style, would get most of the bound *on this
loop*.

**Over the 14 benchmarks: a latency probe.** With
`LOIMI_JIT_REGDELAY=n` at build time, every register-file read in a
stencil passes through n dependent no-op instructions (`imul $1` or
`addsd -0.0`, 3–4 cycles each). The slowdown says how much of the run
time waits on register-file loads. Best of 5, interleaved, JIT on:

| | binary_trees | closures | fannkuch | fib | hash_strings | interp | mandelbrot | nbody | oo_methods | queens | sieve | sort | spectral_norm | string_build |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| slowdown, n = 1 | 2% | 8% | 5% | 3% | 23% | 9% | 39% | 27% | 10% | 13% | 14% | 16% | 43% | 1% |

Calibrated on mandelbrot (where the bound is measured: 0.78 of its
slowdown with all registers cached, 0.52 with one), this projects at
most 12% by geometric mean with every value in registers, and 8% with
one cached register. Both are generous: they assume every delayed read
could be served from a register. Much of spectral_norm's and
hash_strings' sensitivity is arguments and results crossing calls, which
go through the callee's frame in memory and would need a register
calling convention, not a cache. A SIGPROF profile agrees: only
mandelbrot (97%), spectral_norm, nbody, queens and interp (53–62%) spend
more than a third of their time in stencils whose operands are all
registers and constants; the rest is calls, table access, allocation
and the runtime.

**The one-register cache, built.** With `LOIMI_JIT_CACHE=1` at build
time (Mini-shaped: it uses the operand kind named `reg` and the operand
named `dst`):

- Every stencil that falls through (`__jit.n.SIZE`) loads its `dst`
  register and leaves it in xmm15 (inline asm, right before the `jmp`).
  The register file still holds it (write-through), so exits, cold
  paths, GC, re-entry and rewrites need nothing.
- Each op gets a *cache variant* per register operand of its first part
  (204 for Mini; 367 stencils in all, 76 KB). Its prologue stores xmm15
  into that register's slot, which already holds that value; LLVM then
  forwards it to the op's loads of that register wherever nothing may
  have written in between. That keeps it correct without tracking
  stores or calls in the generator.
- The runtime uses a variant only where *every* continuation into the
  instruction is a fall-through that leaves that register in xmm15 (no
  label, `redo` or other jump lands there). Such an instruction gets no
  enter word: the interpreter never enters compiled code there. A
  rewritten predecessor keeps the invariant: family members share `dst`,
  and a predecessor sent to the interpreter dispatches to a word that is
  not an enter word.

The golden test `mini_jit_stencils` passes with it on (every script
and conformance test, three GC modes, threshold 1, with and without
recompiles). In mandelbrot 27 instructions take a cached register. A check of the object shows that every variant reads xmm15 before
anything writes it (the four that do not read it only write their
operand). Results, best of 5, interleaved, against the same build
without it:

| | mandelbrot | fannkuch | fib, hash_strings | the other 10 | geometric mean |
|---|---:|---:|---:|---:|---:|
| cache / no cache | 0.91 | 0.97 | 1.00 | 1.01–1.08 | **1.015** |

Why it falls so far short of the bound:

- In mandelbrot's loop only 2 of the 5 stencil boundaries hit the
  cache. The other three hops read a register written two or more ops
  earlier, or reach `JGTKN`, whose const variant is chosen instead
  (a variant cannot be both). The hand-written one-slot loop also
  forwarded *within* supers. The stencils do not: at the second part,
  every match arm of the first part has stored `dst`, and LLVM reloads
  it instead of merging the values. Fixing that is independent of any
  cache.
- Integers pay a round trip `movq` to and from xmm15 (2 + 2 cycles on
  Skylake), about what the store-to-load forwarding cost. Integer code
  (spectral_norm's `A`, sieve, queens) gains nothing and pays the extra
  instructions.
- Every stencil pays the export (a `movq` or `movapd`) and every variant
  the prologue store, used or not. Instructions that use the cache lose
  their enter words, so after an exit the interpreter runs them.

**Recommendation: stop the template-JIT line here.** Dispatch and
operand decoding were the part a template JIT could remove, and phase 1
removed them (0.77 of the interpreter's time). Phases 3, 4 and this one
attacked what remains with template techniques and gained nothing. The
register-file traffic is worth real time only in loops that keep their
values in registers (mandelbrot, a third at best), and getting it needs
register allocation over a loop or trace, guards hoisted out of it, and
arguments passed in registers: an optimizing tier with deoptimization
metadata, which the shared-frame design (section 5) exists to avoid.
Keep the JIT as it is, an opt-in tier at about 0.8 of the interpreter's
time. If the speed matters later, start an optimizing tier over traces
or loops, not more stencil variants. `LOIMI_JIT_REGDELAY` stays as a
probe; the cache stays behind `LOIMI_JIT_CACHE` for reproduction.
