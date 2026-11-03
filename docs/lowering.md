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
  wasm3's `fp0`; a vm that wants it needs ops that read and write the
  accumulator and a translator that keeps a value there between adjacent
  ops.
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
one copy and the state never leaves its registers.

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
as in tail-call mode. (*As implemented:* the driver sets no such option, and
LLVM keeps the `indirectbr`s replicated on its own; `cold` blocks stay inline
and are marked unlikely.) Register pressure is shared across all handlers, which is
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
call.

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
through it.
Other stores through raw pointers get no barrier: the code that makes
them calls `gc.write_barrier(h, obj)` (see
[`stdlib/gc.md`](stdlib/gc.md#as-implemented)). Initializing a new
object (`new(h, T { .. })`) needs none.

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
deletes the barrier (Mini's `CLOSURE` has none left). The `remember` call keeps the handler from being a
leaf: in tail-call mode MiniJS's `SETPROP_IN` gains a `push`/`pop` pair
that aligns the stack for it on the fast path too, since the slow path
rejoins the dispatch.

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
| branch tail-merge disabled for `goto`-mode run functions (not needed so far, not implemented) | keeps `indirectbr` replicated |
| handlers `nounwind`; whole program `-fno-exceptions`-equivalent | no unwind tables in the hot path |
| `cold` functions placed in `.text.unlikely` | i-cache locality for handlers |
| optional handler ordering from `--vm-stats` profiles (not yet implemented) | hot handlers placed together |
| target check: `preserve_none` available? otherwise warn and fall back to `goto` (not implemented: both targets have it) | portability |
| executables `-fno-pic -no-pie`; `--lib`/`--shared` `-fPIC` | absolute dispatch-table addresses; libraries link anywhere |
| initial-exec TLS; local-dynamic for `--shared` (`--tls=`) | `dlopen`-safe shared objects (§4) |
