# WebAssembly interpreter: friction report

A usability study: a WebAssembly interpreter written from the docs
(`README.md`, `docs/language.md`, `docs/stdlib.md`, `docs/tutorial/`,
`std/`, `loimi --help`, `tests/run/*.lm`), without reading the other
example interpreters. It stresses what the earlier studies (stack
language, regex, Lox) did not: untagged typed values, structured control
flow resolved to jumps at load time, a validator the bytecode verifier
leans on, linear memory with bounds checks, `call_indirect` with
signature checks, and traps.

## Summary

| File | Lines | What |
|---|---:|---|
| `decode.lm` | 628 | binary format: LEB128, sections, UTF-8 names, syntax of every instruction (all "malformed" errors) |
| `compile.lm` | 1142 | validator and translator in one pass: control stack, typed operand stack, pending locals/constants, labels, K-forms, compare-and-branch fusion |
| `vm.lm` | 812 | the vm (`W`, about 330 ops, most from `inline for` tables), runtime structures, table/bulk helpers |
| `rt.lm` | 481 | module validation, linking (imports, `register`), instantiation, segments, start, invocation |
| `host.lm` | 177 | `spectest`, `env` (`print_i32`, `print_f64`, `putchar`, ..) and a WASI preview 1 subset |
| `main.lm` | 265 | CLI (`wasm prog.wasm`, `--invoke`, `--disasm`) and the spec-test script protocol |
| `check.sh`, `tests/` | 67 + 56 | every dispatch mode with `-W error`, C programs compiled to wasm32, the spec files |
| `bench/wasm/spec_run.py` | 234 | spec runner (wast2json JSON to a command script, one process per file) |
| `bench/wasm/*` (subagent) | ~430 | pinned testsuite, wabt, wasm3, cmake, wasi-libc (MVP build), CoreMark, CLBG; `bench.py` |

**Features.** The wasm MVP core, plus sign extension, saturating
float-to-int, multi-value (blocks with params and results, multiple
returns), reference types (funcref/externref, `ref.*`, `table.*`,
several tables, `call_indirect` through any table, typed `select`) and
bulk memory (`memory.copy/fill/init`, `data.drop`, `table.init/copy`,
`elem.drop`). Imports and exports of every kind across instances
(calls into another instance switch memory, globals and tables and
come back through a verified RESTORE stub). Traps with the spec's
messages. Not built: SIMD, threads, GC, tail calls, multi-memory, the
text format.

**Spec tests** (WebAssembly/testsuite f3f04866, the commit wabt 1.0.36
pins; `bench/wasm/spec_run.py`): **all 90 non-SIMD .wast files pass,
27,331 commands, 0 failures**, in all three dispatch modes. 581
commands are skipped: text-format modules (`assert_malformed` on `.wat`,
`module quote`), which a binary-only interpreter cannot read, and a few
actions whose arguments are non-null funcref constants. The 57 SIMD
files fail (not implemented: 121 commands pass by accident). Per file:
`python3 bench/wasm/spec_run.py --wasm WASM --skip simd`.

**Time.** First doc read 01:51 (by the clock in this container); first
commit with a working interpreter (a C program compiled to wasm32 ran
correctly) 02:10; every MVP spec file passing 02:21; reference types
and bulk memory 02:37. The commit timestamps are in `git log`. (The
container clock advanced much less than the work suggests; the
relative order is what it is.)

**Speed** (best of 3, seconds; wasm-interp from the infrastructure
run, best of 3, run two at a time):

| benchmark | wasm3 0.5.0 (-O3) | Loimi (tailcall) | Loimi / wasm3 | Loimi goto | wasm-interp 1.0.36 |
|---|---:|---:|---:|---:|---:|
| binarytrees (CLBG, depth 15) | 1.462 | 1.540 | 1.05 | 1.686 | 32.9 |
| coremark (5000 iterations) | 2.027 | 2.172 | 1.07 | 2.507 | 55.6 |
| fannkuchredux (CLBG, 10) | 1.684 | 1.722 | 1.02 | 1.871 | 50.5 |
| fasta (CLBG, 2.5M) | 1.165 | 1.369 | 1.18 | 1.562 | 26.3 |
| mandelbrot (CLBG, 2400) | 2.000 | 1.887 | 0.94 | 2.280 | 63.1 |
| nbody (CLBG, 2.5M steps) | 1.992 | 2.489 | 1.25 | 2.810 | 57.0 |
| spectralnorm (CLBG, 2000) | 1.909 | 2.314 | 1.21 | 2.652 | 60.7 |
| **geometric mean** | **1.718** | **1.889** | **1.10** | 2.146 | ~48 |

All programs: unmodified public sources (CoreMark at eembc/coremark
1f483d5 with a timer port; Computer Language Benchmarks Game C programs
at benchmarksgame 4029666), compiled by clang 19 `-O2 -mcpu=mvp`
against an MVP build of wasi-libc; every output checked (CoreMark's
CRCs). Loimi is within 10% of wasm3 by geometric mean, faster on
mandelbrot and slowest relative to it on float-heavy nbody and
spectralnorm (F64_MUL/F64_LOAD-bound: 45% of nbody's dispatches),
and about 25x faster than wabt's interpreter. `--dispatch=goto` is
1.14x slower than tail calls on every program.

## Design

**Register vm over untyped 64-bit slots.** Wasm is a stack machine, but
after validation every operand stack height is static, so each stack
position gets a fixed *home slot* in the frame (`nlocals + position`)
and `i32.add` becomes `I32_ADD r5, r3, r4` (the approach of wasm3 and
Wizard's fast path). The translator also does not materialize
`local.get` or constants: a stack entry can be *pending* (a local's
slot, or a constant), so `local.get a; local.get b; i32.add; local.set c`
is one instruction, `I32_ADD c, a, b` (the last instruction's
destination is retargeted to the local when no label intervenes, with
`bc.Code.last()`). Pending entries are flushed to their home slots at
block boundaries, before a local they read is assigned, and for calls.
A stack design would have spent dispatches on `local.get`/`local.set`
(about 40% of wasm instructions) and on stack-pointer updates.

Calls use register windows: arguments are the caller's top home slots,
the callee's frame starts at the first one, and results come back in
the same slots. Frames record the return pc and the caller's window.
Branches that carry values move them to the target's home slots first;
`br_table` is `BR_TABLE idx, n` followed by n + 1 JMP words (stubs after
the table do the moves for targets that need them).

**Typed views of one slot.** Values are untagged: the validator has
proved every type, so each op reads a slot as the type it needs, and
operand kinds express that directly:

```loimi
operand ri = *((regs + it) as *i32) verify it < (f.nslots as u64) display "r{}";
operand ru = *((regs + it) as *u32) verify it < (f.nslots as u64) display "r{}";
operand rd = *((regs + it) as *f64) verify it < (f.nslots as u64) display "r{}";
op I32_LT_U(d: ri, a: ru, b: ru) : RRR { d = b2i(a < b); next }
```

**What the vm verifier checks, and what the wasm validator checks.**
The wasm validator (compile.lm) proves types: no op reads a slot as a
type that was not written there. The generated `W.verify` proves memory
safety of the *translated* code: every slot index is below the frame
size, every function, global and type index is in range for the
instance (`verify_context f: *Func, inst: *Inst`), every jump lands on
an instruction, a JMP follows every compare-and-branch (`then JMP`), and
a `verify brtable { .. }` block checks that `BR_TABLE n` is followed by
n + 1 JMPs. Type confusion between slots cannot break memory safety
(every view stays inside its 8-byte slot), so the vm verifier does not
need types, and the translator is not trusted: a translator bug becomes
a verification error at load time, not a wild write. Bounds that exist
only at run time (memory and table size, call depth, stack room for the
callee's frame) are checked in the handlers, with `cold` slow paths.

This division worked very well, and the verifier earned its keep: it
rejected the translator's output during development, each time for a
real translator bug (item 1 is the first one).

## Friction, ranked by impact

### 1. A `const` name in a `match` pattern silently binds a new variable

**Tried.** Stack-entry locations as `u8` constants, matched by name:

```loimi
const HOME: u8 = 0; const LOCAL: u8 = 1; const CONST: u8 = 2;
match v.loc { LOCAL => v.x, CONST => { .. }, _ => t.home(pos) }
```

**Happened.** No error and no warning. `LOCAL` is a fresh binding that
matches everything, so every value took the first arm; the translator
emitted register numbers taken from constants' bits. The only symptom
was the vm verifier rejecting the code: `internal error: translated
code fails verification: word 10 (I32_AND): operand `b` (ri) fails
`it < (f.nslots as u64)`` (operand `b=r65532`, the constant -4). A
3-line probe confirmed it (`match x { A => 10, B => 20, _ => 30 }`
returns 10 for every x).

**Did instead.** An `enum Loc { Home, Local, Const }`.

**Suggested fix (language).** Resolve an identifier pattern that names
a `const` in scope as a comparison with it (Rust does), or make it an
error ("`LOCAL` is a constant; this pattern binds a new variable");
in any case warn about unreachable arms after an irrefutable one
(`B => 20` and `_ => 30` here can never match). This is the costliest
item of the study: in a less checked setting it is a silent
miscompile.

### 2. `f32` is a second-class type

**Tried.** `x.to_bits()`, `f32.from_bits(b)`, `x.sqrt()` on `f32`.

**Happened.** `error: no method `to_bits` on type `f32``; likewise
`sqrt`, `floor`. `docs/stdlib.md` lists them for floats without saying
"f64 only". No test in `tests/run` uses `f32` at all.

**Did instead.** Punning through a local
(`var y = x; *((&y) as *u32)`, which LLVM turns into a `movd`), and
libm through `extern "C"` (`sqrtf`, `ceilf`, `floorf`, `truncf`,
`rintf`). Wasm's f32 arithmetic itself (`a + b` on `f32`) works and
compiles to `addss`.

**Suggested fix (language/std).** The same built-ins on `f32` as on
`f64` (`to_bits -> u32`, `from_bits`, `sqrt`, `floor`), and the missing
float operations for both (`ceil`, `trunc`, `round_even`/`nearest`,
`abs`, `copysign`, `min`/`max` with a stated NaN rule), lowered to the
LLVM intrinsics so they become `roundsd`/`sqrtss` instead of libm calls.
A guest with IEEE semantics needs all of them.

### 3. No expression for "the next instruction" in a vm function

**Tried.** A shared `-> never` vm function `do_call(f, base)` that saves
the return address as `pc + 1` (as the tutorial's CALL does), called
from CALL (one word) and later from CALL_INDIRECT_X (one word plus an
inline-cache word holding a table index).

**Happened.** Calls through a table other than 0 crashed with SIGILL:
the callee returned to `pc + 1`, the cache word, whose value (2) ran as
opcode 2 (CONST64) and the stream went off the rails into the
placeholder's invalid opcode. Debug and release builds alike; no
diagnostic. Found with print statements.

**Did instead.** `do_call(f, base, ret)` with each op passing its own
size (`pc + 1`, `pc + 2`). (Afterwards I saw Lox does the same.)

**Suggested fix (language).** A control-word-like expression for the
address `next` would continue at, e.g. `next_pc`, valid in ops and in
`-> never` vm functions (where it already knows the calling op's size,
since `next` works there). Then "save a return address" cannot get an
op's size wrong.

### 4. Formats cannot hold a wide immediate next to three operands

**Tried.** `i64.const` (64-bit immediate), loads with a 32-bit offset
and two 16-bit slots, `ADDR d, a, offset32`.

**Happened.** `error: format `RU` has only 2 operand field(s)` for an op
listing three operands on a 16+16+32 format; a u64 word cannot hold
16+16+16+32. There is no way to declare a second instruction word as an
operand.

**Did instead.** An inline cache `cache Imm64 { v: u64 }` used as an
immediate, written after `code.op(..)` zeroed it
(`let pc = t.code.op(W.CONST64(s)); t.code.set(pc + 1, bits);`). It
works and the disassembler even prints it (`[cache v=4607182418800017408]`),
but the verifier cannot check it: the table and segment indices of the
bulk ops and of CALL_INDIRECT_X live in such a word and must be
bounds-checked at run time, which is exactly what operand kinds exist
to avoid.

**Suggested fix (language).** Trailing immediate words in formats
(`format RRRX { op: 16, a: 16, b: 16, c: 16 } + imm: u64`), with
operand kinds and `verify` clauses like any field, encoder parameters,
and disassembly. `encoding bytes` (not built) would also cover it.

### 5. Re-encoding one field of an instruction word is hand-written bit twiddling

**Tried.** Retargeting the destination of the last instruction
(`local.set` after an op), patching the table-index slot into a
`call_indirect`, turning a compare into a compare-and-branch.

**Happened.** The generated API reads fields (`W.RRR_b(w)`,
`W.RRK_k(w)`) and replaces opcodes (`W.with_opcode`), but cannot set a
field. Encoders exist per op only, so emitting "this format with that
opcode" needs a template op.

**Did instead.** Masks: `(w & 0xFFFFFFFF0000FFFF) | (x << 16)`, a
special case for the 12-bit field of `SELECT4`, and
`W.with_opcode(W.I32_ADD(a as u16, b as u16, c as u16), op)` as a
generic three-register encoder (the casts are needed: 16-bit fields take
`u16`).

**Suggested fix (generated API).** `W.set_RRR_a(w, v) -> ?W`
(range-checked like `set_label`) and a per-format encoder
`W.encode.RRR(op, a, b, c)`.

### 6. LLVM turns a zeroing loop into a `memset` call, and the handler gets a stack frame

**Tried.** In CALL: `while i < n { nr[i] = 0; i += 1; }` to zero the
callee's locals.

**Happened.** The loop became `call memset@plt`, so CALL's handler
pushed a frame and spilled on *every* path (57 instructions:
`push %rbp; sub $0x10,%rsp; .. mov %rdi,0x8(%rsp)`), though no call is
visible in the source. The `cold` discipline cannot help with calls
LLVM introduces itself.

**Did instead.** Four unconditional stores (slots past `nlocals` are
the callee's scratch, and the stack has slack) and a `cold` path that
calls an `#[inline(never)]` loop for longer runs: 46 instructions, no
frame.

**Suggested fix (compiler/docs).** Compile handlers with
`-fno-builtin`-like attributes (`"no-builtins"`) or at least warn in
`--vm-stats`/`-W` mode when a handler's hot path ends up with a frame;
mention the pitfall in tutorial chapter 8's checklist ("no stack frame").

### 7. No unaligned load/store primitive

**Tried.** Guest memory accesses at arbitrary addresses.

**Happened.** `*((mem + ea) as *i32)` compiles to `load i32` with the
type's natural alignment (4), which is undefined behavior for an
unaligned guest address. It works on x86-64 and AArch64, but LLVM is
entitled to assume alignment (vectorizing, combining). Cache fields get
`align 4` unaligned accesses, but there is no user-facing way to ask
for that.

**Did instead.** Kept the plain loads.

**Suggested fix (language/std).** `p.read_unaligned()` /
`p.write_unaligned(v)` (or `*align(1)` pointers), lowered to
`load ... align 1`.

### 8. Fixed-size array literals need their length spelled out twice

**Tried.** Lookup tables of tuples for K-forms and fusions returned
from a function.

**Happened.** The return type needs the exact length
(`-> [(u64, u64, u64, bool); 52]`) and the loop repeats it
(`for i in 0..52`); adding a row means editing two numbers, and there is
no `const` array of tuples of other consts' values in practice (no
comptime, `W.OP_X` constants). Also, matching on `W.OP_X` constants was
avoided after item 1.

**Suggested fix (language).** Infer `[T; _]`, and allow
`for x in array` iteration.

### 9. `mem.Vec<u64>.new()` is "not a value"

**Happened.** `error: `Vec` is not a value` at `mem.Vec<u64>.new()`.
`docs/language.md` explains it (explicit type arguments are not taken
on a generic type's static functions), but the message does not say
what to write.

**Did instead.** `var v: mem.Vec<u64> = mem.Vec.new();`

**Suggested fix (diagnostics).** "write `var v: mem.Vec<u64> =
mem.Vec.new();`" in the error.

### 10. Smaller things

- **Range patterns are undocumented.** `0x28..=0x3E =>` works in
  `match` (seen in `tests/run/control.lm`), but `language.md` never
  mentions it; I almost wrote 23 alternatives.
- **`bool` to integer.** No `c as i32`; a `b2i` helper with
  `#[inline(always)]` (it compiles to `setcc`, fine).
- **No `perf`/sampling profiler** on the machine and none in `std.prof`;
  `--vm-stats` was the only profile, and it does not see time spent in
  host functions. The fasta slowdown (item 11) was invisible to it.
- **`sh`-style probes**: compiling and running a one-off file needs a
  scratch directory and an `-o`; `loimi run file.lm` covers it, but
  there is no `--disasm`-style dump of a vm's generated op table
  (`W.NUM_OPS`, sizes) from the command line.

### 11. Not Loimi, but cost time: WASI and libc buffering

wasi-libc line-buffers stdout when `fd_fdstat_get` reports a character
device. My first stub always said "character device", so fasta made
one `write` per 61-byte line and took 1.9x wasm3's time; reporting a
regular file when `isatty` is false brought it to 1.3x. And the prebuilt
wasi-sdk sysroot is compiled with bulk memory and reference-types
encodings, which wasm3 0.5.0 cannot read, so the benchmark
infrastructure builds wasi-libc from source with `-mcpu=mvp`.

## What worked well

- **Operand kinds as typed views.** One line per wasm type, and every
  handler reads like the spec (`I32_SHR_U(d: ru, a: ru, b: ru) { d = a >> b }`).
  Shifts taken modulo the width, `+%`/`*%` wrapping, `clz`/`ctz`/
  `popcount` and signed/unsigned views made the integer ops one-liners.
- **`inline for`** generated about 300 of the ops from tables: all
  binary and unary ops, loads and stores (and their 64-bit-address
  variants), K-forms and 60 compare-and-branch ops. Errors pointed into
  the table row.
- **The verifier as a translator test oracle** (above), plus `W.disasm`
  for free, which made every translator bug a one-glance diagnosis
  (`--disasm` in `main.lm` is 10 lines).
- **`then JMP`** for compare-and-branch: the fused op and its target
  word, checked by the verifier, with 32-bit offsets and no branch
  relaxation needed.
- **`verify NAME { .. }`** for the `br_table` invariant.
- **`std.bc`**: labels, `last()` for the retargeting peephole,
  `truncate` for fusion, `check()`.
- **`cold`** kept every trap path out of the handlers, and **effects**
  never got in the way (no GC here).
- **Everything compiled the first time**, almost: `vm.lm`, `compile.lm`
  and `rt.lm` (2,000 lines) built after one fix (item 4); the remaining
  bugs were mine, and the spec suite found them.
- **Dispatch modes are free**: the same source passes the whole suite
  in tailcall, goto and switch mode.

## Generated code

x86-64, `--release`, tail-call dispatch. State: `pc` r13, `regs` r14,
`fp` r15, `mem` rdi, `msize` (memory size, a state register), ctx r12;
the instruction word arrives in a register. No handler below has a
stack frame or a spill.

`I32_ADD` (13 instructions + the jump):

```text
mov    %rsi,%rax
shr    $0x20,%rax
movzwl %ax,%eax            ; b
mov    %rsi,%rcx
shr    $0x30,%rcx           ; c
mov    (%r14,%rcx,8),%ecx
add    (%r14,%rax,8),%ecx
shr    $0x10,%esi           ; d (32-bit shift truncates)
mov    %ecx,(%r14,%rsi,8)
mov    0x8(%r13),%rsi       ; next word
add    $0x8,%r13
movzwl %si,%eax
jmp    *0x424110(,%rax,8)
```

`I32_LOAD` (18): the same decode, `lea (%rax,%rcx,1),%rdx; add $4,%rdx;
cmp <msize>,%rdx; ja cold` (the bounds check), `mov (%rdi,%rax,1),%eax`,
store, dispatch. With the memory size in `ctx` it was one `cmp
0x30(%r12),%rdx` instead; making it a state register measured as noise.
`I32_ADDK` 13, `F64_MUL` 14, `RET1` 10, `BR_I32_LT_S` 20 (it reads its
`then` word), `CALL` 46 (after item 6; 57 with a frame before).
Trap paths are a single `jmp vm.W.op.X.coldN`.

## Optimizations, and what helped

Measured with `bench/wasm/bench.py` (best of 3, interleaved).

| change | effect |
|---|---|
| pending locals and constants, destination retargeting (from the start) | most of the gap to a naive stack translation (not measured separately) |
| K-forms (`I32_ADDK d, a, k16`, 36 ops) and compare-and-branch fusion with `then JMP` (40 integer ops) | geomean 1.21x of wasm3 from about 1.6x: nbody 5.4 to 2.6 s, spectralnorm 5.9 to 2.5 s (single runs, before/after) |
| float compare-and-branch (20 ops, NaN-correct negated forms) | noise |
| memory size in a state register instead of `ctx` | noise |
| CALL without a stack frame (item 6) | noise to 10% on call-heavy code (spectralnorm 2.70 to 2.34) |
| `SELECT4 d, a, b, c` (12-bit fields) instead of MOV + SELECT + MOV | noise |
| WASI `fd_fdstat_get` filetype (item 11) | fasta 2.5 to 1.5 s |

`--vm-stats` was the main tool: its first report (fannkuch) showed
CONST32 at 12% and `cmp; BRNZ` pairs everywhere, which led directly to
the K-forms and fusion; its suggested supers (`CONST32_I32_ADD_CONST32`
and friends) were symptoms of missing constant operands rather than
good supers. I did not add `super`s: after the K-forms, no pair was
above a few percent across the suite, and the earlier studies found
supers within noise. Not tried for lack of time: folding constant
addresses into load offsets, `i32.add` + load fusion, `#[forward]`
supers for float chains (nbody is F64_MUL/F64_LOAD-bound), and the
copy-and-patch JIT (`--jit-stencils`).

Would the other examples have helped? Mini's and Lox's vms (skimmed
after the interpreter worked) show `then JMP` compare-and-branch and the
same "pass the return pc into the shared call function" workaround as
item 3, so they would have saved the debugging of item 3. None uses
`f32`, untyped slots or wide immediates, so items 1, 2, 4 and 5 would
have happened anyway.
