# Standard library

The language handles the inner loop. The standard library handles the rest of
an interpreter: getting from source text to verified bytecode, and providing
the runtime structures every dynamic language needs. Modules are listed
roughly in the order a request flows through an interpreter.

Design rules for the library:

- **Generated per VM where it helps.** Encoders, disassemblers, and verifiers
  come from `vm` declarations. The library supplies the generic machinery they
  plug into.
- **Arena-first.** Front-end data (tokens, AST, IR) lives in arenas and is
  freed in one go.
- **Fast paths inline, slow paths `cold`, effects declared.** The library
  follows the same discipline it asks of users.

---

## `std.core`

Integers with checked, wrapping, and overflowing arithmetic. Bit operations
(`clz`, `ctz`, `popcount`, `rotl`, `bswap`), `bitcast`, IEEE `f64` functions
(`fma`, `copysign`, `trunc`, `floor`, `nextafter`), `Option`, `Result`,
slices, `fmt` (compile-time checked format strings), and `assert` /
`debug_assert`.

Guest-semantics helpers that are easy to get subtly wrong:

- `floor_div` / `floor_mod` (Python and Lua semantics) and `trunc_div` /
  `trunc_rem` (C and JS semantics), each with the `MIN / -1` edge case handled;
- `f64.to_i64_exact() -> ?i64` (for int/float keys), `f64.to_i32_js()`
  (JS `ToInt32`), and saturating conversions.

## `std.mem`

- `Arena`: bump allocator with `reset`, scoped sub-arenas, and typed
  allocation (`arena.new(T)`, `arena.slice(T, n)`).
- `Pool<T>`: fixed-size free-list allocator.
- `Id<T>` / `IndexVec<Id<T>, T>`: typed 32-bit indices, the usual way to
  reference AST and IR nodes without pointers.
- `Vec<T>`, `SmallVec<T, N>`.

## `std.collections`

- `HashMap<K, V>` / `HashSet<T>`: SwissTable-style open addressing.
- `IndexMap`: insertion-ordered, for guest dicts with ordering guarantees
  (Python and JS objects).
- `BitSet`, `Deque`, `BinaryHeap`.
- `std.collections.Table`: a **Lua-style hybrid table**. A dense array part
  plus a hash part, with array-part migration. Keys are `Value`s through a
  user-supplied hash/eq interface, so it works with any `value` type.

**As implemented**: `std.map.HashMap<K: hash.Hash, V>` (linear probing).
`std.hash.Hash` is a trait (`hash(k: Self) -> u64`, `eq(a: Self, b: Self)
-> bool`) implemented for `u64`, `i64`, `u32`, `i32` and `str`; a key type
defines both methods and declares `impl hash.Hash for Key;`.

## `std.text`

- `str` (UTF-8 slice), `String` (host-owned), and `Gc<Str>`: a GC string
  object with cached hash, byte length, and code-point length (or an
  "ASCII-only" flag).
- `Interner` → `Symbol` (u32). Symbol comparison is integer comparison, and
  symbols can be pre-seeded at compile time
  (`const SYM_INIT = intern!("__init__")`).
- `StrBuilder`, rope concatenation for repeated `..`/`+=`, UTF-8 and UTF-16
  iteration and validation (JS needs UTF-16 views), and case mapping.
- `hash`: fast non-cryptographic hash with a per-process random seed
  (to resist hash flooding), with a streaming API.

## `std.num`

- `BigInt` with a **small-int fast path**: a value that fits in a machine
  word stays inline, and overflow promotes to heap limbs. This is what Python-
  and Ruby-style integers need. Multiplication is schoolbook below a threshold
  and Karatsuba above it. Also `divmod`, `pow`, `gcd`, and string conversion in
  any radix.
- `f64` ↔ string: **shortest round-trip** formatting (Ryu-style) and correct
  parsing (Eisel-Lemire fast path with an exact fallback). Guest
  `tostring(0.1)` should never print `0.1000000000000000055`.
- Numeric-literal parsing shared by the lexer and runtime `tonumber`: hex,
  binary, underscores, and exponents, configurable per guest language.

**As implemented** (`std/num.lm`): no `BigInt` yet. Parsing returns
`Result<_, NumError>` with `Invalid` and `Overflow` kept apart:
`parse_int` (sign, decimal or `0x` hex, exact i64 range), `parse_i64` /
`parse_u64` (radix 2 to 36), `parse_u64_wrapping` (Lua hex literals),
`parse_f64` (strict decimal syntax checked first, then correctly rounded by
strtod), `parse_number` (`Number.Int` or `Number.Float`, decimal overflow
becomes a float as in Lua) and `decimal_len` (the extent of a numeral, for
lexers). Formatting appends to a `StrBuf`: `write_g(b, x, 14)` (Lua's
`%.14g`), `write_shortest` (shortest round-trip, Python `repr` notation
rules, by trying `%.{1..17}e` with snprintf rather than Ryu), `mark_float`
(append `.0` to integer-looking output) and `write_hex`. Binary literals and
digit separators are not supported. The `std.core` guest-arithmetic helpers
live here too: `floor_div`, `floor_mod`, `trunc_div`, `trunc_rem` (None on
a zero divisor; `MIN / -1` wraps), `floor_div_f64`, `floor_mod_f64`,
`add_overflow`/`sub_overflow`/`mul_overflow` returning `(i64, bool)`, and
`to_i64_exact`. The built-in `add_checked`/`sub_checked`/`mul_checked`
methods cover the `?T` forms.

## `std.lex`: lexer generator

The token specification is turned into a DFA at **compile time** (comptime),
so the runtime is a table-driven loop with no regex engine:

```loimi
const Tok = std.lex.define(.{
    .skip     = .{ "[ \t\r\n]+", "--[^\n]*" },
    .tokens   = .{ .Num = "[0-9]+(\\.[0-9]+)?([eE][+-]?[0-9]+)?",
                   .Name = "[A-Za-z_][A-Za-z0-9_]*",
                   .Str  = std.lex.quoted('"', escapes = .c) },
    .keywords = .{ .from = .Name, "if", "then", "else", "end", "while", "do",
                   "function", "return", "local", "nil", "true", "false" },
    .punct    = .{ "+", "-", "*", "/", "==", "~=", "<", "<=", "=", "(", ")",
                   "{", "}", ",", ".", ".." },
});

var lx = Tok.Lexer.init(source);
while lx.next() is Some(t) { ... t.kind, t.span, lx.text(t) ... }
```

It uses longest match, priority by declaration order, and keyword tables via
perfect hashing. Hand-written lexers can still use `std.lex.Cursor`
(peek, advance, spans, and UTF-8 aware character classes).

**As implemented** (`std/lex.lm`): without comptime there is no generator;
the module is the `Cursor` toolkit, and a lexer is a short function that
tries the scanners in order:

```loimi
fn Lexer.next(lx: *Lexer) -> Result<Token, lex.Error> {
    let c = &lx.cur;
    c.skip_trivia()?;
    let start = c.pos;
    if c.at_end() { return Ok(lx.tok(T.Eof, start)); }
    if let Some(name) = c.ident() { return Ok(lx.tok(lx.keywords.get(name) ?? T.Name, start)); }
    if c.at_number() { lx.num = c.number()?; return Ok(lx.tok(T.Number, start)); }
    if c.peek() == '"' { lx.buf.clear(); c.quoted(&lx.buf)?; return Ok(lx.tok(T.String, start)); }
    if let Some(k) = c.op(&lx.ops) { return Ok(lx.tok(k, start)); }
    Err(c.error_at(start, "unexpected character"))
}
```

- `Syntax` holds the per-language conventions: line comment prefix, block
  comment delimiters (optionally nesting), Lua long-bracket comments, octal
  or decimal `\ddd`, whether backslash-newline keeps the newline, raw
  newlines in strings, whether overflowing decimal/hex literals become
  floats/wrap or are errors, and extra identifier bytes: `ident_extra`
  (anywhere, `"$"` for JavaScript) and `ident_inner` (after the first
  byte: `"?!"`, `"-"`). Presets: `lua()`, `c_like()`, `js()`, `none()`.
  The cursor turns the identifier classes into 256-bit sets
  (`cursor.is_ident_start(b)`, `cursor.is_ident_char(b)`).
- `Cursor`: `peek`, `peek_at`, `bump`, `advance`, `eat`, `eat_str`, `op`,
  `starts_with`, `eat_while(fn(u8) -> bool)`, `since`, `span_from`,
  `save`/`restore`, and `line` tracking. Character classes are ASCII.
- Scanners: `skip_trivia`, `ident`, `at_number`/`number` (via std.num, with
  `3abc` reported as malformed), `quoted` (escapes decoded into a `StrBuf`;
  after a bad escape it still scans to the closing quote so the lexer can
  continue), `long_bracket_level`/`long_string`.
- `OpTable<K>`: operator strings to token kinds, maximal munch through
  per-first-byte chains sorted longest first. Matching is
  `cursor.op(&ops)` (or the older `ops.eat(&cursor)`).
- `Keywords<K>`: a `HashMap<str, K>` wrapper (no perfect hashing yet).
- Errors are `lex.Error { span, msg }` with a static message;
  `e.diag()` turns one into a `diag.Diag`.

## `std.parse`

- `Pratt<Ast>`: precedence-climbing helper driven by a comptime operator table
  (`.{ .{"or", 1, .left}, .{"..", 9, .right}, ... }`), with prefix and postfix
  hooks.
- `TokenStream`: lookahead, `expect`, and error recovery to synchronization
  tokens (`recover_to(.{ .Semi, .End })`) so one syntax error doesn't hide the rest.
- `Span`, `SourceId`, `SourceMap` (file → line/column, computed lazily).

**As implemented** (`std/parse.lm`): `Pratt<P, K, E, X>` over parser state
`P`, a payload-free token-kind enum `K` (used as a table index via
`k as u64`), expression result `E` and error type `X`. The table is filled
at run time (`infix_left(k, bp)`, `infix_right(k, bp)`, `infix(k, l, r)`,
`prefix(k, bp)`), and `parse(p, min_bp)` calls the user's `peek`,
`advance`, `atom`, `unary` and `binary` functions and an optional
`before_rhs` hook (where a single-pass compiler puts the left operand in a
register or emits the jump of `and`/`or`). Operators that are not
`lhs op rhs` get a per-kind `led` hook (`led(k, left, right, hook)`):
when the operator binds, the driver consumes it and the hook parses the
rest and returns the result. That covers mixfix (`c ? a : b`), postfix
(`n!`) and right-hand sides with their own grammar (assignment with a
target check, `x => body`); the hook parses operands with
`pr.parse(p, pr.infix_of(k).right)`. MiniJS parses assignment, `?:` and
arrows this way. Postfix chains such as calls and indexing can stay in
`atom`. A hook that needs to recurse (parentheses) reaches the table through
a pointer in `P`. The hooks' types have no effects, so `parse` has none
and runs from any function: hooks may allocate but not collect (collect
between parses). `TokenStream` and error recovery are not built. `Span` and
`SourceMap` live in `std.diag`.

## `std.diag`

Diagnostics with labeled spans, notes, and fix-it hints, rendered with
source snippets and carets in plain text or ANSI color. Compiler phases push
into a `Diagnostics` sink, and the same machinery formats guest runtime
errors with tracebacks.

**As implemented** (`std/diag.lm`): `Span { file, lo, hi }`; `SourceMap`
(borrowed names and text; `add`, `line_col` by binary search in a line-start
table built on first use, `line_text`, `snippet`, `write_location`); `Diag`
with a `Severity`, a message, an optional primary span with a label, and
notes (`with_label`, `with_note`, `with_note_at`, `with_help`), rendered in
the same format as the Loimi compiler's own errors (tabs preserved under the
caret line, spans clipped to their first line); and `Diagnostics`, which
copies the messages pushed into it into an arena (so they can be built in a
temporary `StrBuf`), counts errors and warnings, and prints a summary such as
`1 warning and 2 errors generated.` Not built: fix-it hints, ANSI color,
multi-line spans, tracebacks.

## `std.ast`

Arena-allocated trees with `Id<T>` links, a generic visitor via comptime
reflection, and scope and resolver helpers: `Scopes<Symbol, Binding>` with
local, upvalue, and global classification, which is the core of Lua-, JS-, and
Python-style closure compilation.

## `std.bc`: bytecode construction

Generic machinery that each vm's generated `Builder` plugs into:

- **Emission:** typed emit functions generated from ops (`b.ADD(r1, r2, r3)`).
  Inline caches are reserved and zeroed automatically.
- **Labels:** `b.label()`, `b.bind(l)`, and forward references with
  backpatching. If an offset doesn't fit its field, emission reports it, or
  applies the op's declared relaxation rule (for example a short conditional
  jump over a long `JMP`).
- **Constant pool** with deduplication (by value bits, so `0.0` and `-0.0`
  stay distinct).
- **Register allocation for expression temporaries** (`b.regs.alloc()`,
  scoped release), the Lua "freereg" discipline packaged as a reusable piece,
  plus tracking of each function's maximum register count.
- **Line table:** compressed pc → span mapping (delta-encoded, as in CPython's
  location table) with `lookup(pc)`.
- **Exception table:** ranges → handler pc, used by `on raise`.
- **Peephole pass** generated from `super` declarations.
- `Proto` assembly: code, constants, nested protos, debug info. Calls the
  generated verifier and returns `Verified<V>`.
- **Serialization** of protos (bytecode cache files). Loading always goes
  through the verifier.

**As implemented** (`std/bc.lm`): a library over the generated encoders
rather than a generated `Builder`. Everything is generic over the vm's
code word `W` through two functions the compiler passes in: `size:
fn(W) -> u64` (the generated `Vm.op_size`) and `patch: fn(W, i64) -> ?W`
(set a jump word's offset, `target - pc`, or None if it does not fit:
the generated `Vm.set_label`, or a few lines over `Vm.opcode` and
`bc.Field` for a custom encoding). The test
`tests/run/std_bc.lm` compiles a small language to a toy register vm
with all of it.

- `Code<W>`: `Code.new(Vm.op_size, patch)`; `op(w)` emits an instruction
  and zeroes its inline-cache words, `word(w)` a raw word; `pc`, `at`,
  `set`, `words`, `copy_into`, `clear`, `free`. Source positions:
  `set_loc(loc)` (returns the previous one) tags the following
  instructions; positions are kept as runs, read with `loc_at(pc)`,
  expanded per word with `locs_into(out)` (what Mini and Scheme store
  today), or encoded with `encode_locs(&bytes)` (delta-encoded LEB128
  runs, about 2 bytes per line change) and read back with
  `bc.line_at(bytes, pc)` on error paths.
- Labels: `label()`, `bind(l)`, `here()` (a placed loop head),
  `jump(w, l)`, `fixup(at, l)` (for a jump word that is not the first
  word emitted), `label_pc`. Jump lists for Lua-style compilers that pass
  pending exits around: plain `mem.Vec<u64>` of jump pcs with
  `patch_list(&l, target)` and `patch_here(&l)`; `patch(at, target)` and
  `jump_to(w, target)` for single jumps.
- Errors are sticky: an offset that does not fit records `JumpRange` with
  the pc and its source position, `check()` returns the first error (also
  `Unbound` for a label with pending jumps that was never placed), and
  `Error.message()` describes it. Compilers that stop at the first error
  can still check `patch`'s bool result.
- Peephole support: `last()` is the pc of the previous instruction if it
  may be rewritten or merged with the next one (no jump targets the
  current pc: binding a label or patching to here blocks it, as Lua's
  `lasttarget` does); `truncate(pc)` removes code (dropping pending
  fixups in it); `mark_target()`.
- `thread_jumps(target_of, is_goto)`: retargets jumps that land on an
  unconditional jump, following chains up to 8 deep; nothing moves.
- `bc.Field { shift, bits, signed }` with `fits`, `get`/`set` (and
  `get32`/`set32`): range-checked access to an encoded field.
- `Pool<K: hash.Hash>`: `Pool.new(limit)`, `add(k) -> ?u32` (deduplicated
  through a `HashMap`; None when full), `push` (no dedup), `find`, `get`,
  `items`. For a nanboxed `Value`, `fn V.hash` over the bits and
  `fn V.eq(a, b) { a == b }` keep `0.0`, `-0.0` and `0` apart.
- `Regs`: `Regs.new(limit)`; `alloc(n)` (past the limit it sets `failed()`
  and still returns, so the error can be reported later) or
  `try_alloc(n) -> ?u32`; `free(r)` (top only, locals ignored, false if
  out of order), `free2(a, b)`, `is_temp`, `mark`/`reset` (Scheme's
  save-and-restore of the free register), `end_stmt`, `activate(n)`
  (registers become locals), `enter`/`leave` (blocks), `frame_size(min)`.

Not built: emitters generated as `b.ADD(..)` methods (the generated
`Vm.ADD(..)` encoders plus `op` cover it), branch relaxation (a too-long
jump is an error), the exception table, `Proto` assembly with the
verifier call (each interpreter's `Proto` and verify context differ; the
compiler calls `Vm.verify` and `Vm.optimize` itself), and serialization.
Nothing in `std.bc` collects, so GC references in a `Pool` are not
roots: compile between collections, as Mini and Scheme do.

## `std.gc`

Full design: [`stdlib/gc.md`](stdlib/gc.md).

- Heap policies (compile-time parameter `#![heap = ...]`):
  - `MarkSweep` (default): non-moving, size-segregated, lazy sweeping,
    no write barrier.
  - `Incremental`: tri-color with a snapshot-at-the-beginning barrier, for
    bounded pause times.
  - `Generational`: non-moving sticky-mark-bits generations with an
    object-logging barrier.
- Allocation: per-size-class free lists with an inlined fast path of about
  five instructions, and a `cold` refill path marked `uses gc`.
- Precise roots generated from `vm` state kinds, frame structs, and effect
  spills. Inline caches in bytecode are *weak* roots, reset when their targets die.
- `Root<T>`, `GcVec<T>`, `Weak<T>`, finalizers (run outside the collector),
  and `EphemeronTable` (for JS `WeakMap` and Lua weak tables).
- Stress mode, heap verification, poisoning, and heap snapshots.

**As implemented**: every object has a 16-byte header (slot size, type
id, mark bits, and the trailing-array length, which compiled code reads
at object - 8; a free slot links the next one through the length), so a
two-word object such as a Lisp pair takes a 32-byte slot. Root scanners and sweepers (`Heap.add_scanner`,
`add_sweeper`) are `fn(*Heap, *u8)` without effects: they run during a
collection and must not start one, so a `uses gc` function is rejected.
The growth policy is `Heap.set_growth(percent, min_heap)`: after a
collection, `percent`% of the live bytes (at least `min_heap`) may be
allocated before the next one. The default, 100% and 1 MiB, collects
again after allocating as much as survived (the heap peaks at about twice
the live data). MiniJS uses 300% with an 8 MiB minimum (as fast as
100% / 64 MiB, a quarter of the memory on small heaps; see
`examples/minijs/README.md`). The default stays because no setting was
better for every interpreter (best of 3-7, wall clock, loaded machine):
Mini at 200% runs binary_trees 17% faster (1.21 to 1.01 s) and the rest
within noise; Scheme at 200% runs nboyer, graphs and mperm 4-10% faster
but string 27% slower (41 to 52 ms) and primes and array1 3-4% slower;
an 8 MiB minimum makes Scheme's small allocation-heavy programs (deriv,
divrec, cpstak) 6-10% slower, as the heap leaves the CPU caches. Mini
and Scheme can opt in with `set_growth`. `Heap.set_max_budget(bytes)`
caps the budget (0, the default, is no cap), so that a large growth
factor bounds the heap at the live data plus `bytes` rather than at a
multiple of it once the live data is large. Scheme uses 400%, a 2 MiB
minimum and a 256 MiB cap (see `bench/scheme/README.md`).

## `std.obj`: objects and shapes

Full design: [`stdlib/obj.md`](stdlib/obj.md).

- A compile-time `Model` configuration chooses attributes, prototypes,
  ordering, element storage, and thresholds per guest language.
- `Shape`: hidden classes in a transition tree with weak edges, stable
  `ShapeId`s, and lazily built lookup tables.
- `Object`: shape, inline slots (sized by allocation-site templates), overflow
  slots, optional elements. Falls back to dictionary mode.
- Inline cache layouts and state machine (mono, poly, megamorphic with a
  global `MegaCache`), validity cells for prototype-chain hits, and property
  cells for globals.

**As implemented** (`std/obj.lm`): shapes with weak transitions and ids
that stay stable across collections (an id names a transition path),
inline and overflow slots, dictionary mode, per-property attribute bits,
prototype chains with validity cells (plain ids, so a prototype hit is a
few plain words), the inline-cache state machine (`obj.Site`,
`read_miss`/`write_miss`: refill, polymorphic, megamorphic, cached
property adds) and allocation templates. There is no comptime `Model` configuration and object types
cannot be generic, so the interpreter declares its object type (fields
`shape`, `dict`, `overflow`, optionally `proto`, and trailing `inline:
[Value]`) and a slot-array type, and instantiates the generic
`obj.Model<Gc<Obj>, Gc<Slots>, Value>` with a function that allocates
slot arrays. Keys are u64 symbol ids. An inline-cache hit is two loads,
a compare and the slot load (`bench/obj`). Details and what is left out:
[`stdlib/obj.md`](stdlib/obj.md#as-implemented).

## `std.rt`: runtime support

- `ValueStack<T>`: the VM stack. It **reserves virtual address space up front**
  and commits pages on demand, so it never moves. `Window` and `StackTop`
  pointers therefore stay valid, with no Lua-style pointer fixups after
  `realloc`. Overflow is a single compare against the limit, done once per call.
- `FrameStack<F>`: guest call frames, with entry markers for reentrant calls.
- `Upvalues`: open-upvalue list and closing (Lua-style), for closure capture by
  reference with stack-allocated locals.
- `Fiber`: stackful coroutines (separate host stack), for guest coroutines
  that must suspend across host frames.
- C-stack depth guard for `reenter`.

## `std.prof`

- `--vm-stats` support: per-op counts, op-pair counts, and `suggest-supers`.
- Sampling profiler: a timer signal reads `ctx.sync.pc` (made accurate by an
  instrumentation variant that syncs on backward jumps and calls) and maps it
  through the line table to guest source lines. It outputs folded stacks for
  flame graphs.

## `std.ffi`, `std.io`, `std.os`

These are minimal: C interop types (`CStr`, `c_int`, ...), files, stdin and
stdout, environment variables, clocks, `mmap`, and dynamic loading. Enough to
build a REPL and a native-module system, and no more.

## `std.test`: interpreter testing

- Golden tests: run every `tests/**/*.<ext>` script and compare stdout and
  stderr with `-- expect:` comments or `.out` files. Supports
  `--update` and parallel runs.
- **Mode matrix:** each test runs under every dispatch strategy, value repr,
  and GC policy you ask for (`--matrix dispatch=*,gc=MarkSweep,stress`). That
  catches miscompilation and rooting bugs that only show up in one
  configuration.
- Differential testing against a reference interpreter binary.
- Bytecode fuzz harness: generates random code, checks that the verifier
  either rejects it or the VM runs it without memory errors (under ASan).
- `std.bench`: benchmark runner with warmup, multiple runs, and
  confidence intervals.
