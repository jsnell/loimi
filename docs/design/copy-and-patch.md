# Design note: a copy-and-patch JIT from `vm` declarations

Status: **prototype**, behind flags (`loimi build --jit-stencils`, run
with `LOIMI_JIT=1`), x86-64 Linux, tail-call dispatch, one vm per
program. It runs Mini (`examples/mini`). Written 2026-11-02 to answer
open question 6 of the [roadmap](../roadmap.md):

> Handlers are separate functions with a fixed register interface, which
> is close to what a copy-and-patch template compiler needs. Can `vm`
> declarations generate a baseline JIT with no additional source?

**Answer: yes.** The prototype generates its stencils from Mini's
existing handlers. It needs no change to Mini's source and no stencil
code written by hand. It compiles every op (113 stencils, supers
included). It passes every Mini script and conformance test with the
JIT forced on at the first call or backward jump.

Against Mini's interpreter, it takes 32% off fib, 11% off sieve and
41% off mandelbrot, and 22% by geometric mean over the 14 benchmarks.
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

1. **The stencil module** is the program's IR with every `internal`
   definition rewritten to `available_externally dso_local`. Bodies stay
   available for inlining (`value.truthy`, `int_index` and the like
   inline as they do in the handlers). Anything not inlined is
   referenced *by name*, with the calling convention as declared, so a
   stencil calls the interpreter's own copy of a function. That copy has
   not been given a different convention by LLVM's interprocedural
   passes, because the program takes its address (step 4).
   Thread-locals become `localexec`. The stencils, the exit stencil and
   the hole declarations are appended.
2. `clang -O3 -fno-pic -ffunction-sections -fdata-sections -c`: one
   section per stencil, and one per constant.
3. **The ELF reader** (about 300 lines of Rust, no crates) walks the
   section headers, symbol table and `.rela.text.__jit.op.*` sections.
   Each relocation becomes a patch record of one of these kinds:
   - a hole (`__jit.w`, `__jit.f`, `__jit.n`, `__jit.j`, parsed from the
     symbol name);
   - a program symbol (cold functions, `vm.Mini.table`, helpers, libc);
   - a thread-local (`R_X86_64_TPOFF32`);
   - read-only data of the object (float constants, `.rodata.cst*`),
     which is copied.

   Relocation types are `R_X86_64_64`, `PC32`, `PLT32`, `32`, `32S` and
   `TPOFF32`. A stencil with any other relocation is rejected, and so is
   one that refers to another text section or to data with relocations.
   Its op then uses the exit stencil. With `-fdata-sections` no Mini op
   is rejected (without it, `.rodata` with the op-name table caused
   two rejections).
4. **The tables** go into the program as IR constants: the code bytes,
   the patch records, a per-opcode stencil index, `loimi_jit_syms` (the
   addresses of the program symbols the stencils use, which also keeps
   those functions' ABI fixed), the read-only data, and a function
   computing each thread-local's offset from the thread pointer
   (`llvm.threadlocal.address` minus `%fs:0`).

The stencils add about 2.2 s to Mini's 4.3 s build: the stencil module
is the whole program again, so most of the time is clang optimizing
`available_externally` bodies that are thrown away. Mini's 113 stencils
are 23 KB of code.

A stencil whose fast path ends in `jmp __jit.n.D` as its *last*
instruction has the jump dropped when the next instruction's code
follows. In practice LLVM places the fast path first and the cold
blocks after it, so the jump is usually in the middle. It stays a
direct `jmp` (section 7).

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
  it compiles quickened members once they are hot. `become` in a JIT
  build first calls `loimi_jit_become(pc)`. If the code object is
  compiled, that restores its original words and drops the compiled
  code. The code object is compiled again when it is hot again (at most
  8 times; after that it stays interpreted). `Vm.optimize` and
  `Vm.deopt` (generated) do the same through `loimi_jit_invalidate`, so
  `refuse` (re-fusing supers after quickening) never sees an enter word.
  Old compiled code that is still running when it is dropped stays
  correct: it implements the old members, and Mini's members are all
  complete. A vm whose members are not complete (a member that assumes
  a type check done elsewhere) would need the old code to exit instead.
  That could be generated, since the compiler knows which ops can
  `become`.
- **The verifier.** Only code built by `Vm.build_verified` is registered
  with the JIT. Stencils rely on the facts the handlers rely on (operands
  in range; label targets are instruction boundaries), and the JIT
  checks that every continuation it patches lands on an instruction of
  the same code object. Enter words use an opcode no op has, so a
  verified program cannot contain one, and the verifier would reject a
  copy of patched code.

## 5. The run-time JIT, exits and re-entry

`src/jit_rt.c` (about 400 lines of C, compiled into the program):

- **Registry.** `Vm.build_verified` calls `loimi_jit_register(code, n)`.
  Code objects are kept sorted by address. A new object overlapping an
  old record means the collector freed the old one, so the record is
  dropped.
- **Hot counters.** With the JIT on (`loimi_jit_on`), the interpreter
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
- **Compiling** a code object is two passes. The first walks the
  instructions (`opinfo`: per opcode, the size and stencil to use) and
  lays out one stencil copy per instruction. The second copies the
  stencils and applies the patch records. The pages are `mmap`ped near
  the program (pc-relative calls into it must fit in 32 bits), written
  while `PROT_READ|PROT_WRITE`, then switched to `PROT_READ|PROT_EXEC`.
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
  the exit stencil, `musttail tbl[opcode(*pc)]`). The interpreter then
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
  fib): the interpreter's calls, returns and backward jumps carry the
  enter-word check and the test before counting. The first version
  updated a counter on every backward jump and cost up to 16%.
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
  direct jump. A block-placement option, or splitting each stencil into
  a hot part and a cold part placed elsewhere (as CPython 3.14 does),
  would let the fast path fall through.
- `pc` is kept current in its register (`add $4` per op). Keeping it
  as a hole constant and materializing it only on exits would save
  that, but cache-field accesses would then need 64-bit addresses.
- Cold paths exit to the interpreter (above). Stencil copies of cold
  blocks whose `next` continues in compiled code would fix that.
- `k[it]` is still a load. The constant pool cannot change after
  verification, but Loimi has no way to say so (section 8).

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
     only `vm` syntax change this work suggests.
   - *Complete members*: the prototype relies on every family member
     handling every input, so that stale compiled code stays correct
     after a `become`. That should be checked or declared. Otherwise
     the compiler must make code that can `become` exit afterwards
     (the compiler knows which ops can).
2. **Compiler.**
   - Build the stencil module from the handlers and what they reach,
     not the whole program again (build time: +2.2 s for Mini).
   - Hot and cold stencil parts, so fast paths fall through, and cold
     continuations that return into compiled code.
   - An interpreter that does not count when the JIT is off: two copies
     of the few handlers that `jump` (as LuaJIT swaps `FORL` and
     `IFORL`), or the instrumentation-variant machinery (`language.md`
     §9). That brings "JIT off" from +5% to 0.
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
   experiment if this tier is built.

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
for profilers. At build time, `LOIMI_JIT_REPORT=1` reports the stencils
(and any rejected ones), and `LOIMI_KEEP_STENCILS=dir` keeps the
stencil module and object.
