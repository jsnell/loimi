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

## 2. Values (`repr nanbox`)

`Value` is an `i64`. Tag assignment for the example:

| Variant | Encoding |
|---|---|
| `Num(f64)` | the double's bits, with NaNs canonicalized to `0x7FF8_0000_0000_0000`; always `< 0xFFF9 << 48` |
| `Int(i32)` | `0xFFF9_0000_xxxx_xxxx` |
| `Nil`, `Bool` | `0xFFFA_...` with payload 0 / 1 / 2 |
| `Str`, `Obj`, `Fn`, `Native` | `0xFFFB`–`0xFFFE` in the top 16 bits, 48-bit pointer in the rest |

Pattern tests the compiler emits (`T_INT = 0xFFF9 << 48`, decimal
`-1970324836974592` as a signed i64):

| Pattern | Test |
|---|---|
| `v is Num` | `v <u T_INT` |
| `v is Int` | `(v >> 48) == 0xFFF9` |
| `(a, b) is (Int, Int)` | `((a ^ T_INT) \| (b ^ T_INT)) >> 48 == 0` |
| `(a, b) is (Num, Num)` | `umax(a, b) <u T_INT` |
| `v is Obj` | `(v >> 48) == 0xFFFC` |
| payload of a pointer variant | `v & 0x0000_FFFF_FFFF_FFFF` |

The match compiler works over the declared encodings. It tries tag-merging
rules (xor/or for equal tags, umax/umin for range-encoded variants) before
falling back to a decision tree. A tuple pattern of `k` values usually costs
one test, not `k`.

For `repr lowtag(bits = 1)` with tag bit 1 meaning integer:
`(Int, Int)` becomes `(a & b & 1) == 1`, and `Int(x)` becomes `(x << 1) | 1`.

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

### `-> never` vm functions

A call on a hot path inlines the function's body; its control words then
lower like the caller's own. In tail-call mode, a call from a `cold` block
stores the arguments and the caller's instruction size (for `next`) in
thread-local slots (`@"Vm.fn.NAME.a0"`, ...) and makes a `musttail` call to
`@"Vm.fn.NAME"`, which has the handler signature, so all cold callers share
one copy and the state never leaves its registers.

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
