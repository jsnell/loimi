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
- the verifier, builder, disassembler, and metadata from the format and
  operand declarations;
- C-ABI entry points (`Mini.run`, `Mini.call`).

Everything after vm expansion is a normal compiler. The output can be
bitcode, so a Loimi interpreter can be LTO-linked into a C/C++ host with
`clang -flto`.

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

**Packed layout (more than 7 tagged variants).** Each variant needs a block
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

### `lowtag`

The word is `payload << s | tag`, where `tag` has `s` bits. With at most 8
variants, `s = 3` and the tag is the variant's index; pointers (8-byte
aligned) are or-ed with it, integers of up to 61 bits are shifted by 3.
With more, pointers and 61-bit integers take 3-bit tags, and a variant with
an `N`-bit payload gets a tag of `64 - N` bits allocated from what is left
(the same buddy allocator, where splitting a tag appends a 0 or a 1 bit).

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
cores).

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
- Each handler is a separate symbol (`Mini.op.ADD`), so `perf` and other
  profilers attribute time per opcode without any extra tooling.

### `next`, `jump`, `redo`, `become`

| Word | Lowering |
|---|---|
| `next` | `pc' = pc + size(op) + size(cache)`. Load handler from `tbl[opcode(*pc')]`, `musttail call` |
| `jump t` | same, with `pc' = t` |
| `redo` | same, with `pc' = pc` |
| `become X` | `store i8 X, ptr %pc` (opcode is in the low byte, little-endian; big-endian targets adjust the offset). Becomes `store atomic ... monotonic` under `#[threads(shared_code)]` |
| `exit v` | `store v` to `ctx.exit`, `ret void`. The `musttail` chain unwinds straight back to the entry function |
| `raise e` | `store e` to `ctx.pending`, `musttail call @Mini.on_raise` |

**Decode-ahead** only applies when every path through the handler ends in
`next` without writing code. Handlers that `jump`, or that may `become`,
dispatch after their body.

## 4. Entry points

```llvm
define { i64, i64 } @Mini.run(ptr %ctx, i64 %closure) {      ; C ABI
  ; run `entry run` as ordinary code to set up pc/regs/k, then:
  call preserve_nonecc void %first_handler(ptr %ctx, ptr %pc, ptr %regs, ptr %k, ptr %tbl)
  ; returns here when some handler does `exit` or `fail`
  ; read ctx.exit / ctx.pending, return { status, value }
}
```

`Mini.call` (reentrant) is the same function with a nested-entry marker pushed
on the frame stack. It counts C-stack depth and fails with a guest
stack-overflow error past the limit.

## 5. `goto` mode

A single function with one basic block per handler. State variables are phis
in each handler block, and every `next` ends in its own `indirectbr`:

```llvm
@Mini.labels = internal constant [N x ptr] [ptr blockaddress(@Mini.run_goto, %op.MOVE), ...]

op.ADD:
  ; ... body ...
  %h = load ptr, ptr (gep @Mini.labels, opcode(%pc1))
  indirectbr ptr %h, [label %op.MOVE, label %op.ADD, ...]
```

The driver disables LLVM's branch tail-merging for this function, which would
otherwise fold the replicated `indirectbr`s back into a single dispatch point
and undo the main benefit of threaded code. `cold` blocks are outlined exactly
as in tail-call mode. Register pressure is shared across all handlers, which is
why `tailcall` is preferred where it is available.

## 6. `switch` mode

```
loop: %opc = load opcode(%pc); switch %opc [ ... ]
```

State variables are loop phis. This mode is used for wasm without tail calls,
and works anywhere. On wasm with tail calls, `tailcall` mode lowers to
`return_call_indirect` and the calling convention is ignored.

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
When a vm declares no variants, `%tbl` is not a state register: handlers
address the default table as a global constant, which frees a register.

Unused opcode slots point at `@Mini.op.INVALID` (a trap). Verified code can't
reach it, so it exists only for defense in depth.

## 8. Superinstructions and families

- `super LOADK_ADD = LOADK; ADD;` becomes a handler that decodes `LOADK`'s
  operands from word 0 and `ADD`'s from word 1, inlines both bodies with
  `LOADK`'s `next` replaced by a branch, and advances `pc` by two words on its
  final `next`. LLVM then optimizes across the boundary (for example,
  forwarding the stored constant into `ADD`'s load).
- Family members are ordinary ops that share a format. `become` stores the
  opcode byte (section 3).

## 9. Effects and sync points

At a call to a `uses gc` function from a handler or cold block:

```llvm
store ptr %pc,  ptr %ctx.sync.pc
store ptr %top, ptr %ctx.sync.top          ; top = regs + proto.nregs, or sp
store i64 %live_value_1, ptr %ctx.tmproots[0]
call @heap_collect_slow(...)
; with a moving heap: reload %live_value_1 from ctx.tmproots[0]
; with `reenter`: reload pc/regs/k from ctx.sync as well
```

Calls without effects get none of this. In ordinary (non-handler) functions,
live `Gc` locals at `gc` call sites are stored in an `alloca`'d shadow-stack
frame `{prev, count, slots[n]}` that is linked into `ctx.roots` on entry and
unlinked on exit. The frame is only materialized on paths that reach a `gc`
call.

## 10. GC barriers and allocation

- `obj.field = v` with a `Gc`/`Value` field becomes a store plus the policy's
  `barrier(obj, &field, v)`, an `alwaysinline` function. For `MarkSweep` it is
  empty and disappears.
- `heap.new(T { ... })` inlines a bump or size-class fast path:
  `p = tlab.cursor; if p + size > tlab.limit { cold { refill() /* uses gc */ } }`.
  Header and field initialization follow. Only the `cold` refill carries
  effects, so only the refill path syncs.

## 11. Bounds checks and verification

Operand accesses in handlers are emitted as `getelementptr inbounds` without
compare-and-branch, because `run` only accepts `Verified<Mini>` (the type
system enforces this). Other indexing uses ordinary checked slices. The
verifier itself is generated MIR: a loop over the code that decodes each word
with the same generated decoder the handlers use, then evaluates the `verify`
clauses.

## 12. Driver settings

| Setting | Why |
|---|---|
| `-O3` with the new pass manager by default | standard |
| branch tail-merge disabled for `goto`-mode run functions | keeps `indirectbr` replicated |
| handlers `nounwind`; whole program `-fno-exceptions`-equivalent | no unwind tables in the hot path |
| `cold` functions placed in `.text.unlikely` | i-cache locality for handlers |
| optional handler ordering from `--vm-stats` profiles | hot handlers placed together |
| target check: `preserve_none` available? otherwise warn and fall back to `goto` | portability |
