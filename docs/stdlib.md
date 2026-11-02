# Standard library

The language handles the inner loop. The standard library handles the rest of
an interpreter: getting from source text to verified bytecode, and providing
the runtime structures every dynamic language needs. Modules are listed
roughly in the order a request flows through an interpreter.

Design rules for the library:

- **Generated per VM where it helps.** Builders, disassemblers, and verifiers
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
    if let Some(k) = lx.ops.eat(c) { return Ok(lx.tok(k, start)); }
    Err(c.error_at(start, "unexpected character"))
}
```

- `Syntax` holds the per-language conventions: line comment prefix, block
  comment delimiters (optionally nesting), Lua long-bracket comments, octal
  or decimal `\ddd`, whether backslash-newline keeps the newline, raw
  newlines in strings, and whether overflowing decimal/hex literals become
  floats/wrap or are errors. Presets: `lua()`, `c_like()`, `none()`.
- `Cursor`: `peek`, `peek_at`, `bump`, `advance`, `eat`, `eat_str`,
  `starts_with`, `eat_while(fn(u8) -> bool)`, `since`, `span_from`,
  `save`/`restore`, and `line` tracking. Character classes are ASCII.
- Scanners: `skip_trivia`, `ident`, `at_number`/`number` (via std.num, with
  `3abc` reported as malformed), `quoted` (escapes decoded into a `StrBuf`;
  after a bad escape it still scans to the closing quote so the lexer can
  continue), `long_bracket_level`/`long_string`.
- `OpTable<K>`: operator strings to token kinds, maximal munch through
  per-first-byte chains sorted longest first. Matching is
  `ops.eat(&cursor)` because methods cannot have their own type parameters.
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
register or emits the jump of `and`/`or`). Postfix operators belong in
`atom`. A hook that needs to recurse (parentheses) reaches the table through
a pointer in `P`. `TokenStream` and error recovery are not built. `Span` and
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
