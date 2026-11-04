# Lowering to LLVM

How each Loimi feature becomes LLVM IR. The constraint throughout: **stock
LLVM only** (no custom passes required for correctness, no forked backend).
A few codegen options are set by the driver and noted below. IR is abridged and
assumes LLVM ≥ 19 (`preserve_none` on x86-64 and AArch64).

Running example: `vm Mini` from `language.md`, using fixed 32-bit instructions
and the `nanbox` value representation.

## 1. Compiler pipeline

```
source ─▶ parse ─▶ resolve ─▶ typecheck + effect inference ─▶ comptime eval
       ─▶ MIR ─▶ vm expansion ─▶ LLVM IR ─▶ opt (O2/O3) ─▶ .bc / .o ─▶ lld
```

**vm expansion** is the Loimi-specific stage. For each `vm` it generates, as
ordinary MIR:

- the handlers, with operand decoding, `next` expansion, and `cold` outlining;
- one dispatch table per variant;
- the verifier, encoders and emitters, disassembler, peephole and deopt
  passes, and metadata from the format and operand declarations;
- C-ABI entry points (`Mini.run`, `Mini.call`).

Everything after vm expansion is a normal compiler. The output can be
bitcode, so a Loimi interpreter can be LTO-linked into a C/C++ host with
`clang -flto`.

**As implemented** (see [`status.md`](status.md)): there is no MIR or
comptime stage. The compiler type-checks the AST into a typed tree and
emits textual LLVM IR, which `clang-19` compiles and links. The vm tools
(encoders, emitters, `op_size`/`op_name`, `optimize`, `deopt`, the
verifier, label helpers, the disassembler) are generated as Loimi source
(`src/vmexpand.rs`) and type-checked like user code; handlers and
dispatch tables are generated as IR (`src/vmgen.rs`). Entries are
internal functions: there are no generated C-ABI entry points.
`Mini.call` is an entry the interpreter writes, declared `uses reenter`
(§4). A C host embeds an interpreter through `export "C"`
functions written in Loimi, built with `--lib` (static archive) or
`--shared`, with a header from `--header` (`src/header.rs`; see §4 and
`language.md` §10.1).

## 2. Values (`nanbox`, `lowtag`, `boxed`)

### `nanbox`

`Value` is an `i64`. The `unboxed` variant is the double's bits, with NaNs
canonicalized to `0x7FF8_0000_0000_0000`. Doubles therefore never exceed
`0xFFF8_0000_0000_0000` (the negative default NaN that x86 arithmetic
produces from canonical inputs), and every word above it is free for the
other variants. Each tagged variant owns an aligned block of that space:
its words are `tag << w | payload`, where `w` is the block's width in bits
and the payload is stored zero-extended in the low `w` bits.

**Classic layout (at most 7 tagged variants).** Each variant gets a 48-bit
block, `0xFFF9` to `0xFFFF` in the top 16 bits, in declaration order.
For the example in language.md:

| Variant | Encoding |
|---|---|
| `Num(f64)` | the double's bits; always `< 0xFFF9 << 48` |
| `Int(i48)` | `0xFFF9_xxxx_xxxx_xxxx` (the integer's low 48 bits) |
| `Nil`, `Bool(bool)` | `0xFFFA_0000_0000_0000`, `0xFFFB_0000_0000_000b` |
| `Str`, `Obj`, `Fn`, `Native` | `0xFFFC`–`0xFFFF` in the top 16 bits, 48-bit pointer in the rest |

Declarations written before wide tags existed keep exactly this encoding.

**Eight tagged variants**, when one of them has a payload of at most 47
bits: the classic layout for seven of them, in declaration order, and
the start of the 47-bit block `0xFFF8_8000_0000_0000` for the narrowest
(the last of that width; a variant without payload is that one word).
Adding an eighth variant thus keeps every other variant's 48-bit block, so
tuple tests that mix pointers with a narrow integer (`(T(t), Int(_))`
with `Int(i32)`) still merge into one test; under the packed layout below
the `i32` would get a 32-bit block, and such tests split in two (Mini's
`hash_strings`, `interp` and `queens` ran 4-6% slower when coroutines
added variants; examples/mini/COROUTINES.md). `NUM` is
`0xFFF8_8000_0000_0000`.

**Pinned blocks (`block N`).** A pinned variant needs a block of `N`
bits instead of its payload width. The classic layout is used only if
no pin is narrower than 48 bits; the eight-variant layout only if at
most one is, and that one takes the 47-bit block (unpinned variants
pinned at 48 never do); otherwise the packed layout allocates the pinned
widths. Mini pins `Int(i32) block 48` and `Nil block 0`: the same
eight-variant layout as without pins, and a ninth variant would move
only unpinned narrow variants. After choosing, the compiler warns
(`value_layout`) about every unpinned variant whose block is narrower
than 48 bits, so a layout change caused by a new variant never goes
unnoticed. `V.X.mask` is `!(2^w - 1)` and `V.X.tag` is `T << w`
(`w` = `V.X.block`); `V.Num.limit` is `NUM`. `--explain-values` prints
all of them.

**Packed layout (more than 8 tagged variants, or 8 with no payload under 48 bits).** Each variant needs a block
of 2^w words, where `w` is its payload width: 48 for pointers, `N` for
`iN`/`uN`, 1 for `bool`, 0 for no payload. The free space is the seven
48-bit blocks `0xFFF9..0xFFFF` and the 47-bit block
`0xFFF8_8000_0000_0000..0xFFF8_FFFF_FFFF_FFFF` (the upper half of `0xFFF8`,
which no canonical double reaches). Variants are placed widest first by a
buddy allocator: each takes the smallest free block that fits (lowest
address on ties) and splits it in halves down to its size. For example,
`tests/run/values_nanbox_packed.lm`:

| Variant | Width | Block | Encoding |
|---|---|---|---|
| `Num(f64) unboxed` | | | `< 0xFFF8_8000_0000_0000` |
| `Int(i48)`, `P`, `Sym`, `Str`, `Vec`, `Clo` (pointers) | 48 | `0xFFF9`..`0xFFFE` | `0xFFF9_xxxx_xxxx_xxxx`, ... |
| `Small(i47)` | 47 | the `0xFFF8` upper half | `0xFFF8_8000_0000_0000 \| x` |
| `Char(u32)` | 32 | low end of `0xFFFF` | `0xFFFF_0000_xxxx_xxxx` |
| `Prim(u16)` | 16 | | `0xFFFF_0001_0000_xxxx` |
| `Flag(bool)` | 1 | | `0xFFFF_0001_0001_000b` |
| `Nil`, `F`, `T`, `Unspec`, `Eof` | 0 | | `0xFFFF_0001_0001_0002` .. `0006` |

**Shared layout (more 48-bit needs than blocks).** The packed layout
fits at most seven variants that need a whole 48-bit block (pointers and
48-bit integers). Beyond that, `Gc` variants share 48-bit blocks, up to
8 in a block: GC objects are at least 8-byte aligned (std.gc's are
16-byte aligned: 16-byte headers and size classes, 16-byte-aligned
chunks and calloc'd large objects), so the low 3 bits of a `Gc` pointer
are free, and a variant in a shared block is told apart by a 3-bit *low
tag* `L` there. Its word is `T << 48 | ptr | L`; its mask covers both
tags, `0xFFFF_0000_0000_0007`, and `V.X.block` is 45 (the pointer bits
between them). Which variants share: those pinned `block 45`, then as
few unpinned `Gc` variants as make the declaration fit, the last
declared first (the allocator tries 0, 1, 2, ... of them; each group of
8 sharing variants is one 48-bit request to the same buddy allocator, so
everything else is placed as in the packed layout). Low tags are given
in declaration order within a block. Raw pointers (`*T`) never share:
their alignment is not known. A declaration that needs no sharing keeps
its packed (or classic, or eight-variant) layout exactly, so only
declarations that did not compile before, or pin `block 45`, change.
`tests/run/values_explain_shared.lm`:

| Variant | Block | Encoding |
|---|---|---|
| `Int(i48)`, `Str`, `Pair`, `Env`, `Raw(*Raw)`, `Vec` | 48 | `0xFFF9`..`0xFFFE` in the top 16 bits |
| `Sym(Gc<Sym>) block 45`, `Clo(Gc<Clo>)` | 45 | `0xFFFF_0000_0000_0000 \| ptr \| 0`, `.. \| ptr \| 1` |
| `Char(u32) block 32`, `Nil`, `F`, `T` | 32, 0 | in the 47-bit block |

| Pattern (shared block) | Test / code | x86-64 |
|---|---|---|
| `v is Sym` | `(v & M) == (T << 48 \| L)`, `M = 0xFFFF_0000_0000_0007` | `movabs; and; cmp` (constants hoisted in loops) |
| `match v { Sym(_) => .., Clo(_) => .., ... }` | LLVM switches on `(v & M) - T << 48`: a jump table over `L` | `and; add; cmp; ja; jmp *` |
| `(a, b) is (Sym, Clo)` | `((a ^ tagS) \| (b ^ tagC)) & M == 0` | one test |
| `(a, b) is (Str, Clo)` (whole-block `Gc` with shared) | the same, with the shared `M`: a whole-block `Gc` pointer's low bits are 0 | one test |
| payload | `inttoptr(v & 0xFFFF_FFFF_FFFF) - L`, the `- L` a `getelementptr` | `and`; then `mov 7(%r)` for field offset 8 and `L = 1` |
| `Sym(p)` | `(p & 0xFFFF_FFFF_FFFF) \| (T << 48 \| L)` | `movabs; or` |

The GC marks a value word through the same payload extraction, so it
sees the untagged pointer (`--gc-check-roots` validates exactly the
object start). The JIT's stencil variants per constant tag
(`--jit-stencils`) select on the same mask and tag. AArch64 needs no top-byte-ignore
(the tags are stripped before every dereference).

*Costs* (scratch microbenchmark: a loop over 64K values matching four
pointer variants and an `Int`, reading a field, best of 3 on x86-64):
whole blocks 20 ms, the four in one shared block 18 ms, one `Obj`
variant with a `kind` field read from the object 26 ms (5-way periodic
pattern); with a random pattern (mispredicted) 147, 139 and 155 ms. A
shared block is as fast as whole blocks here because the match becomes
one jump table, and a test is one more instruction than `shr; cmp`.

*Why not header type ids.* The other way to have more pointer types
than tags is one variant whose object header carries the type (a
`kind` field, CPython's `ob_type`): no encoding bits, but every test is
a dependent load before the branch (26 vs 18-20 ms above), the objects
need a common header layout the compiler would have to own, and a
tuple pattern cannot merge such tests into one compare. Low tags cost
a mask on the test and nothing on the payload, and keep every variant
an ordinary variant: patterns, tuple merging, GC tracing, the JIT and
`--explain-values` work unchanged.

The unboxed limit `NUM` is the lowest tagged word: `0xFFF9 << 48` in the
classic layout, `0xFFF8_8000_0000_0000` if the 47-bit block is used.

Pattern tests (`T` is the variant's tag, `w` its block width):

| Pattern | Test | x86-64 |
|---|---|---|
| `v is Num` | `v <u NUM` | `cmp; jae` |
| `v is Int` | `(v >> w) == T` | `shr; cmp` |
| `v is Nil` (no payload, packed) | `v == T` | `cmp` |
| `(a, b) is (Int, Int)` | `((a ^ T<<w) \| (b ^ T<<w)) >> w == 0` | `xor; xor; or; shr; jne` |
| `(a, b) is (Num, Num)` | `umax(a, b) <u NUM` | `cmp; cmova; cmp` |
| payload of a pointer variant | `v & (2^w - 1)` | `and` |
| payload of an `iN` variant | `trunc v to iN` | free |

Tuple elements with tags in the same position (same `w`) merge into one
test; elements with different widths are tested separately and the results
and-ed. A tuple pattern of `k` values usually costs one test, not `k`.
`--explain-values` lists the variants by block width: each line is one
test.

**Float reads of a value slot.** A float-first fast path adds the two
slots as doubles before testing anything (a tagged word is a NaN, so a
non-NaN sum means both were numbers): `f64.from_bits(a as u64) +
f64.from_bits(b as u64)`. The checker turns `from_bits` of a place
through a pointer (`regs[it]`, `*p`, optionally behind a bit-preserving
cast to `u64`) into a load of the place as `double`, the IR of
`*((regs + a.raw) as *f64)`. As `load i64` + `bitcast`, LLVM keeps the
integer load (GVN merges it with the slow path's integer read of the
same slot) and moves it to an XMM register: two more instructions on
the fast path, and Mini's mandelbrot and nbody ran 4–5% slower. With
the `double` load, `addsd` reads memory and the slow path does the
`movq` instead.

### `lowtag`

The word is `payload << s | tag`, where `tag` has `s` bits. With at most 8
variants, `s = 3` and the tag is the variant's index; pointers (8-byte
aligned) are or-ed with it, integers of up to 61 bits are shifted by 3.
With more, pointers and 61-bit integers take 3-bit tags, and a variant with
an `N`-bit payload gets a tag of `64 - N` bits allocated from what is left
(the same buddy allocator, where splitting a tag appends a 0 or a 1 bit).
`block N` asks for a tag of `64 - N` bits (and a pin below 61 selects
this layout); `V.X.mask` is `2^s - 1` and `V.X.tag` the tag, and
`V.X.block` is `64 - s`.

| Pattern | Test |
|---|---|
| `v is Int` | `(v & (2^s - 1)) == T` |
| `(a, b) is (Int, Int)` | `((a ^ T) \| (b ^ T)) & (2^s - 1) == 0`; for `T = 0`: `(a \| b) & 7 == 0` |
| payload of a pointer variant | `v & -8` |
| payload of an integer variant | `ashr exact (v ^ T), s` |

The `exact` flag (the shifted-out bits are known to be zero) lets LLVM
cancel the shift against the `<< s` of overflow-checked arithmetic and of
re-tagging, so an `Int(i61)` add with tag 0 is `or; test; jne; add; jo;
store`, with no untagging at all.

### Overflow-checked arithmetic on odd widths

`add_checked`/`sub_checked`/`mul_checked` (and the checked `+ - *` of debug
builds) on `iN` with `N` not 8/16/32/64 shift the operands to the top of an
i64 and use LLVM's 64-bit overflow intrinsics, whose flag is then exactly
the overflow at `N` bits (for `mul` only one operand is shifted):

```llvm
%xs = shl i64 %x64, 16          ; i48 payloads
%ys = shl i64 %y64, 16
%r  = call { i64, i1 } @llvm.sadd.with.overflow.i64(i64 %xs, i64 %ys)
%v  = ashr exact i64 %r0, 16    ; exact: the low 16 bits are zero
```

For an `Int(i48)` nanbox add the hot path is

```asm
shlq   $16, %rdx        ; x << 16  (the tag shifts out)
shlq   $16, %rsi        ; y << 16
addq   %rsi, %rdi
jo     .Lslow
shrq   $16, %rdi        ; back to 48 bits, zero-extended
orq    %rcx, %rdi       ; | 0xFFF9 << 48
```

against `addl; jo; movl; orq` for `Int(i32)`. LLVM's own legalization of
`sadd.with.overflow.i48` sign-extends both operands (`shl; sar` each) and
compares the result with its re-extension, about twice as long.
Relational compares of odd widths likewise compare the operands shifted to
the top (`shl; shl; cmp`, not `shl; sar` per operand), and `/` and `%`
wider than 32 bits use a 32-bit divide when both operands are non-negative
and below 2^31 (64-bit `idiv` is two to three times slower on many x86
cores; the test is `(x | y) << k` against bits 31 and up, one `test` with
a mask). The division checks (`y == 0`, `x == MIN and y == -1`) test the
shifted operands too, so that a dominating compare such as Python's
`y > 0` guard on floor division (`icmp sgt i64 (y << 16), 0`) folds them
away: with them in `i48` form LLVM kept both checks and their `panic`
calls, and the handler set up a stack frame on its fast path.

On AArch64 the shifted forms fold into the shifted-register operands:
an `Int(i48)` add is `lsl; adds x, x, y, lsl #16; b.vs; mov; bfxil`, a
compare `lsl; cmp x, y, lsl #16`, and a multiply
`lsl; sbfx; mul; smulh; cmp ..., asr #63; b.ne` (no flag-setting multiply).

Measured alternatives (2026-11-03, Python interpreter, micro benchmarks
of int loops): doing add/sub in i64 on sign-extended operands, which
cannot overflow, and then checking that the sum fits in 48 bits
(`shl; sar` per operand, `add`, a range check) is 12 instructions instead
of 6 and 5-10% slower; computing the value with a plain 48-bit add next
to the shifted overflow add (shorter dependency chain: `add; and; or`) is
2-5% slower (more instructions; these handlers are throughput-bound). The
remaining cost against `Int(i32)` is the two `shl` per operation and the
`shr` before re-tagging; avoiding them needs the tag in the low bits of
the rotated word, which the checked operation does not know.

### `boxed`

`{ i64 tag, i64 payload }`, passed and returned in two registers; the tag
is the variant's index and integer payloads are sign- or zero-extended.

## 3. Handlers in `tailcall` mode

Every op becomes one function. All handlers share a signature: the context,
the state variables in declaration order, then the dispatch table if it is a
register (section 7).

```llvm
define internal preserve_nonecc void
@Mini.op.ADD(ptr %ctx, ptr %pc, ptr %regs, ptr %k, ptr %tbl) {
entry:
  %w    = load i32, ptr %pc, align 4

  ; decode-ahead: fetch the next word and its handler early
  %pc1  = getelementptr inbounds i32, ptr %pc, i64 1
  %w1   = load i32, ptr %pc1, align 4
  %op1  = and i32 %w1, 255
  %op1z = zext i32 %op1 to i64
  %hp   = getelementptr inbounds ptr, ptr %tbl, i64 %op1z
  %h    = load ptr, ptr %hp, align 8

  ; operands: format ABC, kind `reg`. Verified, so no bounds checks.
  %ai = ... ; (w >> 8) & 255, zext
  %bi = ... ; (w >> 16) & 255
  %ci = ... ; w >> 24
  %pdst = getelementptr inbounds i64, ptr %regs, i64 %ai
  %a    = load i64, ptr (gep %regs, %bi)
  %b    = load i64, ptr (gep %regs, %ci)

  ; (Int, Int)
  %xa = xor i64 %a, -1970324836974592
  %xb = xor i64 %b, -1970324836974592
  %o  = or i64 %xa, %xb
  %hi = lshr i64 %o, 48
  %ii = icmp eq i64 %hi, 0
  br i1 %ii, label %int, label %not_int

int:
  %x  = trunc i64 %a to i32
  %y  = trunc i64 %b to i32
  %s  = call { i32, i1 } @llvm.sadd.with.overflow.i32(i32 %x, i32 %y)
  %ov = extractvalue { i32, i1 } %s, 1
  br i1 %ov, label %slow, label %int_ok, !prof !unlikely

int_ok:
  %r  = extractvalue { i32, i1 } %s, 0
  %rz = zext i32 %r to i64
  %rv = or i64 %rz, -1970324836974592
  store i64 %rv, ptr %pdst, align 8
  ; `next`
  musttail call preserve_nonecc void %h(ptr %ctx, ptr %pc1, ptr %regs, ptr %k, ptr %tbl)
  ret void

not_int:
  %m  = call i64 @llvm.umax.i64(i64 %a, i64 %b)
  %nn = icmp ult i64 %m, -1970324836974592
  br i1 %nn, label %num, label %slow

num:
  ; bitcast, fadd, canonicalize NaN (select on fcmp uno), bitcast back, store,
  ; then the same musttail dispatch as above

slow:                                                     ; the `cold { }` block
  %ctl = call preserve_mostcc i32 @Mini.op.ADD.cold0(ptr %ctx, ptr %pc, ptr %regs, i64 %a, i64 %b, ptr %pdst)
  switch i32 %ctl, label %next [ i32 1, label %raise ]
  ...
raise:
  musttail call preserve_nonecc void @Mini.on_raise(ptr %ctx, ptr %pc, ptr %regs, ptr %k, ptr %tbl)
  ret void
}
```

Notes:

- `preserve_nonecc` makes every register caller-saved and uses many of them
  for arguments, so state arguments land in fixed registers and move between
  handlers without shuffling. `musttail` guarantees a jump and no stack growth.
  Every handler has the same prototype, which `musttail` requires.
- **Cold outlining** uses `preserve_mostcc` (`preserve_allcc` if the cold path
  touches FP/vector registers the handler keeps live). The callee saves
  whatever it uses, so the handler's state registers survive the call and its
  hot path needs no spills. The outlined function is marked `cold noinline`.
- A cold block returns a **control code** (`next`, `jump`, `raise`, `redo`,
  ...). Changed state variables come back in the return registers when there
  are a few (as a small struct), otherwise through `ctx.sync`. The error value
  for `raise` goes in `ctx.pending`.
- *As implemented*, an outlined cold block (one that ends in control words
  on every path) is a tail-called function with the handler's signature.
  In a superinstruction, its `next` from a part before the last tail-calls
  the super's **resume function** for the next part (`Mini.op.S.resume1`:
  the handler from that part on, with `pc` at the part's word), so that
  the rest of the super runs as it would in the handler, and as it does
  in goto and switch modes. The same goes for `next` in a shared copy of
  a `-> never` vm function called from such a part, which gets the resume
  function in a thread-local slot.
- Each handler is a separate symbol (`Mini.op.ADD`), so `perf` and other
  profilers attribute time per opcode without any extra tooling.
- **Loops in tail-call vm code** (handlers, cold blocks, vm functions)
  carry `!llvm.loop` metadata that turns off vectorization, interleaving
  and unrolling (what `#pragma clang loop vectorize(disable)
  interleave(disable) unroll(disable)` gives). Such loops run a few times
  (zeroing a callee's locals, copying constants or arguments), and the
  vectorized and unrolled versions need registers enough to give the
  handler a stack frame: the wasm interpreter's CALL with two copy loops
  was 154 instructions with `push %rbp` and spills, and is 50 without.
- **A float state variable stays in an SSE register.** `preserve_none`
  passes floating-point arguments as the C convention does (`xmm0`..), so
  `state { .., acc: f64 }` is `xmm0` in every handler: an op
  `acc = acc * a` is `mulsd (%r14,%rdx,8),%xmm0` and the dispatch, with no
  load or store of `acc` (checked with LLVM 19 on x86-64). That is
  wasm3's `fp0`. An `accumulator` declaration (language.md §7.4) adds
  such a state variable and derives the ops that read and write it.
- **The dispatch table's address.** On x86-64 an executable (`-fno-pic`)
  indexes the table as an absolute address. Whether the load folds into
  the jump depends only on how many values the tail call passes in
  registers: LLVM's x86 isel pattern for a tail call through memory
  (`X86tcret_6regs` in `X86InstrCompiler.td`) refuses to fold when the
  call has more than six register operands, a fixed count ("there
  wouldn't be enough scratch registers for base+index") made for the C
  convention's six argument registers. It does not look at which
  registers are free: under `preserve_none` both forms use the same
  scratch registers. Measured with standalone `.ll` handlers and
  `clang-19 -O3 -fno-pic` (2026-11-03):

  | register arguments (context, state, instruction word) | dispatch |
  |---|---|
  | up to 6 | `movzbl %sil,%eax; jmp *table(,%rax,8)` |
  | 7 or more | `movzbl %dl,%eax; mov table(,%rax,8),%rax; jmp *%rax` |

  The type and position of the arguments do not matter (the instruction
  word first or last, `i32` or `i64`, pointers, `double`s in `xmm`
  registers all count the same), and the fold also needs the table to be
  a constant (an `internal constant`, as the generated one is). Mini,
  MiniJS, Scheme and Lox pass 6 and get the folded form; the wasm
  interpreter passes 7 and Stak 8. The longer form costs nothing
  measurable: rewriting the wasm interpreter's 360 dispatches to
  `jmp *table(,%rax,8)` in its assembly and timing both builds
  (`bench/wasm/bench.py`, best of 3 interleaved, `-align-all-functions=6`)
  gave a geometric mean of 1.007 of the original's time (per benchmark
  0.96 to 1.03, within run-to-run noise). With nothing to win, the
  compiler neither warns about the count nor reorders or packs state to
  get under it.
  A table address passed as an argument does not fold on x86-64 at any
  count (`mov (%r14,%rax,8),%rax; jmp *%rax`), which is one reason the
  table stays a global there.

  On AArch64 a global's address takes `adrp` and `add` in every handler,
  since nothing hoists them across tail calls. So in tail-call mode on
  AArch64 the table's address is a hidden state variable (the `__tbl`
  that variants use, section 7), set by the entry and kept in a register
  by every handler; dispatch is

  ```
  ldr  w26, [x21, #4]!          ; next word, pc += 4
  and  x8, x26, #0xff           ; opcode
  ldr  x0, [x25, x8, lsl #3]    ; table in x25
  br   x0
  ```

  four instructions instead of six. `preserve_none` has about twenty
  argument registers on AArch64, so the extra one costs no spills: the
  wasm interpreter's handlers went from 8045 to 7174 instructions in all
  (most handlers 2 to 4 fewer; the rest is LLVM duplicating the cheaper
  dispatch into branches that jumped to a shared one), Mini's hot
  handlers mostly 2 to 4 fewer. Not timed: there is no AArch64 hardware
  here, and qemu timings mean nothing.

- **Why slow-path calls give handlers a frame (measured 2026-11-03, not
  changed).** `-W handler_frame` reports 175–218 framed tail-call
  handlers across the seven example interpreters on x86-64, most of them
  from calls to `std.gc.remember`, `std.gc.refill` or a panic on a rare
  path. The cause is stack alignment, not registers: a handler is
  entered by `jmp`, so `rsp` is 8 bytes off, and any call needs a
  16-byte-aligned stack, which costs a `push`. LLVM 19's shrink-wrapping
  can't sink that push onto the slow path, because once a block has
  touched the stack, later loads through loaded pointers and the folded
  dispatch (`TCRETURNmi64`) count as frame uses, so the frame moves to
  the entry block. A different calling convention for the callee
  (`preserve_most`/`preserve_all`) does not help by itself.

  An experiment on branch `cold-stub-experiment` (not merged) calls
  these slow paths through inline asm that realigns the stack and calls
  a `preserve_all` stub, so LLVM sees no call. It removes the frame from
  43 of 218 handlers on x86-64 and 96 of 132 on AArch64, but the
  geometric mean over 102 benchmarks was 0.995 (noise), tail-call
  `.text` grew 8–30%, and using it in goto mode made Stak 1.34× slower.
  Not merged for that reason. It could be revisited with AArch64
  hardware, where most of the frames disappeared.

  Since then (2026-11-03) the two commonest causes are gone without
  that machinery. The barrier's slow path (`std.gc.log_obj`) pushes onto
  the remembered set inline, with no call (a full set makes the next
  collection major instead of growing), and allocation in a handler can
  use `try_new`, whose refill is a cold block ending in `redo`. Framed
  handlers in the nine example interpreters went from 223 to 200
  (`-W handler_frame`): Lox 26 to 22, Mini 40 to 38, MiniJS 67 to 59,
  Python 51 to 47, Scheme 12 to 7; Prolog stays at 4 with its free-list
  workaround replaced by `try_new`; Stak 8, Wasm 15, regex 0 unchanged.
  Compiling the barrier's slow path as an implicit cold block instead
  was not built: a cold block is outlined only where the rest of its
  block ends in a control word, which a barrier inside a helper function
  (`table_set`, inlined by LLVM, not by the compiler) never is.

  The inline slow path has a cost outside handlers: each barrier site
  adds the slow path's instructions (where there was one call) to LLVM's
  inlining cost of the function it is in, so a small helper
  with a few barriers can grow past the inlining threshold and become a
  call. Mini's thread switch (`coro.enter`, `coro.suspend`, called from
  `switch_in`/`switch_out`) did, which made chameneos and coroutine_ring
  6% slower in tail-call mode (no collections run in them). `#[inline]`
  on the two restored tail-call mode but made switch mode 7-10% slower
  (more code in its one big dispatch function). What Mini does instead:
  it stores the references through raw pointers (no compiler barrier)
  and logs the objects itself, one `gc.needs_barrier` test per object
  and an out-of-line `gc.remember`, which keeps both functions small. The
  same commit made MiniJS's `Ctx.arr_set` (`SETINDEX`'s cold path) a call.

### `next`, `jump`, `redo`, `become`

| Word | Lowering |
|---|---|
| `next` | `pc' = pc + size(op) + size(cache)`. Load handler from `tbl[opcode(*pc')]`, `musttail call` |
| `jump t` | same, with `pc' = t` |
| `redo` | same, with `pc' = pc` |
| `become X` | `store i8 X, ptr %pc` (opcode is in the low byte, little-endian; big-endian targets adjust the offset). Would become `store atomic ... monotonic` under `#[threads(shared_code)]` (not yet implemented) |
| `exit v` | `store v` to `ctx.exit`, `ret void`. The `musttail` chain unwinds straight back to the entry function. *As implemented:* a result that is a scalar (an integer, pointer, float or non-boxed value) is returned in registers, and handlers return the entry's type; any aggregate result (a `Result`, struct, tuple, option of a non-pointer) is stored to a thread-local `Vm.ret_slot` and handlers return `void`, since a `musttail` call cannot return through a hidden `sret` pointer. The entry loads the slot after its call returns |
| `raise e` | `store e` to `ctx.pending`, `musttail call @Mini.on_raise` |

**Inline cache fields** are places at `pc + 1 + offset` words. A field
wider than a code word (a pointer in a `u32` vm) is loaded and stored
with the word's alignment, `load ptr, ptr %p, align 4`, which x86-64 and
AArch64 execute as one unaligned access. The size of each cache is a
constant of the generated source (`Vm__CW_PropIC`, from `size_of` of the
field types), so encoders, `op_size`, the verifier and `optimize` agree
with the handlers.

**Decode-ahead** only applies when every path through the handler ends in
`next` without writing code. Handlers that `jump`, or that may `become`,
dispatch after their body.

### `-> never` vm functions

A call on a hot path inlines the function's body; its control words then
lower like the caller's own. In tail-call mode, a call from a `cold` block
stores the arguments and the caller's instruction size (for `next`) in
thread-local slots (`@"Vm.fn.NAME.a0"`, ...) and makes a `musttail` call to
`@"Vm.fn.NAME"`, which has the handler signature, so all cold callers share
one copy and the state never leaves its registers. The shared copy is
`noinline` only under `#[inline(never)]` (otherwise LLVM may inline it
back into its callers); such a copy that never reads the calling
instruction's word (no `raise`, `?`, cold block, shared call or chain in
it) is passed `poison` for it, so the cold blocks calling it do not need
the word passed in from the handler.

## 4. Entry points

```llvm
define { i64, i64 } @Mini.run(ptr %ctx, i64 %closure) {      ; C ABI (design; entries are internal today)
  ; run `entry run` as ordinary code to set up pc/regs/k, then:
  call preserve_nonecc void %first_handler(ptr %ctx, ptr %pc, ptr %regs, ptr %k, ptr %tbl)
  ; returns here when some handler does `exit` or `fail`
  ; read ctx.exit / ctx.pending, return { status, value }
}
```

`Mini.call` (reentrant) is the same function with a nested-entry marker pushed
on the frame stack. It counts C-stack depth and fails with a guest
stack-overflow error past the limit.

**As implemented: nested entries.** An entry declared `uses reenter` is
lowered like any entry: in tail-call mode a function that sets up the
state and makes one ordinary `preserve_none` call into the first handler
(so it saves the callee-saved registers the handlers clobber), and
returns what the `exit`ing handler returned or left in `ret_slot`; in
goto and switch modes the entry's own loop function with every handler
inlined, called recursively when runs nest. The nested loop therefore runs on the C stack of the
native that called it, above the outer loop's frames, and its `exit`
returns to that native. Two additions:

```llvm
define internal { i1, i64, i64 } @"vm.Mini.call"(ptr %p0, ptr %p1, ptr %p2) {
start:
  %depth.saved = load i32, ptr @"vm.Mini__depth"     ; thread_local
  %depth.inc = add i32 %depth.saved, 1
  store i32 %depth.inc, ptr @"vm.Mini__depth"
  ...
  store i32 %depth.saved, ptr @"vm.Mini__depth"      ; before every `ret`
  ret { i1, i64, i64 } %r
}
```

- `Vm.depth` is a thread-local `u32` generated with the vm's other items
  (`#[thread_local] var Mini__depth`); the entry saves it, increments it,
  and stores the saved value back before each `ret`, so early returns
  and every exit path restore it.
- Calls to the entry have the `reenter` effect (§9).

The frame sentinel (which frame's return leaves the run), the frame
placement and the depth limit are the interpreter's code (`language.md`
§10). What must not break is the compiler's own per-thread state: cold
captures, `-> never` arguments and `on raise`'s argument are loaded from
their thread-local slots at the top of the function that receives them,
before it can call anything, and `exit` stores `ret_slot` just before
the chain returns to its entry, so a nested run in between never
overwrites a slot that an outer handler still has to read.

Measured on Mini (`bench/reenter/run.sh`: `table.sort` on 200,000
integers, 3.8M comparisons, best of 7 on a loaded machine): the native
sort comparing with `<` takes 13 ns per comparison; with a Lua
comparator, which costs a nested run each (closure call, `call_value`'s
checks and argument copy, the entry and its register saves, the
comparator's few instructions, `exit` through `ret_slot`), 33 ns; the
same algorithm written in Lua, where the comparison is an ordinary
`CALL` inside the one loop, 27 ns. A nested run costs a few tens of
cycles beyond the work it does, so a native higher-order builtin does
not beat a prelude version unless the native part saves more than that
per callback: Scheme's and MiniJS's preludes remain the faster choice
for `map`, `for-each` or `sort` with cheap callbacks. Re-entry is for
what cannot stay in the loop: C natives and embedders calling back,
metamethods invoked from runtime functions, and builtins whose own work
dominates.

**As implemented: libraries.** The interpreter writes its C API as
`export "C"` functions (external linkage, C calling convention, `bool`
`zeroext`); everything else stays `internal`, including `loimi.argc` and
`loimi.argv`. A library build (`--lib`: `clang -c -fPIC` then
`llvm-ar rcs`; `--shared`: `clang -shared -fPIC`) emits no `main`
wrapper. There is no init function: all globals have static
initializers, and runtime thread-locals start zeroed per thread.

`std.rt.protect` is `@loimi_protect`, a `noinline` function that links a
handler `{ prev, shadow_top, jmp_buf }` into the thread-local
`@loimi.handler_top`, calls `_setjmp` (`returns_twice`), and calls the
protected function; `@loimi_throw` `longjmp`s to the top handler, which
restores `@loimi.shadow_top` and the handler chain.

**TLS model.** The runtime's thread-locals (`@loimi.shadow_top`, the
`cold` capture slots `@"Vm.op.X.cN"`, `-> never` argument slots, the
result and raise slots, `@loimi.handler_top`) are emitted
`thread_local(initialexec)`. In an executable or a static archive linked
into one, that is one `%fs`-relative access. A shared object with
initial-exec TLS is flagged `STATIC_TLS` and can only be `dlopen`'ed if
the C library has static TLS space to spare (Mini uses 328 bytes; glibc
keeps 512 spare by default). So `--shared` rewrites them to
`thread_local(localdynamic)` (`--tls=` chooses). The access then calls
`__tls_get_addr`. In handlers those accesses are only on the way into a
`cold` block, but a handler that contains any call needs a stack frame,
which the fast path then pays for too. Measured on Mini's benchmarks
through `examples/embed/minirun.c` (minimum of 5 runs, on a loaded
8-core machine; relative to the `-fno-pic` executable):

| Benchmark | static archive, PIE host | `--shared`, initial-exec | `--shared`, local-dynamic | local-dynamic + TLSDESC |
|---|---:|---:|---:|---:|
| closures | 1.05 | 1.03, 1.04 | 1.14, 1.06 | 1.13, 1.03 |
| binary_trees | 0.98 | 0.94 | 0.92 | 0.94 |
| nbody | 1.05 | 1.08 | 1.07 | 0.96 |
| fib | 1.02 | 1.03 | 1.05 | 1.06 |
| oo_methods | | 1.00 | 1.07 | 1.04 |
| sort | | 1.02 | 1.03 | 1.02 |
| string_build | | 1.00 | 0.99 | 0.96 |

(Two numbers: two separate runs.) Run-to-run noise on this machine is
about ±5%, so the local-dynamic cost is 0–7% on call-heavy code
(`closures`, `oo_methods`) and not measurable elsewhere. TLSDESC
(`-mtls-dialect=gnu2`, tried by hand) replaces the `__tls_get_addr` call
with a cheaper descriptor call and shrinks the frame to one `push`; it
may halve the cost, but within the noise, so the driver does not use it
yet. A host that links the shared object at startup (not `dlopen`) can
build with `--tls=initial-exec` and pay nothing.

Position-independent code itself (the static archive in a PIE host, or a
shared object with initial-exec TLS) costs nothing measurable: dispatch
loads the table address with one `lea` instead of an absolute
address.

Thread-locals rather than plain globals cost nothing either. An
initial-exec access is `mov %fs:OFF, reg`, the same one instruction as
an absolute address. Measured (2026-11-03) by building the same IR with
`thread_local(initialexec)` deleted, so that every runtime thread-local
(the shadow-stack top included, touched at every call that may collect)
is a plain global; `--align-functions=64`, best of 3 interleaved runs,
geometric mean of plain-global time over TLS time: Mini 1.009 (14
benchmarks, all within 0.99–1.05), MiniJS 0.998 (14, 0.96–1.02), Scheme
1.000 (41 r7rs programs, 0.94–1.06). So executables keep their
thread-locals unconditionally, with no single-threaded build mode, and
independent states on several threads (language.md §10.1, Threads) cost
nothing when there is one.

## 5. `goto` mode

A single function with one basic block per handler. State variables are phis,
and every `next` loads the next handler's address from the labels table and
branches to one shared dispatch block, a lone `indirectbr`:

```llvm
@Mini.labels = internal constant [N x ptr] [ptr blockaddress(@Mini.run_goto, %op.MOVE), ...]

op.ADD:
  ; ... body ...
  %h = load ptr, ptr (gep @Mini.labels, opcode(%pc1))
  %h1 = call ptr asm "", "=r,0"(ptr %h)    ; opaque copy, see below
  store ptr %h1, ptr %target               ; a phi in %dispatch after SROA
  br label %dispatch

dispatch:
  %t = load ptr, ptr %target
  indirectbr ptr %t, [label %op.MOVE, label %op.ADD, ...]
```

The machine code is threaded nonetheless: LLVM's tail duplication copies the
dispatch block into every handler *after register allocation*, so each
handler ends in its own `mov table(,%rax,8),%rax; jmp *%rax`. The driver
passes the LLVM options that make it so (§12; only when the program has a
goto-mode vm):

- `-tail-dup-pred-size`, `-tail-dup-succ-size` raised: since LLVM 18, tail
  duplication skips blocks with more than 16 predecessors and 16 successors
  at once, which the dispatch block always has;
- `-tail-dup-indirect-size=0`: no copies *before* register allocation;
- `-tail-dup-placement=false`: no tail duplication during block placement
  (it would find nothing left to copy for dispatch, but searching costs time
  cubic in the op count).

The empty `asm` keeps the address computation in the handlers. Without it,
LLVM folds the identical table loads (and the `pc` increments) of all
handlers into the shared block, which then has five instructions, more than
post-RA tail duplication copies: dispatch stays a single shared indirect jump,
16% slower on the wasm benchmarks (below).

*Why not replicate in IR, or copy before register allocation.* Until
2026-11 the IR itself ended every `next` in its own `indirectbr`. With `S`
dispatch sites and `H` handlers, that CFG has `S×H` edges, and SSA gives
every handler a phi per state variable with an entry per dispatch site:
`S×H×K` phi operands for `K` state variables, which every IR pass (loop
canonicalization, GVN), PHI elimination, the register coalescer and block
placement pay for. Copying the shared block before register allocation (the
default for small indirect-branch blocks) recreates the same phis in machine
IR. After register allocation there are no phis: the state lives in the same
registers in every handler, and the copies are a few hundred `jmp *%rax`.
Measured with clang 19 on an i7-6700 (wall time and peak RSS of the clang
step for the synthetic vms, of the whole `loimi build --release` for wasm;
the synthetic vm has `N` ops of one to three lines each, five state
variables):

| vm (dispatch targets) | `indirectbr` per `next` | shared block, copied after RA |
|---|---|---|
| synthetic, N = 400 | 21 s, 256 MB | 1.5 s, 126 MB |
| synthetic, N = 800 | 157 s, 707 MB | 4.7 s, 183 MB |
| synthetic, N = 1600 | (~7x per doubling: ~20 min, over 2 GB; not run) | 24 s, 367 MB |
| wasm (670: 382 ops + 287 accumulator variants) | 237 s, 970 MB | 17 s, 220 MB |
| wasm with every eligible op accumulated (1170) | out of 2.5 GB | 39 s, 325 MB (tailcall 28 s, switch 27 s) |

The old form grew about 7x per doubling of the op count; the new one about
3-5x at these sizes (still superlinear: the machine CFG has the same
`S×H` edges, and block placement and the IR passes that remain are not
linear in them), but a thousand-opcode vm now builds in goto mode about as
fast as in the other modes. In the old form the time went to block
placement (64 s on wasm, cubic: its tail duplication considers every
predecessor of every successor), the machine scheduler (33 s), loop
canonicalization (23 s across IR and machine IR), GVN (17 s), loop
unrolling, vectorization and load elimination (14 s), the coalescer and
PHI elimination. Copying before register allocation (the shared block
without the `asm`, with the size limits lifted) left 56 s of which the
coalescer, early tail duplication and PHI elimination took 34 s.

The generated code got faster, too: the handlers are register-allocated
around one join point, where every state variable has one home register.
Old against new, `--align-functions=64`, best of 3, interleaved: wasm 0.87
of the time (geometric mean of the seven benchmarks; every one faster but
binarytrees, +4%), Mini 0.97 (fib +4 to 7%, closures +2%, fannkuch -13%),
MiniJS 0.99, Scheme 0.99 (41 r7rs programs). A single shared indirect jump
(no copies at all) is 1.16 on wasm.

The old form also crashed LLVM 19's LoopSimplifyCFG (at `-O1` and `-O3`)
on about 1% of the rooting fuzzer's programs (`tests/fuzz/rootgen.rs`;
`tests/run/goto_loop_simplifycfg.lm` is seed 5026); the IR verified. The
shared dispatch block does not trigger it.

`cold` blocks stay inline and are marked unlikely. Register pressure is
shared across all handlers, which is why `tailcall` is preferred where it is
available.

## 6. `switch` mode

```
loop: %opc = load opcode(%pc); switch %opc [ ... ]
```

State variables are loop phis. This mode is used for wasm without tail calls,
and works anywhere. On wasm with tail calls, `tailcall` mode lowers to
`return_call_indirect` and the calling convention is ignored.

On x86-64 the switch becomes one shared dispatch block (load the word,
compare the opcode with the largest case, `jmp *table(,%rax,8)`): LLVM
does not copy it into the handlers, since it has more than 16
predecessors and successors and more instructions than post-RA tail
duplication copies. The state stays in registers (checked on the wasm
interpreter: no spills in the hot handlers, the `f64` accumulator in
`xmm6` and the integer one in `r15` across the whole loop).

The driver turns off LLVM's tail merging when the program has a
switch-mode vm (`-enable-tail-merge=false`, §12). With it, branch folding
kept one copy of the handlers' common tails (store a result to a
register slot, `pc += 1`, branch to the switch), so many handlers took
two to four unconditional jumps to reach the dispatch block; Lox's EQK
took four. Measured 2026-11-03 (best of 3, interleaved,
`--align-functions=64`), switch builds with tail merging off against on:
wasm 0.79 of the time (geometric mean of 7; mandelbrot and nbody 0.73),
Lox `equality` 0.71 (other Lox benchmarks within 4%), Mini 0.93, MiniJS
0.95, Stak 0.98, Scheme 1.00 (geometric means; results.md, "Follow-up").
The single dispatch site remains: wasm switch still takes about 1.4
times the tail-call time (1.5–1.6 on mandelbrot and nbody, where
handlers are three to five instructions).

## 7. Dispatch tables and variants

```llvm
@Mini.table.default = internal constant [256 x ptr] [ptr @Mini.op.MOVE, ..., ptr @Mini.op.INVALID, ...]
@Mini.table.count   = internal constant [256 x ptr] [ptr @Mini.count.MOVE, ...]

define internal preserve_nonecc void @Mini.count.ADD(ptr %ctx, ptr %pc, ptr %regs, ptr %k, ptr %tbl) {
  ; `before { ctx.stats.ops[op.id] += 1 }`
  musttail call preserve_nonecc void @Mini.op.ADD(ptr %ctx, ptr %pc, ptr %regs, ptr %k, ptr %tbl)
  ret void
}
```

The inner handler dispatches through `%tbl`, which still points at the variant
table, so the interpreter stays instrumented until the table is switched back.
When a vm declares no variants, `%tbl` is a state register only in
tail-call mode on AArch64 (§3, "The dispatch table's address"); elsewhere
handlers address the default table as a global constant, which frees a
register.

In goto and switch modes each entry is one function holding every
handler, and a wrapper is a block that runs `before` and branches to the
op's block. Only entries that can switch tables get the wrapper blocks:
those that say `use_variant` with a variant, or all of them when an op,
vm function or `on raise` does. In an entry that never switches, the
wrappers would be unreachable, but LLVM removes them only late, and on a
large interpreter with `wraps all` its loop strength reduction took
minutes over the IR they left behind.

Unused opcode slots point at a trap handler (`@"Mini.invalid"` as implemented). Verified code can't
reach it, so it exists only for defense in depth.

## 8. Superinstructions and families

- `super LOADK_ADD = LOADK; ADD;` becomes a handler that decodes `LOADK`'s
  operands from word 0 and `ADD`'s from word 1, inlines both bodies with
  `LOADK`'s `next` replaced by a branch, and advances `pc` by two words on its
  final `next`. LLVM then optimizes across the boundary (for example,
  forwarding the stored constant into `ADD`'s load).
  It cannot forward a register: `ADD` loads `regs[a]` after `LOADK` stored
  `regs[dst]`, two indices decoded from different words, so the load must
  stay (no alias metadata can help: when they are equal, the load must see
  the store).
- A forwarding variant of a `#[forward] super` (`MUL_ADD__f1`) is the
  same handler with `ADD`'s `a` decoded from `MUL`'s `dst` field. The two
  addresses are then the same SSA value, so GVN replaces the load with
  the stored value (a phi when several paths of `MUL` store), and the
  value stays in a register. The variant is only correct where the
  fields are equal: `optimize` checks them before choosing it, and the
  verifier rejects it otherwise. Links also join two views of one slot
  (`ri = *((regs + it) as *i32)`, `ru = .. as *u32`, `rd = .. as *f64`):
  the store and the load then have the same address, and GVN forwards
  the value across the type change when the sizes match (a 4-byte store
  does not forward into an 8-byte load; the variant is still correct).
  The wasm interpreter relies on this (`I32_ADDK; F64_LOAD` links `ri`
  to `ru`, `F64_SUB; F64_STORE` links `rd` to `rq`).
- Family members are ordinary ops that share a format. `become` stores the
  opcode byte (section 3). In a super's part, it stores the part's own word
  and the super's first word (the opcode of the super with that part
  replaced, chosen at compile time, or of the first part).

## 9. Effects and sync points

At a call to a `uses gc` function from a handler or cold block:

```llvm
store ptr %pc,  ptr %ctx.sync.pc
store ptr %top, ptr %ctx.sync.top          ; top = regs + proto.nregs, or sp
store i64 %live_value_1, ptr %ctx.tmproots[0]
call @heap_collect_slow(...)
; with a moving heap: reload %live_value_1 from ctx.tmproots[0]
; with `reenter`: run the vm's `reload` block (stores to the state slots)
```

As implemented, the stores before the call are the vm's `sync` block and
the code after a `reenter` call is its `reload` block, both lowered
inline at the call site like the handler's own code: in tail-call mode
the state lives in `alloca`s that mem2reg turns into SSA values, so
`reload`'s assignments simply become the values later code uses. A
`reenter` call in a vm without `reload` is a compile error.

The call's shadow-stack frame (below) lists the handler's traced locals
and, after them, one fresh `alloca` per state variable of a traced type,
stored just before the call:

```llvm
%tos.copy = alloca i64                     ; one per call site
store i64 %tos, ptr %tos.copy              ; %tos: the state variable's value
; frame entries: { ptr %local.slot, @mark.T }, ..., { ptr %tos.copy, @mark.Value }
```

The state variable's own `alloca` never escapes, so mem2reg keeps it in
SSA form in every dispatch mode; in goto and switch mode that slot is
shared by the whole loop function, which a frame entry pointing at it
would have pinned to memory in every handler.

Calls without effects get none of this. In ordinary (non-handler) functions,
live `Gc` locals at `gc` call sites are stored in an `alloca`'d shadow-stack
frame `{prev, count, slots[n]}` that is linked into `ctx.roots` on entry and
unlinked on exit. The frame is only materialized on paths that reach a `gc`
call. It lists the traced locals in scope that are assigned on every path
to the call: a `let` from after its initializer, a pattern binding from
after the test that matched it (not in `let PAT = e else { .. }`'s else
block, nor in a later `match` arm), so no slot is scanned before it is
written. While a `match` guard runs, the frame also lists the
scrutinee's slot (the copy the match tests), since arms after a failing
guard still bind from it and a temporary scrutinee (`match f() { .. }`)
has no other root.

**Checked roots** (`loimi build --gc-check-roots`). `std.gc.mark_ptr`
trusts every pointer it is given: a root listed too early marks
through whatever the slot holds. In this mode the collector checks
every reference that generated code hands it before reading a header:

- Every `mark_ptr` call the compiler emits (the mark functions of
  shadow-stack entries, so traced locals, values kept for defers and the
  copies of vm state; `gc_mark` in root scanners; and the fields that
  `loimi_trace_object` traces) calls `std.gc.rootck_mark` instead. It
  accepts null, or the start of an allocated object of the heap being
  collected: inside one of its segments, in a chunk carved out already,
  at a slot boundary, not a free slot; or one of its large objects. A
  `value` is checked only in its pointer variants (an immediate is
  never dereferenced). Anything else aborts:

  ```
  gc check-roots: bad reference 0x0101010101010101: 0x01 bytes: an unassigned slot, or read from a freed object
    root #1 of shadow-stack frame #0 (0: the innermost) of
    main.victim at tests/run/gc_roots_let_else.lm:35:9 (call of main.churn); roots: #0 x: main.E; #1 n: Gc<main.Node>;
  ```

  To name the root, each frame gets a first entry `{ @info, @loimi_rootck_frame }`
  whose "mark function" stores `@info` (a constant `{ str, kind }`
  naming the function, call site and its roots) in a thread-local and
  resets a slot counter, which each root's mark function increments.
  `loimi_trace_object` and `gc_mark` set the same thread-local to their
  own record (the object's type and address; the `gc_mark` call), so a
  bad heap reference is reported with the object that holds it.
- Unassigned slots are made recognizable instead of holding whatever
  the stack held: each traced local's slot is filled with 0x01 bytes on
  function entry and again where its scope ends (on the path that falls
  out of the scope). A frame that lists a slot too early, or too long,
  shows 0x0101010101010101, a non-canonical address that no heap holds.
- Freed objects are made recognizable: `new` calls `std.gc.rootck_alloc`,
  which registers (once per heap) a sweeper that fills every object
  about to be freed with 0x01 bytes and forgets where they started, in
  both heap modes (in a minor collection old objects count as marked:
  a small object's granules keep their mark bits, `is_marked` accepts a
  large one). A reference that outlived its object is then reported
  when it is marked ("a freed object"), and a traced field read from it
  reads 0x01 bytes, reported as above.

The `rootck_*` functions of `std.gc` and the externs they use are
emitted only when this mode references them (`is_lazy` in
src/codegen.rs), so a default build's IR does not change. The checks
assume that the shadow stack holds only objects of the heap being
collected: with several heaps on one thread, a frame of a nested run
into another heap is reported. Not combinable with `--jit-stencils`.
The rooting fuzzer (`fuzz_gc_roots` in tests/fuzz.rs, generator in
tests/fuzz/rootgen.rs) builds its programs in this mode.

## 10. GC barriers and allocation

- `obj.field = v` with a `Gc`/`Value` field becomes a store plus the policy's
  `barrier(obj, &field, v)`, an `alwaysinline` function. For `MarkSweep` it is
  empty and disappears.
- `heap.new(T { ... })` inlines a bump or size-class fast path:
  `p = tlab.cursor; if p + size > tlab.limit { cold { refill() /* uses gc */ } }`.
  Header and field initialization follow. Only the `cold` refill carries
  effects, so only the refill path syncs.

As implemented, the barrier is not a policy parameter: it is always
compiled in, and the heap decides at run time whether it does anything.
After a store of a traced value (`Gc`, a `value` type with reference
variants, or an option, tuple, struct or array holding one) into a place
inside an object, the compiler emits `call @std.gc.barrier(ptr %obj)`.
The place is inside an object when it is a field reached through a `Gc`
(`o.f`, also `o.pair.a` and `o.arr[i]` through value-type fields), an
element of a trailing array (`o.items[i]`), or unchecked indexing of a
trailing array's pointer (`o.items.ptr[i]`, recognized syntactically).
A call that takes the address of such a place of a traced type as an
argument (`o.pair.set(x)`, which auto-refs `&o.pair`, or `f(&o.pair)`)
is followed by the barrier on the object, since the callee may store
through it. So is a call that passes `o.items` (or a sub-slice) of a
traced element type to a parameter that the callee may store references
through (`src/objref.rs` summarizes each function's slice and pointer
parameters; the callee must not collect, and such slices cannot be kept
elsewhere: `design/safety.md` section 8).
Other stores through raw pointers get no barrier: the code that makes
them calls `gc.write_barrier(h, obj)` (see
[`stdlib/gc.md`](stdlib/gc.md#as-implemented)), or the object is pinned
(`h.pin_dirty(obj)`), which makes every collection trace it.
Initializing a new object (`new(h, T { .. })`) needs none.

**Trace hooks.** `loimi_trace_object(h, obj, tid)` is a switch over the
type ids; each case marks the traced fields, then the trailing array
(its length is at `obj - 8`). For an object with `#[trace(f)]` the case
marks the fields and then calls `f(h, obj)` (an internal function, which
LLVM may inline) instead of looping over the array. Everything that
traces an object (marking, the remembered set in a minor collection,
verify mode) goes through the switch, so the hook is used everywhere;
in a `--gc-check-roots` build its `gc_mark` calls are checked like any
other.

`barrier` is `alwaysinline` and tests one header byte:

```llvm
store i64 %v, ptr %slot
%f = load i8, ptr %obj - 9            ; header flags
%log = and i8 %f, 1                   ; LOG: old, not yet remembered
br i1 %log, label %remember, label %cont   ; expected not taken
```

On x86-64 that is `testb $0x1,-0x9(%rax); jne`. The header shares a
cache line with the object's first fields, which a store has usually just
touched (a shape check before an inline-slot store, the length before an
element store). Measured cost in a non-generational heap (no object ever
has `LOG`): geomean +0.4% on Mini and +0.6% on MiniJS, at worst +3%
(MiniJS `richards`), within the placement noise (see the roadmap, open
question 2). `std.gc.alloc` writes the type id, mark and flags bytes in
one store after its refill path has merged, so for a store into an
object allocated earlier in the same function, with no other stores in
between that could alias the header, LLVM knows the flags are 0 and
deletes the barrier (Mini's `CLOSURE` has none left). The slow path (`log_obj`) is inline and makes
no call, so it does not keep the handler from being a leaf (it used to
call `remember`, which gave MiniJS's `SETPROP_IN` a `push`/`pop` pair on
the fast path too). A store of a value with no references (a payload-free
variant, `None`, literals of such: `gcgen::holds_no_refs`) gets no
barrier.

**Generational GC: moving or not.** The design question (roadmap, open
question 2) was settled for the non-moving scheme with sticky mark bits.
It needs no reloading after `gc` calls, keeps raw pointers into objects
valid (Mini's `aptr`, MiniJS's `Arr.ptr`, upvalue pointers, `k` pointers
into constant arrays), and costs one byte test per barrier. A copying
nursery would need every such pointer re-derived after each safepoint.

## 11. Bounds checks and verification

Operand accesses in handlers are emitted as `getelementptr inbounds` without
compare-and-branch: the verifier has proved them in range. Other indexing
uses ordinary checked slices. The verifier is generated Loimi source
(`src/vmexpand.rs`): a loop over the code that decodes each word with the
same field extraction the handlers use, then evaluates the `verify`
clauses pasted in. It returns `Result<Vm__Verified, Vm__VerifyError>`.

`Verified<Vm>` is the generated struct `Vm__Verified { __code: []W }`:
a slice, passed and stored as `{ ptr, i64 }`. `jump v` lowers to the
slice's pointer (`extractvalue` after mem2reg), so an entry that takes
a `Verified` dispatches exactly as one that takes a raw pointer; it only
receives the length in one more register. `v.start()` is the same
field read. The checks are all at compile time: the `__` field is
private to the vm's expansion file, `#[unsafe]` functions need an
`unsafe { }` block, and `#[require_verified]` is checked on each
entry's `jump` (vmcheck).

`Vm.Resume` (resume points) is `Vm__Resume { __pc: *W }`, one pointer.
`next_resume` is `next_pc` wrapped in it (`pc` plus the op's size, or
plus the size argument in a vm function's shared copy), and `jump r`
lowers to the field, as a raw `jump`. So Mini's resume entry
(`jump c.ypc` with `ypc: Mini.Resume`) is the same code as the
`unsafe { jump c.ypc }` it replaced, and the struct has the layout of
the `*u32` field it replaced. (Keeping it in a `?Mini.Resume` instead
made `Ctx` 8 bytes longer, which moved the fields after it and cost
3% on `fib`, whose code was otherwise identical: placement noise
that `Mini.Resume.placeholder()` avoids.) `v.resume_at(i)` is a
generated loop that steps through the code by `Vm.base_size`
(superinstruction parts are instructions)
until it reaches or passes `i`. The verifier's "may continue" scan (the
code cannot end with such an instruction) counts `next_pc` and
`next_resume` as reads of `pc`. Before resume points it missed
`next_pc`, so code that ended with a call op saving `next_pc` (Lox's
calls) passed verification. The `Vm__Resume` items are generated only
when some source file names them, so other programs' IR does not
change.

Where the proof is stored is the interpreter's choice, and that can
cost. A `Verified` field in the prototype is a `{ ptr, len }` that
CALL loads (one more dependent load per call than a trailing array,
whose address is `p + offset`): measured on Mini (`--release
--align-functions=64`, best of 6), `oo_methods` +4%, `closures` +2%,
`fib` and `binary_trees` within 1%. The same field load with a raw
`*u32` costs the same, so the cost is the layout, not the type.

`code: verified [W] by Vm` keeps the proof without the load. Reading
`p.code` builds `Vm__Verified { __code: <trailing slice> }` (a struct
value, not a place), and `jump p.code` takes the slice's pointer
directly in the checker, so it lowers to the trailing array's
`getelementptr inbounds i8, ptr %p, i64 <size>`: a `lea`. The length
(the object header's word before `p`) is loaded only where `p.code.len()`
or `words()` is used. `Vm.build_verified` is generated Loimi: `new` with
the word count, a copy loop into the array (through `__verified_words`,
a builtin that only the vm's expansion file may call, as `new` on such
an object), then `Vm.verify` over the array with the object as the
`owner` context; a `verify NAME { .. }` block is pasted after the
instruction loop, with `is_insn_boundary` reading the boundary bitmap
the first pass built. Mini went back to the trailing array: against
the `Verified`-field build, `oo_methods` -6.2% (0.826 s to 0.775 s),
`closures` -2.6% (0.496 s to 0.483 s), `fib` unchanged (best of 5,
interleaved, `--align-functions=64`).

## 12. Driver settings

| Setting | Why |
|---|---|
| `-O3` with the new pass manager (as implemented: `-O3` with `--release`, `-O1` otherwise) | standard |
| goto mode: `-mllvm -tail-dup-indirect-size=0 -tail-dup-pred-size=1000000 -tail-dup-succ-size=1000000 -tail-dup-placement=false` when the program has a goto-mode vm (§5) | the shared dispatch block is copied into every handler after register allocation; compile time near linear in the op count |
| switch mode: `-mllvm -enable-tail-merge=false` when the program has a switch-mode vm (§6) | handlers branch straight back to the switch instead of through shared tails |
| handlers `nounwind`; whole program `-fno-exceptions`-equivalent | no unwind tables in the hot path |
| `cold` functions placed in `.text.unlikely` | i-cache locality for handlers |
| optional handler ordering from `--vm-stats` profiles (not yet implemented) | hot handlers placed together |
| target check: `preserve_none` available? otherwise warn and fall back to `goto` (not implemented: both targets have it) | portability |
| executables `-fno-pic -no-pie`; `--lib`/`--shared` `-fPIC` | absolute dispatch-table addresses; libraries link anywhere |
| initial-exec TLS; local-dynamic for `--shared` (`--tls=`) | `dlopen`-safe shared objects (§4) |
| x86-64 programs with a vm: `-mllvm -x86-align-branch-boundary=32 -x86-align-branch=fused+jcc+jmp+indirect -x86-pad-max-prefix-size=5` (`--no-branch-align` drops them) | no branch, dispatch jumps included, crosses or ends at a 32-byte boundary (the JCC erratum, below) |
| `--cpu=NAME` (or `LOIMI_CPU`): `-march=NAME` on x86-64, `-mcpu=NAME` on AArch64, for every clang run of the build (the program, JIT stencils, C runtimes, `-W handler_frame`'s assembly); default none, the target's baseline (SSE2, NEON) | portable binaries by default; vector code for a newer CPU when asked (below) |

**Branch alignment.** On Skylake-derived Intel cores (Skylake to
Comet Lake, Cascade Lake; the i7-6700 here) the microcode fix for the
JCC erratum keeps any 32-byte code block with a branch that crosses or
ends at its end out of the decoded-uop cache: that code runs from the
legacy decoders. A handler is short, so one badly placed dispatch
`jmp *%rax` puts most of it there, and which handlers that hits depends
only on where the linker put them. In the wasm interpreter one such
handler (`F64_LOAD__AR`, whose dispatch jump ended exactly at a 64-byte
boundary) made three PolyBench kernels take 1.4 times as long
(cholesky 0.92 s against 0.63 s; examples/wasm/FRICTION.md, round
three), and moving every handler by 128 bytes made the effect go away.
Clang's `-mbranches-within-32B-boundaries` pads conditional and direct
jumps but not indirect ones, which are the ones that matter in a
threaded interpreter, so the driver passes the underlying options with
`indirect` added. The padding is a prefix on an earlier instruction
where one fits (no extra uop), else a nop; the binaries grow by about
1%. Measured, `--align-functions=64`, best of 5 interleaved, with against
without: wasm PolyBench 0.963 (geometric mean of 30; the three kernels
0.67-0.70), wasm tuning set 1.003, Mini 0.99 (14 programs), MiniJS 1.01
(14), per program within ±4% for the interpreters without such a
handler, both ways: the padding moves code, so other placements change.

**`--cpu`.** The driver asks clang which extensions the CPU has
(`clang -march=NAME -E -dM`: `__SSSE3__`, `__SSE4_1__`), so `native` and
every CPU name clang knows work, and an unknown name is an error before
anything is built. Two of Loimi's own vector lowerings depend on them;
everything else LLVM picks up from `-march`:

- `swizzle` with SSSE3: `paddusb` of 0x70 (indices 0..15 keep their low
  bits below 0x80, 16..255 saturate at or above it) and `pshufb`, which
  zeroes lanes whose index has the top bit set. Without it, lane by lane
  through memory.
- `floor`, `ceil`, `trunc`, `nearest` on vectors with SSE4.1: the LLVM
  intrinsics, which become `roundps`/`roundpd`. Without it, the
  add-and-subtract-2^23 sequence (language.md §1.2, "Vectors").

`--emit-llvm` output for a `--cpu` starts with a comment naming the
`-march` it needs (it may call `llvm.x86.ssse3.pshuf.b.128`).

Instructions per operation (a function taking its operands and result
through pointers, so each count includes two or three loads and a store;
clang 19, `--release --lib`, `llvm-objdump`, `ret` excluded). `native`
on the i7-6700 used here (Skylake: AVX2, no AVX-512) is the same as
`x86-64-v3`:

| operation | x86-64 | x86-64-v2 | x86-64-v3 | x86-64-v4 | AArch64 |
|---|---:|---:|---:|---:|---:|
| `u8x16.swizzle` | 105 | 6 | 6 | 6 | 4 |
| wasm `i8x16.shuffle` (two swizzles, `or`) | 209 | 13 | 12 | 12 | 9 |
| `f32x4.minimum` | 28 | 18 | 12 | 12 | 9 |
| `f64x2.maximum` | 30 | 18 | 12 | 12 | 9 |
| `i8x16 << n` | 12 | 11 | 10 | 10 | 5 |
| `i8x16 >> n` | 17 | 14 | 14 | 13 | 6 |
| `i64x2 < i64x2` | 15 | 5 | 4 | 4 | 4 |
| `i64x2 * i64x2` | 14 | 14 | 12 | 4 | 11 |
| `f32x4.floor` | 28 | 4 | 3 | 3 | 3 |
| `f64x2.nearest` | 19 | 4 | 3 | 3 | 3 |
| `f32x4.trunc` | 26 | 4 | 3 | 3 | 3 |
| `u16x8.narrow_sat(i32x4, i32x4)` | 27 | 5 | 4 | 7 | 5 |
| `f32x4 as i32x4` | 13 | 10 | 11 | 9 | 3 |
| `f32x4 as u32x4` | 18 | 16 | 17 | 13 | 3 |
| `f64x2 as i64x2` | 20 | 20 | 20 | 20 | 3 |
| `i64x2.abs` | 7 | 6 | 6 | 3 | 3 |
| `u32x4.min` | 12 | 5 | 4 | 4 | 4 |
| `u8x16.popcount` | 17 | 13 | 11 | 11 | 3 |
| scalar `f64.floor` | 1 (`jmp floor`) | 2 (`roundsd`) | 2 | 2 | 1 |

On AArch64 `-mcpu` changes none of these except `i64x2 *` (4 with SVE2,
`neoverse-v2`): NEON is the baseline and has the rest.

Interpreters gain nothing measurable from it. `x86-64-v3` against the
baseline, `bench/run_all.py` (best of 3, `--align-functions=64`, two
runs one after the other on a shared machine), geometric means of
v3/baseline per suite, tail calls: Stak 1.05 (5 programs), Lox 0.87
(10), Scheme 1.00 (43), Mini 0.99 (14); the unchanged reference
interpreters moved as much between the runs (clox 0.88, Lua 0.96,
Gauche 0.98), so all of it is noise; goto, switch and JIT builds alike.
The wasm interpreter, on its MVP and `-msimd128` benchmarks: 0.997
(examples/wasm/FRICTION.md, "Fixed-width SIMD"). Vector code that runs
`swizzle`, shuffles, rounding or `minimum`/`maximum` is where `--cpu`
pays; the default stays the baseline, so that binaries run on any
x86-64.
