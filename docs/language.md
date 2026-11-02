# The Loimi language

Design draft. Syntax is provisional; the semantics and the reasons behind them
are the point. Code samples are illustrative.

## 0. Principles

1. **The hot loop is the product.** A feature earns its place by improving the
   machine code of interpreter handlers, or by removing a class of bugs
   specific to interpreters. General-purpose niceties come second.
2. **One source, many dispatch strategies.** Tail-call threading, computed
   goto, and `switch` are build options, not rewrites.
3. **Declare, don't encode.** Instruction formats, value representations,
   object layouts, and inline caches are declarations. The compiler generates
   the bit manipulation and also the tools that must agree with it: builder,
   disassembler, verifier, tracer.
4. **Make the slow path explicit, cheap, and correct.** Slow paths are
   outlined so they don't hurt register allocation. Effects ensure the runtime
   always sees consistent VM state when it needs to.
5. **Safe by default, unchecked by proof.** Bounds checks are removed because
   a verifier proved them redundant, not because someone wrote `unsafe`.
6. **Stock LLVM only.** Every feature lowers to standard LLVM IR, attributes,
   and calling conventions, so the language benefits from LLVM's optimizer and
   targets for free.

## 1. Core language

The core is a small, conventional, statically typed systems language with
roughly C's level of abstraction. It has deliberately few novel features.

### 1.1 Types

```loimi
i8 i16 i32 i64 isize   u8 u16 u32 u64 usize   f32 f64   bool   never
[T; N]          // fixed array
[]T             // slice (ptr + len), bounds-checked
*T, *mut T      // raw non-null pointers; null is ?*T
?T              // option; ?*T is pointer-sized
Result<T, E>
Gc<T>           // traced reference into the GC heap (section 8)
```

Plus `struct`, `enum` (tagged unions), `bits` (packed bitfield integers),
`object` (GC-managed structs, section 8), and `value` (section 2).

### 1.2 Basics

```loimi
fn gcd(a: u64, b: u64) -> u64 {
    var (x, y) = (a, b);
    while y != 0 { (x, y) = (y, x % y); }
    x
}

struct Span { start: u32, end: u32 }

enum Token { Num(f64), Ident(Symbol), Punct(u8), Eof }

bits Insn: u32 { op: 8, a: 8, b: 8, c: 8 }   // insn.a reads bits 8..16
```

- `let` declares an immutable binding and `var` a mutable one. Blocks are expressions.
- Module-level `var NAME: T = init;` declares a global. `init` must be a
  constant expression (literals, consts, `[x; N]`, aggregate and enum
  literals of constants, functions). `#[thread_local] var` gives each thread
  its own copy, which is what an interpreter wants for "current VM" or
  per-thread caches. `pub var` is visible to other modules. Globals are not
  GC roots.
- `match` is exhaustive. `if let`, `let ... else`, and `while let` are supported.
- Generics are monomorphized. Interfaces (`trait`-like) use static dispatch only.
- `defer stmt` runs `stmt` when the enclosing scope exits.
- Error handling uses `Result` and `?`. There are no host-level exceptions or unwinding.
- **Integer arithmetic:** `+ - *` trap on overflow in debug builds and wrap in
  release builds. Explicit forms: `a +% b` (wrapping), `a.add_checked(b) -> ?T`,
  `a.add_overflow(b) -> (T, bool)`. These map directly to LLVM's
  `*.with.overflow` intrinsics, which guest integer semantics need constantly.
- Hints: `likely(c)`, `unlikely(c)`, `assume(c)`, `unreachable`, `#[cold]`,
  `#[inline(always)]`, `#[inline(never)]`.
- `unsafe { }` permits raw pointer dereference and unchecked indexing.

### 1.3 Memory

There is no borrow checker. Host memory is managed through explicit allocators,
mostly arenas (`std.mem`), which suit compilers and interpreters well: ASTs,
IR, and per-compilation data die together. Guest memory lives in the GC heap
(section 8). Slices are bounds-checked, and raw pointers require `unsafe`. This
is "safety-leaning", not memory-safe.

### 1.4 Compile-time evaluation

`comptime` expressions and `const fn` run in the compiler, Zig-style. This is
how tables (lexer DFAs, operator precedence tables, op variants) are built.
`inline for` unrolls at compile time and can generate items, including ops:

```loimi
inline for (name, cc) in [("JLT", Cond.Lt), ("JLE", Cond.Le), ("JEQ", Cond.Eq)] {
    op #name(a: reg, b: reg, target: label) : ABsC {
        if compare(cc, a, b)? { jump target } else { next }
    }
}
```

### 1.5 Modules and interop

`import std.gc.{Gc, Heap};`. `extern "C" fn` declares C functions, and
`export "C"` exposes Loimi functions, which matters for embedding.

---

## 2. `value` types: declared representations

Every dynamic-language interpreter has a `Value` type, and its encoding is the
most performance-critical decision in the VM. Loimi makes the encoding a
declaration:

```loimi
value Value repr nanbox {
    Num(f64) unboxed,          // any bit pattern that isn't a boxed tag
    Int(i32),
    Nil,
    Bool(bool),
    Str(Gc<Str>),
    Obj(Gc<Object>),
    Fn(Gc<Closure>),
    Native(Gc<NativeFn>),
}
```

Available representations:

| `repr` | Layout | Typical user |
|---|---|---|
| `nanbox` | 64-bit word. Doubles stored directly; other variants in the quiet-NaN space with a 48-bit payload | JS engines, LuaJIT |
| `lowtag(bits = N)` | 64-bit word. Aligned pointers with tag bits in the low bits; small integers shifted | Ruby, OCaml, many Schemes |
| `boxed` | Two words `{tag, payload}`, passed and returned in two registers | PUC Lua (`TValue`) |
| `pointer` | Everything is a `Gc<_>` with a header type tag | CPython |

The compiler chooses tag assignments, or you can pin them
(`Int(i32) = tag 0b001`). In return you get:

- **Constructors and patterns.** `Int(3)`, `match v { Int(x) => ... }`, and
  `if v is Nil`. No macros.
- **Optimal match compilation**, including over tuples of values, which is the
  case interpreters care most about (binary operators). The compiler knows the
  bit-level encoding, so it can merge tests. For example, with NaN-boxing and
  an `Int` tag `T` in the high 16 bits, `(Int(x), Int(y))` compiles to
  `((a ^ T) | (b ^ T)) >> 48 == 0`, one test instead of two. `(Num, Num)`
  compiles to `umax(a, b) <= LIMIT`. See `lowering.md`.
- **GC precision for free.** The compiler knows which variants hold `Gc<_>`,
  so tracing a `Value` is generated code.
- **Canonicalization.** For `nanbox`, constructing `Num(x)` canonicalizes
  NaNs, so a guest NaN can never forge a pointer. This is a safety property
  hand-written NaN-boxing regularly gets wrong.
- **Portability.** `nanbox` requires 48-bit pointers. On targets where that
  doesn't hold, or in wasm32, the compiler can fall back to `boxed` with a
  warning, and the source does not change.

Switching `repr` is a one-word change, which makes the representation
something you can benchmark rather than something you commit to on day one.

---

## 3. The `vm` declaration

A `vm` block declares one interpreter: its register state, operand kinds,
instruction encoding, inline caches, instructions, and entry and error
handlers. It is the center of the language.

```loimi
vm Mini {
    context ctx: *Interp;                 // cold state, reachable from all handlers
    state { pc: Cursor<u32>, regs: Window<Value>, k: *const Value }
    operand ...;                           // how operand fields are interpreted
    encoding ... { format ... }            // how instructions are laid out
    cache ...;                             // inline cache layouts
    op ... / family ... / super ...        // instructions
    fn ...                                 // vm functions (helpers with state access)
    entry ... / on raise ...               // ways in and the error path
    variant ...                            // instrumentation
}
```

### 3.1 `state` and `context`

`state` lists the variables that are **live across every dispatch**: program
counter, stack or register-window pointer, constants pointer, and so on.
Inside handlers they behave like mutable locals. The compiler guarantees they
live in machine registers for the entire life of the interpreter:

- in tail-call mode they are the arguments of every handler, in a calling
  convention (`preserve_none`) that assigns them fixed registers;
- in goto and switch modes they are loop-carried SSA values in one function.

`context` is one more register holding a pointer to everything else (heap,
frame stack, globals, interned strings). Fields accessed through `ctx` cost a
load, which is the right price for cold-ish data.

The compiler warns when `state` + `context` exceeds the target's argument
registers under `preserve_none` (about 12 general-purpose registers on x86-64
and more on AArch64, but fewer is better: 3 to 6 is typical).

State variables have **state kinds**, which are thin typed wrappers over a
single pointer that the verifier understands:

| Kind | Meaning |
|---|---|
| `Cursor<W>` | instruction pointer over code words of type `W` |
| `Window<T>` | base of the current register window (register VMs) |
| `StackTop<T>` | operand stack top (stack VMs); `push`, `pop`, `peek(n)` |
| plain types | anything else (`*const Value`, `u32`, ...) |

### 3.2 Operand kinds

An operand kind says what an encoded field *means*. Its body is an expression
over the raw field `it`, evaluated in the handler. Its `verify` clause is
checked once by the generated verifier instead of on every execution.

```loimi
operand reg   = regs[it]   verify it < proto.nregs;
operand kst   = k[it]      verify it < proto.consts.len;
operand imm   = it;
operand label = pc + it    verify is_insn_boundary(pc + it);
operand upv   = ctx.closure.upvals[it] verify it < proto.nupvals;
```

`verify` clauses may refer to the `verify_context` declared in the vm
(typically the function prototype being checked, `verify_context proto: Gc<Proto>;`).

If the expression is a place (like `regs[it]`), the operand is assignable:
`dst = Int(1)` writes the register. `x.raw` gives the undecoded field. The raw
field's width and signedness come from the format, so one `label` kind serves
8-bit and 24-bit offsets alike.

### 3.3 Encodings and formats

```loimi
encoding fixed(u32) {                 // every instruction is one u32 word
    format ABC  { op: 8, a: 8, b: 8, c: 8 }
    format ABx  { op: 8, a: 8, bx: 16 }
    format ABsC { op: 8, a: 8, b: 8, sc: s8 }
    format sJ   { op: 8, sj: s24 }
}

encoding bytes {                       // variable length, byte stream
    format Z    { op: u8 }
    format U8   { op: u8, x: u8 }
    format U16  { op: u8, x: u16le }
}
```

An op lists its operands in field order and names its format. The compiler
type-checks the mapping. From the declarations it **generates**:

- the opcode enum and the per-op decoders, inlined into the handlers;
- `Mini.Builder`: typed emit functions (`b.ADD(dst, a, b)`), labels with
  backpatching, and branch range checks (`std.bc`, see stdlib);
- `Mini.disasm`: a disassembler that prints operands through their kinds
  (`r3`, `k[7] = "foo"`, `-> 0042`);
- `Mini.verify` (section 6);
- metadata tables (sizes, operand kinds, branch targets) for tools.

The format declaration is the single source of truth for all of these, so they
cannot drift apart.

### 3.4 Ops and control words

```loimi
op MOVE(dst: reg, src: reg) : ABC { dst = src; next }
```

A handler body has type `never`: **every path must end in a control word**.

| Control word | Meaning |
|---|---|
| `next` | advance `pc` past this instruction and its inline caches, then dispatch |
| `jump t` | set `pc = t` and dispatch |
| `redo` | dispatch the current instruction again (after `become`) |
| `exit v` | leave the run loop, returning `Ok(v)` |
| `raise e` | transfer to the `on raise` handler with the current `pc` |
| `fail e` | leave the run loop, returning `Err(e)` |
| `suspend v` | leave the run loop with state saved for resumption (generators, coroutines, async) |

Inside a handler, `expr?` on a `Result<T, E>` means "on `Err(e)`, `raise e`".
`E` must be the vm's raise type.

Because the size of each instruction is static, `next` compiles to
`pc += size; dispatch`. In fixed-width encodings the compiler also
**decodes ahead**: it loads the next instruction word and handler address at
the top of the handler when the handler can't modify code, so the dispatch
load overlaps with the handler's work.

### 3.5 VM functions

A `fn` inside a `vm` block is a **vm function**. It can read and write state,
is always inlined into its callers, and, if its return type is `never`, can
use control words. That makes shared tails easy to write:

```loimi
fn call_closure(c: Gc<Closure>, base: *Value, nargs: u8) -> never { ...; jump c.proto.code_start() }
```

### 3.6 Entries and the error path

```loimi
entry run(c: Gc<Closure>) {
    ctx.frames.push_entry_marker();
    regs = ctx.stack.window_at_top(c.proto.nregs)?;
    k = c.proto.consts.ptr();
    jump c.proto.code_start()
}

on raise(err: Value) {
    loop {
        if let Some(h) = ctx.proto.handlers.find(pc) {
            regs[h.reg] = err; jump h.target
        }
        let Some(f) = ctx.frames.pop_guest() else { fail err };
        restore(f);
    }
}
```

The compiler generates `Mini.run(ctx, c) -> Result<Value, Value>` with the C
ABI. `on raise` is an ordinary handler: it receives the live state, so the
`pc` it sees is exact without any syncing.

### 3.7 Dispatch strategies

Selected per build (`--dispatch=` or `#[dispatch(...)]` on the vm):

| Strategy | Lowering | When to use |
|---|---|---|
| `tailcall` (default where supported) | one LLVM function per handler, `preserve_none` convention, `musttail` calls | x86-64 and AArch64 with LLVM ≥ 19; best register allocation, fast compiles |
| `goto` | one function, `indirectbr` replicated at every `next` | targets without `preserve_none` |
| `switch` | one function, loop plus `switch` | portability, wasm without tail calls, debugging |

All three are produced from the same source, so a benchmark can compare them
directly. The dispatch table pointer can be a state register (needed for
instrumentation variants, section 9) or a global constant (saves a register);
by default the compiler uses a register only if the vm declares variants.

---

## 4. Slow paths: `cold` blocks

Handler code splits into a short fast path and a large, rare slow path
(type coercions, metamethods, errors, allocation slow paths). In C, the slow
path's calls force the compiler to treat the VM registers as clobbered, so the
*fast* path ends up spilling them. Loimi makes the split explicit:

```loimi
op ADD(dst: reg, a: reg, b: reg) : ABC {
    if let (Int(x), Int(y)) = (a, b) and let Some(r) = x.add_checked(y) {
        dst = Int(r); next
    }
    cold { dst = arith(ctx, .Add, a, b)?; next }
}
```

A `cold` block is **outlined** into a separate function that uses a
callee-saves-everything convention (`preserve_most`, or `preserve_all` on
request). The handler's state registers survive the call untouched, so the fast
path is compiled as if the slow path weren't there. The block can still use
state and control words: the outlined function returns a small code ("next",
"jump to X", "raise E") and the handler performs the transfer. Branches into
cold blocks are weighted as unlikely. A `cold` block is also an expression:
`let ok = match (a, b) { ..., _ => cold { compare_slow(ctx, a, b)? } };`

---

## 5. Effects and sync points

The most common bug in hand-written interpreters is **stale VM state**: some
slow path allocates (and triggers GC, which scans the stack up to a stale top),
or raises an error (with a stale `pc`, so the wrong line or exception handler),
and nobody saved `pc`/`top` first. Lua guards against this with `Protect()`
and `savepc`; CPython with explicit frame syncs. Both rely on discipline.

Loimi has three ordered **effects** on functions:

| Effect | Meaning | Requires at the call site |
|---|---|---|
| `sync` | callee reads VM state (`pc` for tracebacks, `top`) | state written to `ctx.sync` |
| `gc` | callee may run a garbage collection | `sync`, plus every live `Gc`/`Value` local made visible as a root |
| `reenter` | callee may run guest code (call into the VM) | `gc`, plus state reloaded afterwards |

```loimi
fn new_string(h: *Heap, s: str) -> Gc<Str> uses gc;
fn traceback(ctx: *Interp) -> String uses sync;
fn call_meta(ctx: *Interp, f: Value, args: []Value) -> Result<Value, Value> uses reenter;
```

Rules:

- Effects are part of public signatures and inferred for private functions.
- A function may only call effectful functions if it has at least that effect
  itself, or if it is a handler or `cold` block. Forgetting `Protect()` becomes
  a type error.
- **In handlers**, the compiler inserts the sync at each effectful call site:
  store `pc` and the stack top to `ctx.sync`, and spill live `Value`/`Gc`
  locals into a per-vm temporary root area. With a moving collector it also
  reloads them afterwards. Calls without effects pay nothing.
- **In ordinary functions**, `Gc`/`Value` locals that are live across a `gc`
  call are registered in a compiler-maintained shadow-stack frame. This only
  happens on paths that can collect, which are slow paths by nature.
- Syncs only happen where they are needed, so code with no effects never
  writes `pc` to memory. Saving `pc` on every instruction would add a store per
  dispatch.

Because allocation fast paths are inlined (`std.gc` bump allocation) and only
their refill path carries `uses gc`, allocating a value in a handler costs
nothing extra unless the refill path actually runs.

---

## 6. Verification: unchecked by proof

`Mini.verify(code, proto) -> Result<Verified<Mini>, VerifyError>` is generated
from the declarations. It checks:

- every word decodes to a valid opcode and format;
- every operand satisfies its kind's `verify` clause (register and constant
  indices in range, upvalue indices valid, ...);
- every `label` lands on an instruction boundary (never inside an instruction
  or an inline cache);
- for stack VMs, ops declare stack effects (`op ADD() stack(a, b -- r)`) and
  the verifier computes the maximum depth per function and checks consistency
  at control-flow merges (like the JVM verifier, minus types);
- code can't fall off the end.

`Mini.run` only accepts `Verified` code. As a result, operand accesses compile
**without bounds checks**, and stack VMs check for overflow once per call
(`max_stack` against the limit) instead of on every push. The checks don't
disappear; they move from run time to load time. This matters for interpreters
that load cached or untrusted bytecode files.

Rewriting through quickening (`become`) and superinstructions preserves
verification, because the compiler checks that the rewrite keeps the format
and operand kinds unchanged.

---

## 7. Specialization: quickening, inline caches, superinstructions

### 7.1 Inline caches

```loimi
cache PropIC { shape: weak ShapeId, slot: u32 }
```

A cache is stored **inline in the code stream**, after the instruction (as in
CPython 3.11+), and is attached with `+`: `op GETPROP(...) : ABC + PropIC`.
`next` skips it automatically, the verifier forbids jumps into it, the builder
reserves and zero-fills it, and the disassembler prints it. Inside the handler,
`cache.shape` reads and writes the words directly. Fields marked `weak`
don't keep their target alive. When the target dies, the GC resets the entry
to zero, which the handler sees as a miss (`stdlib/gc.md` §8).

### 7.2 Families and `become`

A `family` is a group of ops that share operands, format, and cache, with
specialized implementations. `become X` rewrites the current instruction's
opcode in place.

```loimi
family GETPROP(dst: reg, obj: reg, name: kst) : ABC + PropIC {
    generic {
        let Obj(o) = obj else cold { raise type_error(ctx, "index", obj) };
        let Some(slot) = o.shape.lookup(name.as_symbol()) else { dst = Nil; next };
        cache.shape = o.shape.id; cache.slot = slot;
        dst = o.slot(slot);
        become GETPROP_MONO; next
    }
    GETPROP_MONO {
        if let Obj(o) = obj and o.shape.id == cache.shape {
            dst = unsafe { o.slot_unchecked(cache.slot) };   // shape match implies slot is in range
            next
        }
        cold { become generic; redo }
    }
}
```

The `generic` member is what the builder emits. Specialized members are only
reached through `become`. The compiler can also generate a deopt helper that
maps every member back to `generic`, for debuggers and for code serialization.

Rewrites are single-word stores. With multiple guest threads they need an
atomic store, which the compiler emits when the vm is declared
`#[threads(shared_code)]`.

### 7.3 Superinstructions

```loimi
super LOADK_ADD = LOADK; ADD;
super MOVE_RETURN = MOVE; RETURN;
```

The compiler builds the handler by inlining the component bodies in order.
Paths of the first component that end in `next` fall through into the second.
Other control words (`jump`, `raise`, ...) keep their meaning.

With `fixed` encodings, the fused op **reuses the components' words**: the
first word's opcode is rewritten to the super-op and the following words stay
as they are, so the super-op reads operands from both words. A jump into the
second word still runs the original second instruction, so fusion is always
safe, needs no control-flow analysis, and can happen at build time (peephole
in `Mini.Builder`) or at run time (quickening).

`loimi build --vm-stats` records opcode pair frequencies.
`loimi vm suggest-supers` turns them into ranked `super` declarations.

---

## 8. GC integration

The collector is in the standard library (`std.gc`), but the language knows
enough to make it precise and cheap:

- **`object` declarations** are GC-managed structs. The compiler generates the
  header (type id, mark bits) and the trace function from field types.
  A trailing variable-length array is allowed (`slots: [Value]`).
- **`Gc<T>`** is a traced reference. A `Value` traces whichever variants hold `Gc`.
- **Write barriers.** Stores of `Gc`/`Value` into object fields are lowered
  through the heap policy's barrier, which is a compile-time parameter
  (`#![heap = std.gc.MarkSweep]`). Non-incremental mark-sweep has no barrier,
  so stores cost nothing. A generational policy adds a card-mark at each such
  store, and the source does not change.
- **Roots.** These come from three places: the VM stacks (described by the
  `state` kinds and the `ctx.sync` area), compiler-managed temporary roots at
  effectful calls (section 5), and explicit `Root<T>` handles for host data
  structures.
- **Moving is opt-in.** The default `MarkSweep` is non-moving and
  size-segregated, which keeps interior pointers into the stack and objects
  stable across handlers. A moving policy makes the compiler reload spilled
  references after `gc` calls.

Loimi uses compiler-managed roots instead of LLVM's statepoint/stackmap
machinery. Statepoints constrain the optimizer everywhere, while the effect
system confines rooting to the slow paths that can actually collect.

---

## 9. Instrumentation variants

```loimi
variant count wraps all {
    before { ctx.stats.ops[op.id] += 1; }
}
variant trace wraps all {
    before { ctx.tracer.on_op(op.id, pc, regs) }      // uses sync
}
variant debug wraps all {
    before { if ctx.breakpoints.has(pc) { cold { ctx.debugger.stop()? } } }
}
variant line_hook wraps [JMP, JLT, JLE, JEQ, CALL, RETURN] { ... }
```

Each variant generates a separate dispatch table of wrapper handlers, each
wrapping the original handler. Switching is a single register update
(`set_dispatch(Mini.tables.debug)`), and the new table takes effect at the next
dispatch. **Normal execution pays nothing**: no flag check in any handler, and
no second copy of the interpreter to maintain. This is how debuggers, profilers,
coverage, and `sys.settrace`-style hooks are implemented.

The `count` variant also feeds `--vm-stats`, together with a pair counter.

---

## 10. Calls, reentrancy, coroutines

- **Guest-to-guest calls** should stay in the loop: push a frame (`std.rt.FrameStack`),
  slide the register window, and `jump`. This uses no C stack, so deep guest
  recursion is bounded by the VM stack, not the C stack.
- **Host-to-guest calls** (metamethods, callbacks, `sort` comparators) use
  `Mini.call(ctx, f, args) uses reenter`, which runs a nested loop on top of
  the same VM stack. The C stack depth is limited and checked.
- **Coroutines and generators** fall out of the explicit state: a coroutine is
  a separate `ValueStack` plus `FrameStack`, `suspend v` leaves the loop with
  state saved, and resuming re-enters through an entry that restores it.
  Suspending across a host frame is an error (as with Lua's "yield across a
  C-call boundary") unless the coroutine runs on a `std.rt.Fiber`, which has
  its own stack.

---

## 11. What Loimi deliberately leaves out

- **No JIT.** However, each handler is a separate LLVM function with a known
  register interface, which is exactly the input a copy-and-patch baseline JIT
  needs (CPython 3.13 builds its JIT from LLVM-compiled templates).
  See `roadmap.md`.
- **No borrow checker or ownership types.** Arenas and the GC cover
  interpreter workloads. The complexity budget goes to the hot loop.
- **No exceptions or unwinding in host code.** Guest exceptions are a VM
  concept (`raise` / `on raise`). Host errors are `Result`.
- **No implicit runtime.** No hidden allocations, no implicit GC outside `std.gc`.
