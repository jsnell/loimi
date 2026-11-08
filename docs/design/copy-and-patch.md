# Design note: a copy-and-patch JIT from `vm` declarations

`loimi build --jit-stencils` builds a baseline JIT into an interpreter,
generated from its `vm` declaration; `LOIMI_JIT=1` turns it on at run
time. Its stencils are the vm's own handlers, compiled by clang with
holes for operand fields and continuations. It needs no stencil code
written by hand and no change to the interpreter's source: Mini, MiniJS,
Scheme and Prolog build with it as they are (they mark their constant
operands `const`, section 12). Every op, family member and super gets a
stencil, and every script and conformance test of the four passes with
the JIT forced on at the first call or backward jump.

It takes 0.76–0.83 of the interpreter's time on Mini, MiniJS and Scheme
(current numbers in [results.md](../results.md)), and about 0.98 on
Prolog (section 14). It removes dispatch and operand decoding. It does
not remove the loads and stores through the register file, the type
checks, or the call overhead (section 7); the trace tier
([trace-tier.md](trace-tier.md)) builds on these stencils for that.
Sections 1–5 describe the design, 6–7 its results, 8 its limitations,
10 and 12 rewrites of compiled code and constant operands, 13 an
experiment with guest registers in machine registers, 14 Prolog, and 15
the placement of compiled code.

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
interpreter**. (A cold path's `next` in a super's part before the last
continues in the interpreter's resume function for the rest of the
super, as in the interpreter; see language.md, section 7.3.) Inline `cold { }` blocks (unit-typed, such as
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
   the hole declarations are appended. (Helpers are not shared with the
   interpreter by address: taking their addresses changed how LLVM
   inlined them in the interpreter, which cost a JIT build 5% with the
   JIT off.)
2. `clang -O3 -fno-pic -ffunction-sections -fdata-sections -c`: one
   section per stencil, helper, and constant.
3. **The ELF reader** (`src/elf.rs`, no crates) walks the
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

Mini's 113 stencils were 21 KB of code, plus 26 helpers (17 KB), when
first measured (2026-11-02). Build times are in section 9.

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

`src/jit_rt.c` (C, compiled into the program):

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
  byte-identical to a default build's. (Testing a flag before counting,
  in every handler, cost 5%.)
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
  head may still run in the interpreter. Host code never sees enter
  words: in a `--jit-stencils` build the generated readers of code words
  (`Vm.disasm`, `op_name`, `op_size`, `label`, `set_label`, the field
  getters, operand metadata, `Vm.verify`) first map a word through
  `Vm.real_word`, which compares the opcode with `ENTER` and, for an
  enter word, loads the real word from the runtime's table
  (`loimi_jit_words`, by entry index, kept by the runtime wherever it
  writes or restores an enter word). The load is inline, not a call, so
  handlers may read other instructions' words too: an op that decodes
  the word at a computed pc (a jump table's JMP) gets the real word in
  the interpreter and in its stencil.
- **Exits.** Compiled code leaves through the dispatch table: dynamic
  jumps, cold paths, and instructions without a stencil (their slot holds
  the exit stencil, `musttail tbl[opcode(*pc)]`). A `jump` to a computed
  guest pc that is none of the op's labels (Mini's SWITCH, MiniJS's
  SWITCHI) tests the target word first: an enter word goes straight to
  its compiled code, anything else to the interpreter at that pc. A continuation that
  is not an instruction goes to the shared exit copy. It is never taken:
  for example, MiniJS's `SWITCHI` jumps to `pc + 1 + i`, which the
  stencil compares with `pc + field` for each signed field, and a field
  that is not a label gives a target outside the code. The interpreter then
  runs until it dispatches to an enter word. An exit costs one or two
  indirect jumps, about what the interpreter pays per instruction anyway.

No frame map, deoptimization metadata or state reconstruction is
needed, because compiled code and the interpreter share one frame
layout and one register assignment. A baseline JIT whose stencils kept
values in machine registers across instructions would need all three.

## 6. Results

First measurement on Mini (2026-11-02). Seconds, best of 5, interleaved,
on the shared 8-core machine (about ±5% noise). *interpreter*: Mini's
default build. *JIT off*: the `--jit-stencils` build without
`LOIMI_JIT` (this 1.05 is 1.00 since section 9). *JIT*: the same with
`LOIMI_JIT=1` (threshold 50). LuaJIT 2.1 as in `bench/mini/RESULTS.md`.

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
  than LuaJIT's interpreter on all 14 benchmarks. LuaJIT's trace
  compiler is 2.3× faster by geometric mean: 2–4× on numeric loops and
  calls, and 10× on spectral_norm, where it inlines the `A(i, j)` call.
  The JIT beats it only where allocation dominates (binary_trees,
  closures).
- **There is little gain where the work is not in handlers.**
  string_build spends its time building and interning strings, closures
  in `CLOSURE`'s allocation and the GC, and sieve partly in table growth
  (16% of its profile).
- **Default builds are unaffected.** Without `--jit-stencils` the IR is
  byte-identical to a build of a compiler without the JIT.

**Compile time** (`LOIMI_JIT_STATS=1`, which includes the first
compile's `mmap`): 7–30 µs per function, or 250–700 ns per bytecode
instruction. interp compiles 19 functions (931 instructions) in 0.27 ms
in total, cheap enough to compile at the first call. Compiled code is
150–200 bytes per instruction (188 KB for interp's 931), against 4 bytes
of bytecode: the stencils carry their type-check fallbacks inline,
after the fast path.

**Correctness.** The golden tests `mini_jit_stencils`,
`minijs_jit_stencils`, `scheme_jit_stencils` and `prolog_jit_stencils`
run every script and conformance test of their interpreter (normally,
under GC stress, and with generational GC verification) with
`LOIMI_JIT_HOT=1`, so every function is compiled at its first call or
loop, before quickening, and again after each rewrite.

## 7. Where the time goes

SIGPROF profiles, with compiled code attributed to stencils
(`LOIMI_JIT_MAP`):

- **mandelbrot**: all samples are in compiled code: `JGTKN` 23%,
  `MUL_F` 20%, `ADD_F` 19%, `FORLOOP` 14%. A float op is about 9
  instructions (two loads from `regs`, the operation, the NaN test, the
  store, `pc += 1`, a `jmp`), against about 16 in its handler. Every
  value goes through the register file in memory, so
  `zi = 2.0 * zr * zi + ci` is a chain of store-to-load forwardings,
  and every op checks its operand types again. LuaJIT's trace keeps
  `zr`, `zi`, `zrq` and `ziq` in xmm registers and has no type checks
  inside the loop.
- **fib**: `CALL` 42%, `RET1` 18%, then the compare, `ADDI` and `ADD`.
  Call and return do the same work as in the interpreter (frame push,
  clearing the callee's registers, stack checks, frame pop); only the
  dispatch around them is gone.
- **sieve**: `SETINDEXK` 36%, `FORLOOP` 24%, `GETINDEX` 10%, and 16% in
  the table runtime.

Dispatch and operand decoding were 20–40% of the time in handler-bound
code of an already tuned interpreter, and copy-and-patch removes most
of that. The next layer (memory traffic through the register file,
repeated type checks, call overhead) needs register allocation and
type specialization across instructions. Sections 10, 12 and 13 attacked
it with template techniques and gained nothing; the trace tier
([trace-tier.md](trace-tier.md)) does it over loops.

## 8. Limitations

- The JIT is x86-64 Linux only, for executables built without PIE, with
  tail-call dispatch. The ELF reader handles x86-64 relocations only
  (no AArch64, Mach-O, COFF or position-independent code).
- One vm per program: `--jit-stencils` compiles the program's only
  tail-call vm, which needs 32-bit code words with the opcode in the low
  bits, one free opcode (for enter words), and no variants.
- Compiled code and registry records are never freed. The JIT region is
  256 MiB, mapped at a fixed address within ±2 GiB of the program; a
  relocation that does not fit fails that compile, and the code stays
  interpreted.

## 9. MiniJS and Scheme

MiniJS and Scheme built with `--jit-stencils` as they were and passed
all their tests with the JIT forced on, in all three GC modes, before
any compiler change. Every op gets a stencil (MiniJS 134, Scheme 88).
The vm features Mini does not use were already covered by the generic
design: MiniJS's `-> never` helpers (`branch`, `call_slow`, `enter`)
are shared functions that stencils tail-call, exceptions are `raise` to
`on raise` (whose `jump` to the handler is ordinary dispatch), re-entry
and `reload` are the handlers' own code, and Scheme's escaping
`call/cc` is plain vm code. MiniJS's `SWITCHI` (`jump pc + 1 + i`) needed
the exit through a computed guest pc (section 5, *Exits*).

The same phase made the JIT cost nothing when it is off (counting
copies, section 5; helpers copied from the stencil object, section 3),
compiled only the functions stencils reach, and set the recompile
policy: after a rewrite that drops compiled code, a code object is
compiled again after 1/8 of the threshold, and given up on after 32
compiles.

Results (2026-11-02), best of 5 (Scheme: 3), interleaved; geometric
means of the time ratios:

| | benchmarks | JIT off / interp | JIT / interp | range | JIT against references |
|---|---:|---:|---:|---:|---|
| Mini | 14 | 1.00 (0.99–1.02) | **0.77** | 0.59–0.97 | LuaJIT `-joff` 0.65, LuaJIT 2.3× slower |
| MiniJS | 14 | 1.00 (0.99–1.02) | **0.85** | 0.74–1.02 | node `--jitless` 0.45, QuickJS 0.35, node 3.9× slower |
| Scheme | 41 | 1.00 (0.94–1.06) | **0.79** | 0.50–1.84 | Guile 3 (JIT) 0.80, Guile without JIT 0.37, Gauche 0.23, Chibi 0.05 |

- **MiniJS** gains least. Its time is more in property lookup,
  allocation and strings (json 1.02, strings 0.97, raytrace 0.95) than
  in dispatch. Numeric and call-heavy code gains as in Mini (nbody and
  closures 0.74, awfy_micro 0.78).
- **Scheme** gains most on calls and arithmetic loops (sum 0.50, ntakl
  0.54, sumfp 0.52, puzzle 0.65), least on list and string programs.
  With the JIT it is faster than Guile 3's JIT by geometric mean.

**Build time.** Stencils add 1.6 s to Mini's build, 2.3 s to MiniJS's
and 1.6 s to Scheme's.

## 10. Rewrites of compiled code

`become` calls `loimi_jit_become(word, opcode)`, and
`Vm.optimize`/`Vm.deopt` bracket their rewrites with
`loimi_jit_invalidate`/`loimi_jit_rewritten`. If the code object is
compiled, the runtime picks, per rewritten instruction:

1. code it already has for the new word (the copy made at compile time,
   or one made earlier: a site going back and forth between members);
2. otherwise, compiling the whole code object again at once, up to 32
   compiles;
3. after that, an out-of-line copy of the new op's stencil for that
   instruction.

If none applies (out of memory), it drops the code. Stencil copies
differ in size, and a stencil's own branches may target its first
bytes, so old code is never overwritten. Instead each instruction whose
word can change (ops that can `become`, supers and their first parts:
`jitgen::rewritable_ops`) gets a 5-byte nop in front of its code, its
*home*. Every way into that instruction's code goes through the home:
direct jumps, falling through from the previous copy, and enter words
(via `loimi_jit_entries`). Retargeting writes a `jmp` there and updates
the entry; a recompile points each home of the old code at the new code.

This is safe while frames of the old code are live, including re-entry:
compiled code is never freed, and only the 5-byte homes are written,
which no return address points into. A stencil that calls
`loimi_jit_become` (or a helper that re-enters the interpreter, which
rewrites) returns into its unchanged body, finishes as the old member,
and continues at a home, which leads to the new code. Members of a
family therefore need not be complete. GC is not involved (compiled
code holds no references).

The JIT region is one `memfd` mapped twice (executable, and writable at
another address), so patching needs no system call and no page is
writable and executable at once. With two `mprotect` calls per
retarget, deltablue (a store site that alternates between
`SETPROP_ADD` and `SETPROP` 150,000 times) took 1.85× as long. Writing
code the processor is running still costs a pipeline flush, so a site
retargeted more than 16 times is left to the interpreter (its home
jumps to the exit stencil).

`LOIMI_JIT_RECOMPILE=1` turns recompiles off, so that every rewrite
takes the retarget, out-of-line copy and interpreter paths; the golden
JIT tests pass both ways. Over MiniJS's tests that is 1,256 recompiles
normally, and 1,402 retargets (1,188 new copies, 212 reuses, 2 sites
left to the interpreter) with recompiles off.

Measured (2026-11-02, against dropping and recompiling the whole code
object on every rewrite): no faster (Mini 0.77 → 0.78, MiniJS 0.85 →
0.86, Scheme 0.77 → 0.78 of the interpreter). deltablue now never drops
its code, but its invalidations had cost little. What it buys is
robustness: sites that keep changing no longer cost whole recompiles,
and no function is given up on because of quickening. Neither the
prefixes nor the double mapping (which rules out transparent huge
pages) is measurable on Mini: 0.997 without prefixes, 1.001 with a
private mapping.

**Hot and cold stencil parts.** `LOIMI_JIT_SPLIT=1` at build time gives
stencils a profile entry count, so that clang's
`-fsplit-machine-functions` moves blocks it thinks never run to
`.text.split`; `elf.rs` joins the parts and the runtime lays out each
code object's cold parts after its fast paths, so that fast paths fall
through. Only 36 of Mini's 112 stencils split. It is off by default: 1.01×
the JIT time without it on Mini and MiniJS, and Mini's nbody 13% slower.

## 11. Environment variables

At run time (`src/jit_rt.c`): `LOIMI_JIT=1` turns the JIT on,
`LOIMI_JIT_HOT=n` sets the hot threshold (default 50),
`LOIMI_JIT_STATS=1` prints compile times and rewrite counts,
`LOIMI_JIT_DEBUG=1` lists each compiled code object,
`LOIMI_JIT_MAP=file` writes address ranges of compiled code for
profilers, and `LOIMI_JIT_RECOMPILE=n` stops recompiling a code object
after n compiles (section 10; for testing).

At build time: `LOIMI_JIT_REPORT=1` reports the stencils (and any
rejected ones), `LOIMI_KEEP_STENCILS=dir` keeps the stencil module and
object, `LOIMI_JIT_VARIANTS=0` turns off type variants (section 12),
`LOIMI_JIT_SPLIT=1` splits stencils into hot and cold parts (section
10), and `LOIMI_JIT_REGDELAY=n` is a latency probe (section 13).

```sh
loimi build examples/mini/main.lm --release --jit-stencils -o mini-jit
LOIMI_JIT=1 LOIMI_JIT_STATS=1 ./mini-jit bench/mini/programs/fib.lua
```

## 12. Constant operands patched in

An operand kind takes a `const` clause, beside `verify` and `display`
([language.md](../language.md) §3.2):

```
operand kst = k[it] const proto.consts.data.ptr verify (it as u64) < proto.consts.data.len;
```

The clause promises that the pool cannot change once the code is
verified; the checker enforces that no handler, vm function, cold block
or `on raise` writes through the pool's state register or takes an
address in it. It adds nothing to the interpreter. `Vm.build_verified`
passes the pool to `loimi_jit_register`.

1. *Values.* In a stencil, a read of a const operand of a 64-bit type
   (`k[c]`, and `f64.from_bits(c as u64)`, which the checker turns into
   a load from the pool) becomes the hole `__jit.c.OFF.SHIFT.BITS.u`, an
   `R_X86_64_64` immediate patched with the constant's bits. It is read
   with `movabs` in inline asm (as a plain symbol address, clang 19
   sometimes stored it with a sign-extended 32-bit immediate, whatever
   its `!absolute_symbol` range). `LOADK` is a `movabs` and a store.
2. *Variants.* An op with one const operand of a one-word `value` type
   gets a stencil per variant its handler's patterns name (`ADDK.v.Int`,
   `ADDK.v.Num`). In a variant the constant is rebuilt as
   `(bits & payload mask) | tag` (for the unboxed float of a nanbox,
   `llvm.assume(bits < limit)`), so LLVM folds the tag tests on it. The
   runtime picks a variant from the constant (`loimi_jit_variants`).
   Mini gets 50 variants, MiniJS 48. Tests written as bit tricks
   (`JGTKN`'s `(a ^ bias | b ^ bias) < 1 << 48`) do not fold.

Measured (2026-11-02): within noise. JIT with constants and variants
against without: Mini 0.996, MiniJS 0.998, Scheme 1.003 (only `LOADK`
is const). A constant load is an L1 hit off a live register, a type
check of a constant is a well-predicted branch, and neither is on the
critical path; variants grow Mini's stencils from 22 KB to 29 KB. The
trace tier uses the same holes to fold constants.

## 13. Guest registers in machine registers (experiment)

How much keeping register-file values in machine registers across
instructions could buy, measured 2026-11-02; nothing of it is in the
JIT, which led to the trace tier.

- **Upper bound, by hand.** Mandelbrot's inner loop (97% of its
  samples), copied from the JIT and run in a harness: 0.42 s as
  compiled; 0.68 of that with all 11 guest registers in host
  registers; 0.79 with only the previous op's result in a register
  (write-through); 0.56 with registers and no type checks, `pc` updates
  or jumps.
- **Over the 14 benchmarks: a latency probe.** With
  `LOIMI_JIT_REGDELAY=n` at build time, every register-file read in a
  stencil passes through n dependent no-op instructions. Slowdowns at
  n = 1 range from 1% (string_build) to 43% (spectral_norm); calibrated
  on mandelbrot, that projects at most 12% by geometric mean with every
  value in registers and 8% with one cached register, and much of it is
  arguments crossing calls, which a cache does not help.
- **A one-register cache, built** (branch `jit-regcache-experiment`):
  every falling-through stencil leaves its `dst` register in xmm15, and
  cache variants of each op (367 stencils for Mini) read it. 1.015× the
  time by geometric mean (mandelbrot 0.91, the other 10 1.01–1.08):
  only 2 of mandelbrot's 5 stencil boundaries hit, integers pay a
  `movq` round trip, and every stencil pays the export.

## 14. Prolog (2026-11-04)

The Prolog interpreter (`examples/prolog`, a WAM) builds with
`--jit-stencils` unchanged; every op has a stencil (116, 34 KB). The
JIT build first took 1.11× the interpreter's time over 29 of the 30
benchmarks, and 48× on sieve. Two causes, both fixed in the JIT:

- **Clauses entered through resume points never got hot.** Try chains
  and guard chains enter a clause with `jump cache.code`, a
  `Wam.Resume`. A jump through a `Vm.Resume` is its own kind
  (`JumpKind::Resume`) and counts like a call, in the counting copies
  and, on the not-compiled path, in stencils (times10: 21% → 91% of
  samples in compiled code). Mini's coroutine resume is the only other
  user; its JIT times did not change.
- **The registry is a treap.** Prolog verifies a code object per
  asserted clause and never frees them, and the collector reuses
  addresses, so registrations land anywhere: sieve registers 700,000
  objects, and a sorted array kept by `memmove` took 85% of its time.
  The treap takes one descent to find an overlapping (freed) record and
  one to insert; lookups try the last record found or registered first.
  `LOIMI_JIT_STATS` prints the objects registered.

Over the 29, the JIT build takes **0.98** of the interpreter's time
(unify 0.83, tak 0.83, crypt 0.86 .. prover 1.16, mu 1.16); sieve 1.30.
The JIT does little for Prolog: its ops do much more work per dispatch
(unification, binding with trailing, allocation, first-argument
indexing), and `backtrack` and PROCEED jump through one indirect jump
each to many more targets in compiled code (one per chain position or
return point) than in the interpreter (a few handlers). Aligning
compiled instructions to 16, 32 or 64 bytes changed nothing (0.996,
1.001, 1.008 over 12 programs).

## 15. Placement of compiled code: the Scheme puzzle/array1 regression (2026-11-07)

The 2026-11-07 run found Scheme with the JIT at puzzle 1.11 and array1
1.07, bisected to 3e0de22 ("errors and deoptimization are cold by
default"), with the interpreter unchanged. The hot stencils
(`VREF`/`VSET`/`ADDI_JMP`/compare-and-branch) are byte-identical in the
two builds; 3e0de22 only shortened copies of stencils that raise after
a call by 5 bytes (`CALLK*`, `PRIMCALL`, ...), and the JIT packs copies
back to back, so the loops after them moved. Starting the JIT region 16
bytes later swaps the two builds' times, and start offsets 0–63 give
array1 0.247–0.294 s with a 32-byte period.

Two principled layouts were measured on the whole suite against an A/A
copy and not kept: JCC-erratum-aware padding of each copy (as
`-x86-align-branch-boundary=32` does for the interpreter, lowering.md
§12) gave 1.017 (A/A 1.011), with array1 at its slow level at every
offset; aligning loop heads to 32 or 64 bytes gave 0.992 and 0.999 (A/A
0.995). The cost follows the absolute placement of the loops, through a
front-end effect that is not the JCC erratum. A JIT A/B within ±10% on
one short loop-bound program should be rechecked with the JIT's code
moved before it is taken as a regression.
