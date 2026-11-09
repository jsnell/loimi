# Lowering to LLVM

How each Loimi feature becomes LLVM IR, and why it is done that way. This
is for contributors to the compiler.

The rule throughout: **stock LLVM only**. No custom passes are needed for
correctness and there is no forked backend; the driver sets a few LLVM
options, listed in section 12. The IR below is abridged and assumes LLVM
19 or later (`preserve_none` on x86-64 and AArch64).

The running example is `vm Mini` from `language.md`: fixed 32-bit
instructions and a `nanbox` value type.

## 1. Compiler pipeline

```
source ─▶ lex, parse ─▶ enum and vm expansion ─▶ type check ─▶ redo check
       ─▶ write-barrier and lifetime checks ─▶ LLVM IR (text) ─▶ clang-19
```

- **vm expansion** (`src/vmexpand.rs`) is the Loimi-specific stage. For
  each `vm` it generates Loimi source for the vm's tools: encoders,
  emitters, `op_size`/`op_name`, `optimize`, `deopt`, the verifier,
  label helpers and the disassembler. This source is type-checked like
  user code.
- **Handlers and dispatch tables** are generated directly as IR
  (`src/vmgen.rs`).
- **The output** is textual LLVM IR, which `clang-19` compiles and links.
  A large module is split into parts that compile in parallel
  (`src/splitir.rs`).

Entries are internal functions; the compiler generates no C-ABI entry
points. A C host embeds an interpreter through `export "C"` functions
written in Loimi (section 4).

## 2. Values (`nanbox`, `lowtag`, `boxed`)

### `nanbox`

A `nanbox` value is an `i64`.

- The `unboxed` variant (usually `Num(f64)`) is the double's bits. NaNs
  are canonicalized to `0x7FF8_0000_0000_0000`.
- So no double is above `0xFFF8_0000_0000_0000` (the negative default
  NaN that x86 produces from canonical inputs). Every word above it is
  free for the other variants.
- Each tagged variant owns an aligned block of that free space. Its words
  are `tag << w | payload`, where `w` is the block's width in bits and
  the payload is zero-extended in the low `w` bits.

The compiler picks one of four layouts, depending on how many variants
there are and how wide their payloads are. `--explain-values` prints the
result.

**Classic layout (at most 7 tagged variants).** Each variant gets a 48-bit
block, `0xFFF9` to `0xFFFF` in the top 16 bits, in declaration order. For
the example in `language.md`:

| Variant | Encoding |
|---|---|
| `Num(f64)` | the double's bits; always `< 0xFFF9 << 48` |
| `Int(i48)` | `0xFFF9_xxxx_xxxx_xxxx` (the integer's low 48 bits) |
| `Nil`, `Bool(bool)` | `0xFFFA_0000_0000_0000`, `0xFFFB_0000_0000_000b` |
| `Str`, `Obj`, `Fn`, `Native` | `0xFFFC`–`0xFFFF` in the top 16 bits, 48-bit pointer in the rest |

**Eight tagged variants**, when one has a payload of at most 47 bits.
Seven variants get the classic layout, in declaration order. The
narrowest (the last of that width) goes at the start of the 47-bit block
`0xFFF8_8000_0000_0000`; a variant without a payload is that one word.
`NUM` (the unboxed limit) is `0xFFF8_8000_0000_0000`.

Why not use the packed layout here: an eighth variant then leaves every
other variant's 48-bit block where it was. A tuple test that mixes
pointers with a narrow integer (`(T(t), Int(_))` with `Int(i32)`) stays
one test. Under the packed layout the `i32` would get a 32-bit block and
such tests split in two; Mini's `hash_strings`, `interp` and `queens`
ran 4–6% slower that way.

**Pinned blocks (`block N`).** A pinned variant asks for a block of `N`
bits instead of its payload width.

- The classic layout is used only if no pin is narrower than 48 bits.
- The eight-variant layout is used only if at most one pin is narrower,
  and that one takes the 47-bit block.
- Otherwise the packed layout allocates the pinned widths.

Mini pins `Int(i32) block 48` and `Nil block 0`. That gives the same
eight-variant layout as without pins, and a ninth variant would move only
unpinned narrow variants. After choosing, the compiler warns
(`value_layout`) about every unpinned variant whose block is narrower than
48 bits, so a layout change caused by a new variant is never silent.

The constants: `V.X.mask` is `!(2^w - 1)`, `V.X.tag` is `T << w` (with
`w` = `V.X.block`), and `V.Num.limit` is `NUM`.

**Packed layout (more than 8 tagged variants, or 8 with no payload under
48 bits).** Each variant needs a block of 2^w words, where `w` is its
payload width: 48 for pointers, `N` for `iN`/`uN`, 1 for `bool`, 0 for no
payload. The free space is:

- the seven 48-bit blocks `0xFFF9..0xFFFF`;
- the 47-bit block `0xFFF8_8000_0000_0000..0xFFF8_FFFF_FFFF_FFFF` (the
  upper half of `0xFFF8`, which no canonical double reaches).

A buddy allocator places the variants, widest first. Each takes the
smallest free block that fits (lowest address on ties) and splits it in
halves down to its size. From `tests/run/values_nanbox_packed.lm`:

| Variant | Width | Block | Encoding |
|---|---|---|---|
| `Num(f64) unboxed` | | | `< 0xFFF8_8000_0000_0000` |
| `Int(i48)`, `P`, `Sym`, `Str`, `Vec`, `Clo` (pointers) | 48 | `0xFFF9`..`0xFFFE` | `0xFFF9_xxxx_xxxx_xxxx`, ... |
| `Small(i47)` | 47 | the `0xFFF8` upper half | `0xFFF8_8000_0000_0000 \| x` |
| `Char(u32)` | 32 | low end of `0xFFFF` | `0xFFFF_0000_xxxx_xxxx` |
| `Prim(u16)` | 16 | | `0xFFFF_0001_0000_xxxx` |
| `Flag(bool)` | 1 | | `0xFFFF_0001_0001_000b` |
| `Nil`, `F`, `T`, `Unspec`, `Eof` | 0 | | `0xFFFF_0001_0001_0002` .. `0006` |

**Shared layout (more 48-bit needs than blocks).** The packed layout fits
at most seven variants that need a whole 48-bit block (pointers and
48-bit integers). Beyond that, `Gc` variants share 48-bit blocks, up to 8
per block.

- GC objects are at least 8-byte aligned (`std.gc`'s are 16-byte
  aligned), so the low 3 bits of a `Gc` pointer are free.
- A variant in a shared block is told apart by a 3-bit *low tag* `L` in
  those bits. Its word is `T << 48 | ptr | L`.
- Its mask covers both tags, `0xFFFF_0000_0000_0007`, and `V.X.block` is
  45 (the pointer bits between them).
- Raw pointers (`*T`) never share, since their alignment is not known.

Which variants share: those pinned `block 45`, then as few unpinned `Gc`
variants as make the declaration fit, last declared first. Each group of
8 sharing variants is one 48-bit request to the same buddy allocator, so
everything else is placed as in the packed layout. Low tags are given in
declaration order within a block. A declaration that needs no sharing
keeps its packed, classic or eight-variant layout exactly. From
`tests/run/values_explain_shared.lm`:

| Variant | Block | Encoding |
|---|---|---|
| `Int(i48)`, `Str`, `Pair`, `Env`, `Raw(*Raw)`, `Vec` | 48 | `0xFFF9`..`0xFFFE` in the top 16 bits |
| `Sym(Gc<Sym>) block 45`, `Clo(Gc<Clo>)` | 45 | `0xFFFF_0000_0000_0000 \| ptr \| 0`, `.. \| ptr \| 1` |
| `Char(u32) block 32`, `Nil`, `F`, `T` | 32, 0 | in the 47-bit block |

How patterns on a shared block compile:

| Pattern (shared block) | Test / code | x86-64 |
|---|---|---|
| `v is Sym` | `(v & M) == (T << 48 \| L)`, `M = 0xFFFF_0000_0000_0007` | `movabs; and; cmp` (constants hoisted in loops) |
| `match v { Sym(_) => .., Clo(_) => .., ... }` | LLVM switches on `(v & M) - T << 48`: a jump table over `L` | `and; add; cmp; ja; jmp *` |
| `(a, b) is (Sym, Clo)` | `((a ^ tagS) \| (b ^ tagC)) & M == 0` | one test |
| `(a, b) is (Str, Clo)` (whole-block `Gc` with shared) | the same, with the shared `M`: a whole-block `Gc` pointer's low bits are 0 | one test |
| payload | `inttoptr(v & 0xFFFF_FFFF_FFFF) - L`, the `- L` a `getelementptr` | `and`; then `mov 7(%r)` for field offset 8 and `L = 1` |
| `Sym(p)` | `(p & 0xFFFF_FFFF_FFFF) \| (T << 48 \| L)` | `movabs; or` |

The GC marks a value word through the same payload extraction, so it sees
the untagged pointer (`--gc-check-roots` checks exactly the object start).
The JIT's stencil variants per constant tag (`--jit-stencils`) use the
same mask and tag. AArch64 needs no top-byte-ignore, because the tags are
stripped before every dereference.

*Cost.* In a microbenchmark that matches four pointer variants and an
`Int` and reads a field, four variants in one shared block (18 ms) were
as fast as whole blocks (20 ms). The match becomes one jump table, and a
test is one instruction more than `shr; cmp`.

*Why not a type id in the object header.* The other way to have more
pointer types than tags is one variant whose object header carries the
type (a `kind` field, like CPython's `ob_type`). That needs no encoding
bits, but:

- every test is a dependent load before the branch (26 ms in the
  benchmark above);
- the objects need a common header layout that the compiler would have
  to own;
- a tuple pattern cannot merge such tests into one compare.

Low tags cost a mask on the test and nothing on the payload. Every
variant stays an ordinary variant, so patterns, tuple merging, GC
tracing, the JIT and `--explain-values` work unchanged.

**Pattern tests** in general (`T` is the variant's tag, `w` its block
width; `NUM` is the lowest tagged word, `0xFFF9 << 48` in the classic
layout and `0xFFF8_8000_0000_0000` if the 47-bit block is used):

| Pattern | Test | x86-64 |
|---|---|---|
| `v is Num` | `v <u NUM` | `cmp; jae` |
| `v is Int` | `(v >> w) == T` | `shr; cmp` |
| `v is Nil` (no payload, packed) | `v == T` | `cmp` |
| `(a, b) is (Int, Int)` | `((a ^ T<<w) \| (b ^ T<<w)) >> w == 0` | `xor; xor; or; shr; jne` |
| `(a, b) is (Num, Num)` | `umax(a, b) <u NUM` | `cmp; cmova; cmp` |
| payload of a pointer variant | `v & (2^w - 1)` | `and` |
| payload of an `iN` variant | `trunc v to iN` | free |

Tuple elements whose tags sit in the same position (same `w`) merge into
one test. Elements with different widths are tested separately and the
results and-ed. So a tuple pattern of `k` values usually costs one test,
not `k`. `--explain-values` lists the variants by block width: each line
is one test.

**Float reads of a value slot.** A float-first fast path can add two slots
as doubles before testing anything: a tagged word is a NaN, so a non-NaN
sum means both were numbers.

```
f64.from_bits(a as u64) + f64.from_bits(b as u64)
```

The checker turns `from_bits` of a place reached through a pointer
(`regs[it]`, `*p`, optionally behind a bit-preserving cast to `u64`) into
a load of the place as `double`. Why: as `load i64` plus `bitcast`, LLVM
keeps the integer load (GVN merges it with the slow path's integer read
of the same slot) and moves it to an XMM register. That is two more
instructions on the fast path, and Mini's mandelbrot and nbody ran 4–5%
slower. With the `double` load, `addsd` reads memory and the slow path
does the `movq` instead.

### `nanbox(int_high)`

The layout is chosen as above, and then every word is stored rotated:
layout word `c` is stored as `rotl(c, 16) ^ k`. Here `k` is the 16-bit tag
of the integer variant's 48-bit block (the first variant with an integer
payload and a whole 48-bit block; `0xFFF9` in the classic layout). This is
a bijection, so everything the layout guarantees still holds. What
changes is where the bits are:

| Variant | Word | Test | Payload |
|---|---|---|---|
| `Int(i48)` (the `k` block) | `x << 16` | `(w & 0xFFFF) == 0` | `ashr exact w, 16` |
| a pointer with a whole block `T` | `p << 16 \| (T ^ k)` | `(w & 0xFFFF) == T ^ k` | `w >> 16` |
| a shared block `T`, low tag `L` | `(p \| L) << 16 \| (T ^ k)` | `(w & 0x7FFFF) == ..` | `(w >> 16) - L`, the `- L` in the load's displacement |
| narrower blocks (packed) | the payload at bit 16 | `(w & rotl(mask, 16)) == ..` | `w >> 16`, truncated |
| `Num(f64) unboxed` | `rotl(d, 16) ^ k` | `rotr(w ^ k, 16) <u NUM` | `rotr(w ^ k, 16)`: `xor; ror` |

`V.X.mask` and `V.X.tag` are the rotated mask and tag. Tuple tests merge
as before: `(Int, Int)` is `(a | b) & 0xFFFF == 0`, which is
`mov; or %si, %bx; je` on x86-64.

The `exact` on the integer payload is what removes the shifts:

- The checked add on `i48` shifts its operands left by 16 (see
  "Overflow-checked arithmetic on odd widths" below), and LLVM folds
  `shl(ashr exact(w, 16), 16)` back to `w`.
- Re-tagging `Int(r)` is `r << 16`, which folds into the `ashr exact` of
  the sum.

Python's `ADD` is `match (a, b) { (Int(x), Int(y)) => if let Some(r) =
x.add_checked(y) { dst = Int(r); next }, (Float(x), Float(y)) => .. }`.
Its int path, operand decoding and dispatch included, went from 28
instructions to 18:

```asm
mov    %edx, %ebx
or     %si, %bx           ; (a | b) & 0xFFFF == 0: both Int
je     .Lint
...                       ; the float arm: xor; rol each, umax test, addsd, rol; xor
.Lint:
add    %rsi, %rdx
jo     .Lcold
mov    %rdx, (%r14,%rcx,8)
```

Other operations:

- `MUL` is `sar $16, %rsi; imul %rsi, %rdx; jo` (one operand shifted, as
  for odd widths).
- `LT` is a word `cmp`.
- `x as f64` is `sar; cvtsi2sd` (a plain `i48` needs `shl; sar` first).
- Pointer payloads are `shr $16` instead of an `and` with a 48-bit mask
  that needs a `movabs`. Python's handlers are 5% smaller overall.

Floats pay: two instructions to decode each operand and two to encode the
result, and the test decodes before the `umax`. In Python's `ADD` the
float path still only went from 31 instructions to 32, because the int
test in front of it got shorter. Per-benchmark numbers are in
`design/speed-audit.md`, item 7.

Other effects of the rotation:

- The GC marks through the same payload extraction.
- The JIT's stencil variants per constant tag use the rotated mask and
  tag. The float gets no stencil variant, since its test is not a mask
  test of the stored word.
- `#[scan_raw_words]` is refused: a raw pointer's low 16 bits can be any
  variant's tag.
- The all-zero word is `Int(0)`, not `0.0`. That is what zero-filled
  trailing arrays hold.

### `lowtag`

The word is `payload << s | tag`, where `tag` has `s` bits.

- With at most 8 variants, `s = 3` and the tag is the variant's index.
  Pointers (8-byte aligned) are or-ed with it; integers of up to 61 bits
  are shifted by 3.
- With more, pointers and 61-bit integers take 3-bit tags. A variant with
  an `N`-bit payload gets a tag of `64 - N` bits, allocated from what is
  left by the same buddy allocator (splitting a tag appends a 0 or a 1
  bit).
- `block N` asks for a tag of `64 - N` bits, and a pin below 61 selects
  this layout.

The constants: `V.X.mask` is `2^s - 1`, `V.X.tag` the tag, `V.X.block` is
`64 - s`.

| Pattern | Test |
|---|---|
| `v is Int` | `(v & (2^s - 1)) == T` |
| `(a, b) is (Int, Int)` | `((a ^ T) \| (b ^ T)) & (2^s - 1) == 0`; for `T = 0`: `(a \| b) & 7 == 0` |
| payload of a pointer variant | `v & -8` |
| payload of an integer variant | `ashr exact (v ^ T), s` |

The `exact` flag (the shifted-out bits are known to be zero) lets LLVM
cancel the shift against the `<< s` of overflow-checked arithmetic and of
re-tagging. An `Int(i61)` add with tag 0 is `or; test; jne; add; jo;
store`, with no untagging at all.

### Overflow-checked arithmetic on odd widths

This covers `add_checked`/`sub_checked`/`mul_checked`, and the checked
`+ - *` of debug builds, on `iN` where `N` is not 8, 16, 32 or 64. The
operands are shifted to the top of an `i64` and LLVM's 64-bit overflow
intrinsics do the work. Their flag is then exactly the overflow at `N`
bits. For `mul`, only one operand is shifted.

```llvm
%xs = shl i64 %x64, 16          ; i48 payloads
%ys = shl i64 %y64, 16
%r  = call { i64, i1 } @llvm.sadd.with.overflow.i64(i64 %xs, i64 %ys)
%v  = ashr exact i64 %r0, 16    ; exact: the low 16 bits are zero
```

For an `Int(i48)` nanbox add, the hot path is:

```asm
shlq   $16, %rdx        ; x << 16  (the tag shifts out)
shlq   $16, %rsi        ; y << 16
addq   %rsi, %rdi
jo     .Lslow
shrq   $16, %rdi        ; back to 48 bits, zero-extended
orq    %rcx, %rdi       ; | 0xFFF9 << 48
```

Compare `addl; jo; movl; orq` for `Int(i32)`. LLVM's own legalization of
`sadd.with.overflow.i48` sign-extends both operands (`shl; sar` each) and
compares the result with its re-extension, which is about twice as long.

The same idea applies to other operations on odd widths:

- **Relational compares** compare the operands shifted to the top
  (`shl; shl; cmp`, not `shl; sar` per operand).
- **`/` and `%` wider than 32 bits** use a 32-bit divide when both
  operands are non-negative and below 2^31. A 64-bit `idiv` is two to
  three times slower on many x86 cores. The test is one `test` of
  `(x | y) << k` against a mask of bits 31 and up.
- **Division checks** (`y == 0`, `x == MIN and y == -1`) also test the
  shifted operands. That lets a dominating compare fold them away, such
  as Python's `y > 0` guard on floor division (`icmp sgt i64 (y << 16),
  0`). With the checks in `i48` form, LLVM kept both checks and their
  `panic` calls, and the handler set up a stack frame on its fast path.

On AArch64 the shifted forms fold into shifted-register operands:

- an `Int(i48)` add is `lsl; adds x, x, y, lsl #16; b.vs; mov; bfxil`;
- a compare is `lsl; cmp x, y, lsl #16`;
- a multiply is `lsl; sbfx; mul; smulh; cmp ..., asr #63; b.ne` (there is
  no flag-setting multiply).

Two alternatives measured slower on Python's int loops:

- add/sub in `i64` on sign-extended operands, then a range check: 12
  instructions instead of 6, 5–10% slower;
- a plain 48-bit add for the value beside the shifted overflow add
  (shorter dependency chain): 2–5% slower, as these handlers are
  limited by instruction throughput.

What remains against `Int(i32)` is the two `shl` per operation and the
`shr` before re-tagging. `nanbox(int_high)` removes them: there the add is
`add; jo`.

### `boxed`

`{ i64 tag, i64 payload }`, passed and returned in two registers. The tag
is the variant's index; integer payloads are sign- or zero-extended.

## 3. Handlers in `tailcall` mode

Every op becomes one function. All handlers share one signature:

1. the vm's context and state variables, in declaration order;
2. the dispatch table's address, when it is a state register (section 7);
3. the current instruction word.

`Mini` has `context ctx` and `state { pc, regs, k, fp }`. Its `ADD`
handler, abridged:

```llvm
define internal preserve_nonecc void
@Mini.op.ADD(ptr %ctx, ptr %pc, ptr %regs, ptr %k, ptr %fp, i32 %insn) nounwind {
entry:
  ; operands: format ABC, kind `reg`. Verified, so no bounds checks.
  %bi = ... ; (insn >> 16) & 255, zext
  %ci = ... ; insn >> 24
  %pdst = getelementptr inbounds i64, ptr %regs, i64 %ai
  %a    = load i64, ptr (gep %regs, %bi)
  %b    = load i64, ptr (gep %regs, %ci)

  ; (Int, Int)
  %xa = xor i64 %a, -1970324836974592
  %xb = xor i64 %b, -1970324836974592
  %o  = or i64 %xa, %xb
  %hi = and i64 %o, -281474976710656
  %ii = icmp eq i64 %hi, 0
  br i1 %ii, label %int, label %not_int

int:
  %x  = trunc i64 %a to i32
  %y  = trunc i64 %b to i32
  %s  = call { i32, i1 } @llvm.sadd.with.overflow.i32(i32 %x, i32 %y)
  %ov = extractvalue { i32, i1 } %s, 1
  br i1 %ov, label %slow, label %int_ok

int_ok:
  %r  = extractvalue { i32, i1 } %s, 0
  %rz = zext i32 %r to i64
  %rv = or i64 %rz, -1970324836974592
  store i64 %rv, ptr %pdst, align 8
  ; `next`: load the next word, look up its handler, tail-call it
  %pc1  = getelementptr inbounds i32, ptr %pc, i64 1
  %w1   = load i32, ptr %pc1, align 4
  %op1  = and i32 %w1, 255
  %hp   = getelementptr inbounds [256 x ptr], ptr @Mini.table, i64 0, i32 %op1
  %h    = load ptr, ptr %hp, align 8
  musttail call preserve_nonecc void %h(ptr %ctx, ptr %pc1, ptr %regs, ptr %k, ptr %fp, i32 %w1)
  ret void

not_int:
  %m  = call i64 @llvm.umax.i64(i64 %a, i64 %b)
  %nn = icmp ult i64 %m, -1970324836974592
  br i1 %nn, label %num, label %slow

num:
  ; bitcast, fadd, bitcast back, store, then `next` as above

slow:                          ; the `cold { }` block, outlined
  musttail call preserve_nonecc void @Mini.op.ADD.cold0(ptr %ctx, ptr %pc, ptr %regs, ptr %k, ptr %fp, i32 %insn)
  ret void
}
```

### The calling convention

- `preserve_nonecc` makes every register caller-saved and passes many
  arguments in registers. State variables land in fixed registers and
  move between handlers without shuffling.
- `musttail` guarantees a jump with no stack growth. It requires every
  handler to have the same prototype.
- Each handler is its own symbol (`Mini.op.ADD`), so `perf` and other
  profilers attribute time per op with no extra tooling.
- Handlers are compiled with `"no-builtin-memset"`, `"no-builtin-memcpy"`
  and `"no-builtin-memmove"`: LLVM may not turn their loops into library
  calls, which would give the handler a stack frame.

### Cold blocks

A `cold { }` block that ends in a control word (`next`, `jump`, `raise`,
...) on every path is **outlined**: it becomes its own function
(`Mini.op.ADD.cold0`), marked `cold noinline`, with the handler's
signature, and the handler reaches it by `musttail`. The hot path then
makes no call.

- Operands are decoded again in the cold function from the instruction
  word it receives.
- Other locals it uses are passed through thread-local slots
  (`@"Mini.op.ADD.cold0.cN"`), stored just before the tail call.
- In a superinstruction, a `next` in the outlined block from a part other
  than the last tail-calls the super's **resume function** for the next
  part (`Mini.op.S.resume1`: the handler from that part on, with `pc` at
  the part's word). The rest of the super then runs as it would in the
  handler, and as it does in goto and switch modes. The same goes for
  `next` in a shared copy of a `-> never` vm function called from such a
  part, which gets the resume function in a thread-local slot.

A cold block that does not end in a control word stays inline. It is
marked unlikely with `call void @llvm.sideeffect() cold`, which emits no
machine code.

### The instruction word, only where it is read

A `raise` stores its error in the thread-local `@"Mini.raise_slot"` and
tail-calls `on raise` with the handler signature. Two cases pass `poison`
instead of the word:

- a raise, when the vm's `on raise` never reads the word it is passed
  (`vmgen::on_raise_reads_insn`);
- an outlined cold block that decodes no operand and reads no word
  (`reads_insn`).

Why: otherwise a handler whose cold branch follows a call must keep the
word in a callee-saved register across the call (a `push`, a `mov`). A
handler whose cold branch needs nothing else would go through a
`mov %eax, %esi; jmp cold` trampoline instead of branching to the cold
function directly. This made 365 functions in the examples shorter and
none longer on a hot path.

### Loops

Loops in tail-call vm code (handlers, cold blocks, vm functions) carry
`!llvm.loop` metadata that turns off vectorization, interleaving and
unrolling. It is what `#pragma clang loop vectorize(disable)
interleave(disable) unroll(disable)` gives.

Why: these loops run only a few times (zeroing a callee's locals, copying
constants or arguments). The vectorized and unrolled versions need enough
registers to give the handler a stack frame. The wasm interpreter's
`CALL`, with two copy loops, was 154 instructions with `push %rbp` and
spills, and is 50 without.

### Float state variables

**A float state variable stays in an SSE register.** `preserve_none`
passes floating-point arguments as the C convention does (`xmm0`, ...).
So `state { .., acc: f64 }` is `xmm0` in every handler. An op `acc = acc *
a` is `mulsd (%r14,%rdx,8),%xmm0` and the dispatch, with no load or store
of `acc` (checked with LLVM 19 on x86-64). This is wasm3's `fp0`.

An `accumulator` declaration (`language.md` §7.4) adds such a state
variable and derives the ops that read and write it. `xmm0` is not
preserved by C calls, so a handler that calls out on its hot path saves
and restores it around the call.

A value accumulator declared `f64` (`accumulator vacc: f64 for reg;` over
a nanbox `V`) holds the word as `f64.from_bits(w as u64)`, and a reader
takes `vacc.to_bits() as V`. These are bitcasts, so a float chain keeps
its result in `xmm0` with no move. A writer still stores its slot
(write-through) and sets the accumulator before each `next`; LLVM forwards
the value just stored, so this costs no load, only a `movq` to `xmm0` on
an integer path. A `jump` to a computed target zeroes the accumulator
(`xorps`).

### The dispatch table's address

**x86-64.** An executable (`-fno-pic`) indexes the table as an absolute
address. Whether LLVM folds the table load into the jump depends only on
how many values the tail call passes in registers:

| register arguments (context, state, instruction word) | dispatch |
|---|---|
| up to 6 | `movzbl %sil,%eax; jmp *table(,%rax,8)` |
| 7 or more | `movzbl %dl,%eax; mov table(,%rax,8),%rax; jmp *%rax` |

The limit is fixed in LLVM's x86 instruction selection
(`X86tcret_6regs` in `X86InstrCompiler.td`), made for the C
convention's six argument registers. It ignores which registers are free,
and the type and order of the arguments do not matter. The fold also
needs the table to be a constant, as the generated `internal constant`
is.

Mini, MiniJS, Scheme and Lox pass 6 and get the folded form; the wasm
interpreter passes 7 and Stak 8. The longer form costs nothing
measurable: the wasm interpreter with every dispatch rewritten to the
folded form in its assembly ran at 1.007 of the original's time, within
noise. The fold does not help because dispatch waits on two dependent
loads (the word, then the target) either way. Folding in the assembly
was tried and dropped for that reason.

A table address passed as an argument never folds on x86-64
(`mov (%r14,%rax,8),%rax; jmp *%rax`), which is one reason the table stays
a global there.

**AArch64.** A global's address takes `adrp` and `add` in every handler,
since nothing hoists them across tail calls. So in tail-call mode on
AArch64 the table's address is a hidden state variable (`__tbl`, the one
that variants use, section 7). The entry sets it and every handler keeps
it in a register. Dispatch is four instructions instead of six:

```
ldr  w26, [x21, #4]!          ; next word, pc += 4
and  x8, x26, #0xff           ; opcode
ldr  x0, [x25, x8, lsl #3]    ; table in x25
br   x0
```

`preserve_none` has about twenty argument registers on AArch64, so the
extra one costs no spills. The wasm interpreter's handlers went from 8045
to 7174 instructions in all. This is not timed on hardware (qemu timings
mean nothing).

### Why slow-path calls give handlers a frame

`-W handler_frame` reports tail-call handlers whose fast path sets up a
stack frame. Most come from a call on a rare path, such as a panic or a
GC call.

The cause is stack alignment, not register pressure:

- A handler is entered by `jmp`, so `rsp` is 8 bytes off.
- Any call needs a 16-byte-aligned stack, which costs a `push`.
- LLVM 19's shrink-wrapping can't sink that `push` onto the slow path.
  Once a block has touched the stack, later loads through loaded
  pointers and the folded dispatch (`TCRETURNmi64`) count as frame uses,
  so the frame moves to the entry block.
- A different calling convention for the callee (`preserve_most`,
  `preserve_all`) does not help by itself.

Calling slow paths through inline asm and a `preserve_all` stub, so that
LLVM sees no call, was tried and not merged (branch
`cold-stub-experiment`). It removed the frame from 43 of 218 handlers but
measured as noise over 102 benchmarks, grew tail-call `.text` 8–30%, and
made Stak's goto build 1.34× slower.

What removed the two commonest causes instead:

- The write barrier's slow path (`std.gc.log_obj`) pushes onto the
  remembered set inline, with no call (section 10).
- A `new` in a handler becomes `try_new` with a cold refill that ends in
  `redo` (section 10).

The inline barrier slow path has a cost outside handlers. Each barrier
adds the slow path's instructions to LLVM's inlining cost of its
function, so a small helper with a few barriers can grow past the
inlining threshold and become a call. Mini's thread switch did, which
made two coroutine benchmarks 6% slower in tail-call mode. Mini now
stores those references through raw pointers and logs the objects itself
(`gc.needs_barrier` and an out-of-line `gc.remember`), which keeps the
functions small.

### `next`, `jump`, `redo`, `become`

| Word | Lowering |
|---|---|
| `next` | `pc' = pc + size(op) + size(cache)`. Load the word at `pc'`, load its handler from the table, `musttail call` it with the word |
| `jump t` | the same, with `pc' = t` |
| `redo` | the same, with `pc' = pc` |
| `become X` | load the word at `pc`, replace its opcode field with `X`, store it back |
| `exit v` | return `v` (see below); the `musttail` chain unwinds straight back to the entry function |
| `raise e` | store `e` to `Vm.raise_slot`, `musttail call` the `on raise` handler |

How `exit v` returns its value:

- A scalar result (an integer, pointer, float or non-boxed value) is
  returned in registers, and handlers return the entry's type.
- Any aggregate result (a `Result`, struct, tuple, option of a
  non-pointer) is stored to a thread-local `Vm.ret_slot`, and handlers
  return `void`. A `musttail` call cannot return through a hidden `sret`
  pointer. The entry loads the slot after its call returns.

**Inline cache fields** are places at `pc + 1 + offset` words. A field
wider than a code word (a pointer in a `u32` vm) is loaded and stored with
the word's alignment, `load ptr, ptr %p, align 4`, which x86-64 and
AArch64 execute as one unaligned access. The size of each cache is a
constant of the generated source (`Vm__CW_PropIC`, from `size_of` of the
field types), so encoders, `op_size`, the verifier and `optimize` agree
with the handlers.

### `-> never` vm functions

A call on a hot path inlines the function's body. Its control words then
lower like the caller's own.

In tail-call mode, a call from a `cold` block instead does this:

1. stores the arguments and the caller's instruction size (for `next`) in
   thread-local slots (`@"Vm.fn.NAME.a0"`, ...);
2. makes a `musttail` call to `@"Vm.fn.NAME"`, which has the handler
   signature.

So all cold callers share one copy, and the state never leaves its
registers. The shared copy is `noinline` only under `#[inline(never)]`
(otherwise LLVM may inline it back into its callers). A shared copy that
never reads the calling instruction's word (no `raise`, `?`, cold block,
shared call or chain in it) is passed `poison` for it, so its callers
need not pass the word in.

## 4. Entry points

An entry is lowered as an internal function.

- **Tail-call mode:** the function sets up the state and makes one
  ordinary `preserve_none` call into the first handler, so it saves the
  callee-saved registers that the handlers clobber. It returns what the
  `exit`ing handler returned or left in `ret_slot`.
- **Goto and switch modes:** the entry is its own loop function with
  every handler inlined.

### Nested entries

An entry declared `uses reenter` can be called while a run of the same vm
is active (for example from a native that calls back into the
interpreter, like `Mini.call`, which the interpreter writes). In goto and
switch modes the entry's loop function is simply called recursively. The
nested run therefore runs on the C stack of the native that called it,
above the outer run's frames, and its `exit` returns to that native.

The compiler adds two things:

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

- `Vm.depth` is a thread-local `u32` generated with the vm's other items.
  The entry saves it, increments it, and stores the saved value back
  before each `ret`, so every exit path restores it.
- Calls to the entry have the `reenter` effect (section 9).

The frame sentinel (which frame's return ends the run), frame placement
and the depth limit are the interpreter's code (`language.md` §10).

What the compiler must protect is its own per-thread state. Cold-block
captures, `-> never` arguments and `on raise`'s argument are loaded from
their thread-local slots at the top of the function that receives them,
before it can call anything. `exit` stores `ret_slot` just before the
chain returns to its entry. So a nested run in between never overwrites a
slot that an outer handler still has to read.

**Cost.** A nested run costs a few tens of cycles beyond the work it does.
Measured on Mini (`bench/reenter/run.sh`, `table.sort` on 200,000
integers):

| Comparison | ns per comparison |
|---|---:|
| native sort, comparing with `<` | 13 |
| native sort, Lua comparator (a nested run each) | 33 |
| the same algorithm written in Lua (an ordinary `CALL` in the one loop) | 27 |

So a native higher-order builtin beats a prelude version only if its
native part saves more than that per callback. Scheme's and MiniJS's
preludes remain the faster choice for `map`, `for-each` or `sort` with
cheap callbacks. Re-entry is for what cannot stay in the loop: C natives
and embedders calling back, metamethods invoked from runtime functions,
and builtins whose own work dominates.

### Libraries

The interpreter writes its C API as `export "C"` functions (external
linkage, C calling convention, `bool` as `zeroext`). Everything else
stays `internal`, including `loimi.argc` and `loimi.argv`.

| Build | clang | Output |
|---|---|---|
| `--lib` | `clang -c -fPIC`, then `llvm-ar rcs` | static archive |
| `--shared` | `clang -shared -fPIC` | shared object |

A library build emits no `main` wrapper. There is no init function: all
globals have static initializers, and runtime thread-locals start zeroed
on each thread. `--header` writes a C header for the exports
(`src/header.rs`).

`std.rt.protect` is `@loimi_protect`, a `noinline` function. It links a
handler `{ prev, shadow_top, jmp_buf }` into the thread-local
`@loimi.handler_top`, calls `_setjmp` (`returns_twice`), and calls the
protected function. `@loimi_throw` `longjmp`s to the top handler, which
restores `@loimi.shadow_top` and the handler chain.

### TLS model

The runtime's thread-locals are emitted `thread_local(initialexec)`:
`@loimi.shadow_top`, the cold-block capture slots, the `-> never`
argument slots, the result and raise slots, and `@loimi.handler_top`. In
an executable, or a static archive linked into one, each access is one
`%fs`-relative instruction.

`--shared` uses `thread_local(localdynamic)` instead (`--tls=` chooses).
Why: a shared object with initial-exec TLS is flagged `STATIC_TLS`. It
can only be `dlopen`ed if the C library has static TLS space to spare
(Mini uses 328 bytes; glibc keeps 512 spare by default).

A local-dynamic access calls `__tls_get_addr`. In handlers those accesses
are only on the way into a `cold` block. But a handler that contains any
call needs a stack frame, which its fast path then pays for too. Measured
on Mini's benchmarks through `examples/embed/minirun.c`, relative to the
`-fno-pic` executable (minimum of 5 runs; noise about ±5%):

| Benchmark | static archive, PIE host | `--shared`, initial-exec | `--shared`, local-dynamic | local-dynamic + TLSDESC |
|---|---:|---:|---:|---:|
| closures | 1.05 | 1.03, 1.04 | 1.14, 1.06 | 1.13, 1.03 |
| binary_trees | 0.98 | 0.94 | 0.92 | 0.94 |
| nbody | 1.05 | 1.08 | 1.07 | 0.96 |
| fib | 1.02 | 1.03 | 1.05 | 1.06 |
| oo_methods | | 1.00 | 1.07 | 1.04 |
| sort | | 1.02 | 1.03 | 1.02 |
| string_build | | 1.00 | 0.99 | 0.96 |

Two numbers in a cell are two separate runs. What this shows:

- Local-dynamic costs 0–7% on call-heavy code (`closures`,
  `oo_methods`) and nothing measurable elsewhere.
- TLSDESC (`-mtls-dialect=gnu2`) replaces the `__tls_get_addr` call with
  a cheaper one, but measured within the noise, so the driver does not
  use it.
- A host that links the shared object at startup (not with `dlopen`) can
  build with `--tls=initial-exec` and pay nothing.
- Position-independent code itself costs nothing measurable: dispatch
  loads the table address with one `lea`.

**Thread-locals cost nothing over plain globals.** An initial-exec access
is `mov %fs:OFF, reg`, one instruction like an absolute address. The same
IR built with every runtime thread-local turned into a plain global (the
shadow-stack top included, touched at every call that may collect)
measured within noise: geometric means of 1.009 on Mini, 0.998 on MiniJS
and 1.000 on Scheme. So executables always use thread-locals, with no
single-threaded build mode, and independent states on several threads
cost nothing when there is only one.

## 5. `goto` mode

The whole vm is one function with one basic block per handler. State
variables are phis. Every `next` loads the next handler's address from
the labels table and branches to one shared dispatch block, a lone
`indirectbr`:

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

The machine code is still threaded. LLVM's tail duplication copies the
dispatch block into every handler *after register allocation*, so each
handler ends in its own `mov table(,%rax,8),%rax; jmp *%rax`. The driver
passes the LLVM options that make this happen, only when the program has
a goto-mode vm (section 12):

| Option | Why |
|---|---|
| `-tail-dup-pred-size`, `-tail-dup-succ-size` raised | since LLVM 18, tail duplication skips blocks with more than 16 predecessors and 16 successors, which the dispatch block always has |
| `-tail-dup-indirect-size=0` | no copies *before* register allocation |
| `-tail-dup-placement=false` | no tail duplication during block placement: it finds nothing left to copy, but the search costs time cubic in the op count |

**The empty `asm`** keeps the address computation in the handlers.
Without it, LLVM merges the identical table loads (and `pc` increments) of
all handlers into the shared block. That block then has five
instructions, more than post-RA tail duplication copies, and dispatch
stays a single shared indirect jump: 16% slower on the wasm benchmarks.

**Why not one `indirectbr` per `next` in the IR.** With `S` dispatch
sites and `H` handlers, that CFG has `S×H` edges. SSA then gives every
handler a phi per state variable with an entry per dispatch site:
`S×H×K` phi operands for `K` state variables. Every IR pass, PHI
elimination, the register coalescer and block placement pay for them.
Copying the shared block before register allocation (LLVM's default for
small indirect-branch blocks) recreates the same phis in machine IR.
After register allocation there are no phis: the state lives in the same
registers in every handler, and the copies are a few hundred
`jmp *%rax`.

Build time and peak memory of the clang step, clang 19 on an i7-6700. The
synthetic vm has `N` ops of one to three lines each and five state
variables; the wasm rows time the whole `loimi build --release`:

| vm (dispatch targets) | `indirectbr` per `next` | shared block, copied after RA |
|---|---|---|
| synthetic, N = 400 | 21 s, 256 MB | 1.5 s, 126 MB |
| synthetic, N = 800 | 157 s, 707 MB | 4.7 s, 183 MB |
| wasm (670: 382 ops + 287 accumulator variants) | 237 s, 970 MB | 17 s, 220 MB |
| wasm with every eligible op accumulated (1170) | out of 2.5 GB | 39 s, 325 MB (tailcall 28 s, switch 27 s) |

The shared form still grows faster than linearly (the machine CFG has the
same `S×H` edges), but a thousand-op vm builds in goto mode about as fast
as in the other modes.

The generated code is faster too, because the handlers are
register-allocated around one join point where every state variable has
one home register: wasm runs at 0.87 of the old time, Mini 0.97, MiniJS
and Scheme 0.99. The old form also crashed LLVM 19's LoopSimplifyCFG on
about 1% of the rooting fuzzer's programs
(`tests/run/goto_loop_simplifycfg.lm`); the shared block does not.

`cold` blocks stay inline and are marked unlikely. Register pressure is
shared across all handlers, which is why `tailcall` is preferred where it
is available.

## 6. `switch` mode

```
loop: %opc = load opcode(%pc); switch %opc [ ... ]
```

State variables are loop phis. This mode works on any target.

On x86-64 the switch becomes one shared dispatch block: load the word,
compare the opcode with the largest case, `jmp *table(,%rax,8)`. LLVM
does not copy it into the handlers, because it has more than 16
predecessors and successors and more instructions than post-RA tail
duplication copies. The state stays in registers (checked on the wasm
interpreter: no spills in the hot handlers, the `f64` accumulator in
`xmm6` and the integer one in `r15` across the whole loop).

**Tail merging is off** (`-enable-tail-merge=false`, only when the program
has a switch-mode vm; section 12). With it on, LLVM's branch folding kept
one copy of the handlers' common tails (store a result to a register
slot, `pc += 1`, branch to the switch). Many handlers then took two to
four unconditional jumps to reach the dispatch block. Turning it off
measured:

| Interpreter (switch build) | time, tail merging off vs on |
|---|---:|
| wasm (geometric mean of 7) | 0.79 |
| Lox `equality` | 0.71 |
| Mini | 0.93 |
| MiniJS | 0.95 |
| Stak | 0.98 |
| Scheme | 1.00 |

The single dispatch site remains: wasm in switch mode still takes about
1.4 times the tail-call time.

## 7. Dispatch tables and variants

```llvm
@Mini.table.default = internal constant [256 x ptr] [ptr @Mini.op.MOVE, ..., ptr @Mini.invalid, ...]
@Mini.table.count   = internal constant [256 x ptr] [ptr @Mini.count.MOVE, ...]

define internal preserve_nonecc void @Mini.count.ADD(ptr %ctx, ptr %pc, ..., ptr %tbl, i32 %insn) {
  ; `before { ctx.stats.ops[op.id] += 1 }`
  musttail call preserve_nonecc void @Mini.op.ADD(ptr %ctx, ptr %pc, ..., ptr %tbl, i32 %insn)
  ret void
}
```

- **The table pointer.** With variants, the table's address is a hidden
  state variable `__tbl`. The inner handler dispatches through it, and it
  still points at the variant table, so the interpreter stays
  instrumented until the table is switched back.
- **Without variants**, `__tbl` exists only in tail-call mode on AArch64
  (section 3, "The dispatch table's address"). Elsewhere handlers address
  the default table as a global constant, which frees a register.
- **Goto and switch modes.** Each entry is one function holding every
  handler, and a variant's wrapper is a block that runs `before` and
  branches to the op's block. Only entries that can switch tables get the
  wrapper blocks: those that say `use_variant` with a variant, or all of
  them when an op, vm function or `on raise` does. Why: in an entry that
  never switches, the wrappers are unreachable, but LLVM removes them
  only late, and on a large interpreter with `wraps all` its loop
  strength reduction took minutes over what they left behind.
- **Unused opcode slots** point at a trap handler (`@"Mini.invalid"`).
  Verified code can't reach it; it is there for defense in depth.

## 8. Superinstructions and families

**Superinstructions.** `super LOADK_ADD = LOADK; ADD;` becomes one handler
that:

- decodes `LOADK`'s operands from word 0 and `ADD`'s from word 1;
- inlines both bodies, with `LOADK`'s `next` replaced by a branch;
- advances `pc` by two words on its final `next`.

LLVM then optimizes across the boundary, for example forwarding the
stored constant into `ADD`'s load. It cannot forward a register: `ADD`
loads `regs[a]` after `LOADK` stored `regs[dst]`, two indices decoded from
different words, so the load must stay. No alias metadata can help,
because when the indices are equal the load must see the store.

**Forwarding variants.** A forwarding variant of a `#[forward] super`
(`MUL_ADD__f1`) is the same handler with `ADD`'s `a` decoded from `MUL`'s
`dst` field.

- The two addresses are then the same SSA value, so GVN replaces the load
  with the stored value (a phi when several paths of `MUL` store), and
  the value stays in a register.
- The variant is only correct where the fields are equal. `optimize`
  checks them before choosing it, and the verifier rejects it otherwise.
- Links can also join two views of one slot (`ri = *((regs + it) as
  *i32)`, `ru = .. as *u32`, `rd = .. as *f64`). The store and the load
  then have the same address, and GVN forwards the value across the type
  change when the sizes match. (A 4-byte store does not forward into an
  8-byte load; the variant is still correct.) The wasm interpreter relies
  on this (`I32_ADDK; F64_LOAD` links `ri` to `ru`, `F64_SUB; F64_STORE`
  links `rd` to `rq`).

**Families.** Family members are ordinary ops that share a format.
`become` rewrites the opcode (section 3). In a super's part, it stores the
part's own word and the super's first word (the opcode of the super with
that part replaced, chosen at compile time, or of the first part).

## 9. Effects and sync points

At a call to a function with an effect (`uses sync`, `gc` or `reenter`)
from a handler or cold block, the compiler adds code around the call:

- **Before the call:** the vm's `sync` block, which stores `pc`, the stack
  top and so on where the callee reads them. It is lowered inline at the
  call site, like the handler's own code.
- **For `gc` and `reenter`:** a shadow-stack frame that makes live traced
  locals and state visible as roots (below).
- **After a `reenter` call:** the vm's `reload` block. In tail-call mode
  the state lives in `alloca`s that mem2reg turns into SSA values, so
  `reload`'s assignments become the values later code uses. A `reenter`
  call in a vm without `reload` is a compile error.

Calls without effects get none of this.

### Shadow-stack frames

Live `Gc` locals at a call that may collect are kept in a shadow-stack
frame: an `alloca` of `{ prev, count, [n x { ptr slot, ptr mark_fn }] }`,
linked into the thread-local `@loimi.shadow_top` and unlinked on exit.

- The frame is only built on paths that reach such a call.
- It lists the traced locals in scope that are assigned on every path to
  the call, so no slot is scanned before it is written:
  - a `let` from after its initializer;
  - a pattern binding from after the test that matched it (not in `let
    PAT = e else { .. }`'s else block, nor in a later `match` arm).
- While a `match` guard runs, the frame also lists the scrutinee's slot
  (the copy the match tests). Arms after a failing guard still bind from
  it, and a temporary scrutinee (`match f() { .. }`) has no other root.

In a handler, the frame lists the handler's traced locals and then one
fresh `alloca` per state variable of a traced type, stored just before
the call:

```llvm
%tos.copy = alloca i64                     ; one per call site
store i64 %tos, ptr %tos.copy              ; %tos: the state variable's value
; frame entries: { ptr %local.slot, @mark.T }, ..., { ptr %tos.copy, @mark.Value }
```

Why a copy: the state variable's own `alloca` never escapes, so mem2reg
keeps it in SSA form in every dispatch mode. In goto and switch modes
that slot is shared by the whole loop function. A frame entry pointing at
it would pin it to memory in every handler.

### Checked roots (`--gc-check-roots`)

`std.gc.mark_ptr` trusts every pointer it is given, so a root listed too
early marks through whatever the slot holds. `loimi build
--gc-check-roots` makes the collector check every reference that
generated code hands it, before reading a header. Three changes:

**1. Every mark is checked.** Every `mark_ptr` call the compiler emits
calls `std.gc.rootck_mark` instead. That covers the mark functions of
shadow-stack entries (traced locals, values kept for defers, copies of vm
state), `gc_mark` in root scanners, and the fields that
`loimi_trace_object` traces. `rootck_mark` accepts:

- null;
- the start of an allocated object of the heap being collected: inside
  one of its segments, in a chunk already carved out, at a slot boundary,
  not a free slot;
- one of the heap's large objects.

A `value` is checked only in its pointer variants (an immediate is never
dereferenced). Anything else aborts:

```
gc check-roots: bad reference 0x0101010101010101: 0x01 bytes: an unassigned slot, or read from a freed object
  root #1 of shadow-stack frame #0 (0: the innermost) of
  main.victim at tests/run/gc_roots_let_else.lm:35:9 (call of main.churn); roots: #0 x: main.E; #1 n: Gc<main.Node>;
```

To name the root, each frame gets a first entry `{ @info,
@loimi_rootck_frame }`. Its "mark function" stores `@info` (a constant
naming the function, call site and its roots) in a thread-local and
resets a slot counter, which each root's mark function increments.
`loimi_trace_object` and `gc_mark` set the same thread-local to their own
record, so a bad heap reference is reported with the object that holds
it.

**2. Unassigned slots are recognizable.** Each traced local's slot is
filled with 0x01 bytes on function entry, and again where its scope ends
(on the path that falls out of the scope). A frame that lists a slot too
early, or for too long, shows `0x0101010101010101`, a non-canonical
address that no heap holds.

**3. Freed objects are recognizable.** `new` calls
`std.gc.rootck_alloc`, which registers (once per heap) a sweeper. It fills
every object about to be freed with 0x01 bytes and forgets where it
started, in both heap modes. A reference that outlived its object is then
reported when it is marked ("a freed object"), and a traced field read
from it reads 0x01 bytes, reported as above.

Notes:

- The `rootck_*` functions and the externs they use are emitted only in
  this mode (`is_lazy` in `src/codegen.rs`), so a default build's IR does
  not change.
- The checks assume the shadow stack holds only objects of the heap being
  collected. With several heaps on one thread, a frame of a nested run
  into another heap is reported.
- With `--jit-stencils` the stencils are generated in this mode too.
- The rooting fuzzer (`fuzz_gc_roots` in `tests/fuzz.rs`, generator in
  `tests/fuzz/rootgen.rs`) builds its programs in this mode.

## 10. GC barriers and allocation

### The write barrier

The barrier is always compiled in; the heap decides at run time whether
it does anything. After a store of a traced value into a place inside an
object, the compiler emits `call @std.gc.barrier(ptr %obj)`.

A traced value is a `Gc`, a `value` type with reference variants, or an
option, tuple, struct or array holding one. A place is inside an object
when it is:

- a field reached through a `Gc` (`o.f`, also `o.pair.a` and `o.arr[i]`
  through value-type fields);
- an element of a trailing array (`o.items[i]`);
- unchecked indexing of a trailing array's pointer (`o.items.ptr[i]`,
  recognized syntactically).

Calls that may store into the object are followed by the barrier too:

- a call that takes the address of such a place of a traced type
  (`o.pair.set(x)`, which auto-refs `&o.pair`, or `f(&o.pair)`);
- a call that passes `o.items` (or a sub-slice) of a traced element type
  to a parameter that the callee may store references through.
  `src/objref.rs` summarizes each function's slice and pointer
  parameters; the callee must not collect, and such slices cannot be
  kept elsewhere (`design/safety.md` section 8).

No barrier is emitted for:

- other stores through raw pointers. The code that makes them calls
  `gc.write_barrier(h, obj)` (see
  [`stdlib/gc.md`](stdlib/gc.md#as-implemented)), or pins the object
  (`h.pin_dirty(obj)`), which makes every collection trace it;
- initializing a new object (`new(h, T { .. })`);
- a store of a value with no references: a payload-free variant, `None`,
  literals of such (`gcgen::holds_no_refs`).

`barrier` is `alwaysinline` and tests one header byte:

```llvm
store i64 %v, ptr %slot
%f = load i8, ptr %obj - 9            ; header flags
%log = and i8 %f, 1                   ; LOG: old, not yet remembered
br i1 %log, label %remember, label %cont   ; expected not taken
```

On x86-64 that is `testb $0x1,-0x9(%rax); jne`. The header shares a cache
line with the object's first fields, which a store has usually just
touched (a shape check before an inline-slot store, the length before an
element store). In a non-generational heap (no object ever has `LOG`)
the barrier costs +0.4% on Mini and +0.6% on MiniJS (geometric means),
at worst +3%, within placement noise.

Two details keep it cheap:

- `std.gc.alloc` writes the type id, mark and flags bytes in one store
  after its refill path has merged. For a store into an object allocated
  earlier in the same function, with no aliasing store in between, LLVM
  knows the flags are 0 and deletes the barrier (Mini's `CLOSURE` has
  none left).
- The slow path (`log_obj`) is inline and makes no call, so it does not
  keep the handler from being a leaf function (section 3, "Why slow-path
  calls give handlers a frame").

### Trace hooks

`loimi_trace_object(h, obj, tid)` is a switch over the type ids. Each case
marks the traced fields, then the trailing array (its length is at
`obj - 8`). For an object with `#[trace(f)]`, the case marks the fields
and then calls `f(h, obj)` (an internal function, which LLVM may inline)
instead of looping over the array. Everything that traces an object
(marking, the remembered set in a minor collection, verify mode) goes
through the switch, so the hook is used everywhere. In a
`--gc-check-roots` build its `gc_mark` calls are checked like any other.

### Allocation in handlers, polls and `redo`

`std.gc.alloc` refills a size class's free run with a call (`refill`). In
a tail-call handler, that call sets up a stack frame on every execution,
not only when it refills. So after checking, a `new` in an op handler is
rewritten into the form handlers used to write by hand
(`src/redocheck.rs`):

<!-- snippet: skip the rewritten typed tree, shown as source -->
```loimi
let c = new(&ctx.heap, Pair { head: a, tail: b });
// becomes
let Some(c) = try_new(&ctx.heap, Pair { head: a, tail: b }) else cold {
    gc.make_room(&ctx.heap, 32);   // the object's size, a constant
    redo
};
```

- `try_new` is `std.gc.try_alloc`: `alloc` without the refill branch
  (`#[inline]`; it returns null where `alloc` would call `refill`).
- `make_room` refills the class and never collects.
- A `new` inside an expression becomes `match try_new(..) { Some(t) => t,
  _ => cold { .. } }`.

The rewrite is of the typed tree, so the result is exactly the
hand-written form: a test vm written both ways gives the same assembly in
all three dispatch modes. Python's `MAKECELL` lost its stack frame this
way (47 lines of assembly to 33).

The rewrite applies only where:

- the size is a constant of at most 496 bytes (the inline path of
  `try_alloc`). Above that, or with a variable trailing length,
  `try_new` is a call too, so the plain `new` is kept, which does not run
  the op twice on a refill;
- the `new` is outside loops of the op and outside cold blocks;
- the path to it has no effect.

**"No effect"** is the same analysis that checks every `redo`. It runs
over each op handler (and each superinstruction resume function) after
type checking, in evaluation order. The state at each program point is
one of: unreachable, clean, or dirty (with the first effect seen).

- Branches join (dirty wins); loops iterate to a fixed point.
- A superinstruction part starts clean (`redo` there reruns the part's
  word alone).
- A `-> never` vm function's shared copy is walked from the call's state.

An effect is any of:

- a store whose place is not a local: a pointer, a `Gc`, a slice, a
  global or a state register (the instruction's own words and the
  dispatch-table register are exempt);
- an atomic or unaligned store;
- a call of a function that may store. Whether a function may store is
  computed over the call graph: it may if it contains a store, a call
  through a value, or a call of an extern that takes a pointer, directly
  or in a callee. Calls into `std.gc` and `std.gil` are not effects.

At a `redo` that ends a block, the state checked is the one at the
block's entry, so the fix-up the block makes (a collection, `become`, a
stack grown) is allowed. Elsewhere it is the state at the `redo`. The
check is the lint `redo_effect`, an error unless allowed. Every example
interpreter passes; the one exception is Python's `CALL_NEW`, whose
`ensure` stores only when the op goes on to push a frame, so it is
`#[allow(redo_effect)]`.

**Polls.** A `poll` is expanded by the parser. `#[poll(gc)]` on an op
prepends the declaration's statements to the op's body, and `poll gc;`
splices them where it stands. So the checker, the lifetimes pass, the
cold-block outlining and the stencils all see the same tree as for the
statements written out. (`tirwalk::extend_cold` copies the rest of the op
into a poll whose cold block falls through.) `--vm-stats` counts a poll's
cold block under each op, with the poll's source line; a `new`'s refill
block is counted at the `new`.

### Why the heap does not move objects

The generational GC is non-moving, with sticky mark bits. It needs no
reloading after `gc` calls, keeps raw pointers into objects valid (Mini's
`aptr`, MiniJS's `Arr.ptr`, upvalue pointers, `k` pointers into constant
arrays), and costs one byte test per barrier. A copying nursery would
need every such pointer re-derived after each safepoint.

## 11. Bounds checks and verification

Operand accesses in handlers are emitted as `getelementptr inbounds` with
no compare-and-branch: the verifier has proved them in range. Other
indexing uses ordinary checked slices.

The verifier is generated Loimi source (`src/vmexpand.rs`): a loop over
the code that decodes each word with the same field extraction the
handlers use, then evaluates the pasted-in `verify` clauses. It returns
`Result<Vm__Verified, Vm__VerifyError>`.

**`Verified<Vm>`** is the generated struct `Vm__Verified { __code: []W }`:
a slice, passed and stored as `{ ptr, i64 }`.

- `jump v` lowers to the slice's pointer (`extractvalue` after mem2reg),
  so an entry that takes a `Verified` dispatches exactly like one that
  takes a raw pointer. It only receives the length in one more register.
- `v.start()` is the same field read.
- All checks are at compile time: the `__` field is private to the vm's
  expansion file, `#[unsafe]` functions need an `unsafe { }` block, and
  `#[require_verified]` is checked on each entry's `jump` (vmcheck).

**`Vm.Resume`** (resume points) is `Vm__Resume { __pc: *W }`, one pointer.

- `next_resume` is `next_pc` wrapped in it (`pc` plus the op's size, or
  plus the size argument in a vm function's shared copy).
- `jump r` lowers to the field, like a raw `jump`. So Mini's resume entry
  (`jump c.ypc` with `ypc: Mini.Resume`) is the same code as an
  `unsafe { jump c.ypc }` on a `*u32`.
- `v.resume_at(i)` is a generated loop that steps through the code by
  `Vm.base_size` (superinstruction parts count as instructions) until it
  reaches or passes `i`.
- The verifier's "may continue" scan (the code cannot end with such an
  instruction) counts `next_pc` and `next_resume` as reads of `pc`.
- The `Vm__Resume` items are generated only when some source file names
  them, so other programs' IR does not change.

**Where the proof is stored** is the interpreter's choice, and it can
cost. A `Verified` field in the prototype is a `{ ptr, len }` that `CALL`
loads: one more dependent load per call than a trailing array, whose
address is `p + offset`. On Mini that cost `oo_methods` 4% and `closures`
2%. A raw `*u32` field costs the same, so the cost is the layout, not the
type.

`code: verified [W] by Vm` keeps the proof without the load:

- Reading `p.code` builds `Vm__Verified { __code: <trailing slice> }` (a
  struct value, not a place).
- `jump p.code` takes the slice's pointer directly in the checker, so it
  lowers to the trailing array's `getelementptr inbounds i8, ptr %p, i64
  <size>`: a `lea`.
- The length (the object header's word before `p`) is loaded only where
  `p.code.len()` or `words()` is used.
- `Vm.build_verified` is generated Loimi: `new` with the word count, a
  copy loop into the array (through `__verified_words`, a builtin that
  only the vm's expansion file may call), then `Vm.verify` over the array
  with the object as the `owner` context. A `verify NAME { .. }` block is
  pasted after the instruction loop, with `is_insn_boundary` reading the
  boundary bitmap the first pass built.

Mini uses the trailing array.

## 12. Driver settings

| Setting | Why |
|---|---|
| `-O3` with `--release`, `-O1` otherwise | standard |
| goto mode: `-mllvm -tail-dup-indirect-size=0 -tail-dup-pred-size=1000000 -tail-dup-succ-size=1000000 -tail-dup-placement=false`, when the program has a goto-mode vm (section 5) | the shared dispatch block is copied into every handler after register allocation; compile time stays near linear in the op count |
| switch mode: `-mllvm -enable-tail-merge=false`, when the program has a switch-mode vm (section 6) | handlers branch straight back to the switch instead of through shared tails |
| handlers `nounwind` | no unwind tables in the hot path |
| `cold` functions in `.text.unlikely` | instruction-cache locality for handlers |
| a vm with `#[profile("file")]` (or `--vm-profile=FILE`): handlers in profile order, the profiled ones in a page-aligned `.text.hot.<vm>` section, outlined cold blocks in `.text.<vm>.cold` after it ("Handler order", below) | the hot handlers' layout depends only on their own code |
| executables `-fno-pic -no-pie`; `--lib`/`--shared` `-fPIC` | absolute dispatch-table addresses; libraries link anywhere |
| initial-exec TLS; local-dynamic for `--shared` (`--tls=`) | `dlopen`-safe shared objects (section 4) |
| x86-64 release builds of programs with a vm: `-mllvm -x86-align-branch-boundary=32 -x86-align-branch=fused+jcc+jmp+indirect -x86-pad-max-prefix-size=5` (`--no-branch-align` drops them) | no branch, dispatch jumps included, crosses or ends at a 32-byte boundary ("Branch alignment", below) |
| `--cpu=NAME` (or `LOIMI_CPU`): `-march=NAME` on x86-64, `-mcpu=NAME` on AArch64, for every clang run of the build (the program, JIT stencils, C runtimes, `-W handler_frame`'s assembly); default none, the target's baseline (SSE2, NEON) | portable binaries by default; vector code for a newer CPU when asked ("`--cpu`", below) |

### Branch alignment

On Skylake-derived Intel cores (Skylake to Comet Lake, Cascade Lake) the
microcode fix for the JCC erratum changes how some code is decoded. Any
32-byte code block with a branch that crosses or ends at its end is kept
out of the decoded-uop cache, so that code runs from the slower legacy
decoders.

A handler is short, so one badly placed dispatch `jmp *%rax` puts most of
it there. Which handlers that hits depends only on where the linker put
them. In the wasm interpreter, one such handler made three PolyBench
kernels take 1.4 times as long, and moving every handler by 128 bytes
made the effect go away.

Clang's `-mbranches-within-32B-boundaries` pads conditional and direct
jumps but not indirect ones, which are the ones that matter in a threaded
interpreter. So the driver passes the underlying options with `indirect`
added.

- The padding is a prefix on an earlier instruction where one fits (no
  extra uop), else a nop. Binaries grow by about 1%.
- With against without: wasm PolyBench 0.963 (geometric mean of 30; the
  three kernels 0.67–0.70), wasm tuning set 1.003, Mini 0.99, MiniJS
  1.01.
- Per program, results move ±4% both ways in interpreters without such a
  handler, because padding moves code and so changes other placements.

Two such losses were traced (PolyBench `gemm` in goto mode, Lox
`properties` in tail-call mode). Neither was the padding itself; both
were where it happened to leave hot code:

- `gemm` depended on the hot handlers' absolute addresses, with a period
  of more than 32 KiB: two hot branches colliding in the branch
  predictor, not a decoder or cache effect.
- `properties` depended on the distance between its three hot handlers,
  which padding elsewhere had grown by 64 bytes. Placing the three next
  to each other made it fast with and without alignment.

No clang setting avoids either: whether a layout has such a collision is
not visible in the code, and any edit before the hot handlers draws again.
What can be controlled is that unrelated edits do not draw again. That is
what the handler order below does.

### Handler order

A vm with `#[profile("file")]` (`language.md` §3.7), or any vm with
`--vm-profile=FILE`, has its handlers laid out by the profile. The ops the
profile counts come first, most dispatched first; the rest follow in
declaration order. `src/hotorder.rs` reads the file, which is one of:

- `--vm-stats` output, any number of runs concatenated (each run's counts
  scaled to fractions of its own dispatches, so that a long run does not
  drown short ones);
- a list of op names, hottest first.

What the order changes, per dispatch mode:

| Mode | Effect |
|---|---|
| tail-call | handler functions emitted in profile order; the profiled ones in `section ".text.hot.<vm>"`, the first with `align 4096` |
| goto | handler blocks emitted in profile order (block placement keeps it, since only the indirect branch reaches them); the vm's entry function, which holds them, goes to the hot section |
| switch, and `--jit-stencils` builds | nothing: switch timings hardly depend on placement, and the profile's order measured slower in both |
| all | the vm's outlined cold blocks go in `section ".text.<vm>.cold"` |

GNU ld's default script places `.text.hot.*` after `.text.unlikely.*` and
before the rest of `.text`. So what precedes the hot handlers is the C
start-up code and the program's own `cold` functions. The vm's cold
blocks, which change with every slow path, are moved behind the hot
handlers (by default they are `cold` functions in `.text.unlikely`, ahead
of everything).

The page alignment makes the hot handlers' addresses modulo 4096 depend
only on their own code and the profile. A change to a handler outside the
profile, or to any ordinary function, no longer moves them.
`--vm-profile=none` builds as without the attribute (the A/B baseline).

**Measured** over `bench/run_all.py`'s 166 benchmarks per mode, ordered
against `--vm-profile=none`:

- Overall speed is unchanged: geometric means tail-call 1.001, goto 0.997,
  switch 1.001 (Python 1.027, hence switch mode is left out).
- Goto mode is where layout moved timings, and ordering removes that.
  With 128-byte function alignment, the mean change per benchmark was
  1.22% in declaration order (six over 5%, `gemm` 33%) and 0.47% ordered
  (none over 3.1%), against 0.55% noise.
- Tail-call handlers built with `--align-functions=64` already start on
  their own 64-byte boundaries, and the measurements do not tell ordered
  from unordered there.

`gemm`'s recovery is itself a draw: its collision depends on absolute
addresses, which ordering changes. What ordering adds is that the draw no
longer changes with unrelated edits. Scheme is the one example
interpreter not opted in (1.008 slower ordered in tail-call mode).

### `--cpu`

The driver asks clang which extensions the CPU has (`clang -march=NAME -E
-dM`: `__SSSE3__`, `__SSE4_1__`). So `native` and every CPU name clang
knows work, and an unknown name is an error before anything is built.

Two of Loimi's own vector lowerings depend on the extensions; LLVM picks
up everything else from `-march`:

- **`swizzle` with SSSE3:** `paddusb` of 0x70 (indices 0..15 keep their
  low bits below 0x80, 16..255 saturate at or above it), then `pshufb`,
  which zeroes lanes whose index has the top bit set. Without SSSE3, lane
  by lane through memory.
- **`floor`, `ceil`, `trunc`, `nearest` on vectors with SSE4.1:** the LLVM
  intrinsics, which become `roundps`/`roundpd`. Without SSE4.1, the
  add-and-subtract-2^23 sequence (`language.md` §1.2, "Vectors").

`--emit-llvm` output for a `--cpu` starts with a comment naming the
`-march` it needs (it may call `llvm.x86.ssse3.pshuf.b.128`).

Instructions per operation, for a function that takes its operands and
result through pointers (so each count includes two or three loads and a
store; clang 19, `--release --lib`, `ret` excluded). `native` on the
i7-6700 used here (Skylake: AVX2, no AVX-512) is the same as `x86-64-v3`:

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

On AArch64, `-mcpu` changes none of these except `i64x2 *` (4 with SVE2,
`neoverse-v2`): NEON is the baseline and has the rest.

**Interpreters gain nothing measurable from `--cpu`.** `x86-64-v3` against
the baseline on Stak, Lox, Scheme and Mini moved by no more than the
unchanged reference interpreters (clox, Lua, Gauche) moved between runs,
in every dispatch mode and with the JIT. The wasm interpreter measured
0.997. Vector code that runs `swizzle`, shuffles, rounding or
`minimum`/`maximum` is where `--cpu` pays. The default stays the baseline,
so that binaries run on any x86-64.
