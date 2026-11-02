# Standard library

The language handles the inner loop. The standard library handles the rest of
an interpreter: getting from source text to verified bytecode, and providing
the runtime structures every dynamic language needs.

Each module section below starts with **what exists** (the API in
`std/*.lm`, which you can import today), followed by a **Design (not
built)** part describing what the module was planned to grow into. Do not
plan around the design parts: nothing in them exists unless the built part
says so. Some modules are design only; their sections say so at the top.

| Module | Status | Source |
|---|---|---|
| `std.core` (built-ins) | built in part (no `assert`, `rotl`, ...; formatting is in `std.text`) | the compiler |
| `std.mem` | built | `std/mem.lm` |
| `std.hash`, `std.map` | built (`HashMap` only) | `std/hash.lm`, `std/map.lm` |
| `std.text` | built (`StrBuf` with `{}` formatting, `Interner`, small helpers) | `std/text.lm` |
| `std.num` | built (no `BigInt`) | `std/num.lm` |
| `std.io`, `std.os` | built (minimal) | `std/io.lm`, `std/os.lm` |
| `std.lex` | built (a cursor toolkit, no generator) | `std/lex.lm` |
| `std.parse` | built (`Pratt`, `Parser`; no `TokenStream`) | `std/parse.lm` |
| `std.diag` | built | `std/diag.lm` |
| `std.bc` | built (a library, no generated `Builder`) | `std/bc.lm` |
| `std.gc` | built (mark-sweep, optional generational mode) | `std/gc.lm` |
| `std.obj` | built (no comptime `Model`) | `std/obj.lm` |
| `std.rt` | `protect`/`throw` only (no `ValueStack`, `FrameStack`, `Fiber`) | `std/rt.lm` |
| `std.collections` (beyond `HashMap`) | **not built** | |
| `std.ast` | **not built** | |
| `std.prof` | built (clock, `Bench`, `Counters`, `--vm-stats` counts; no sampling profiler) | `std/prof.lm` |
| `std.ffi` | **not built** (`extern "C"` is in the language) | |
| `std.test` | built (`#[test]`, `loimi test`, assertions, capture, golden files; no matrix or differential runner) | `std/test.lm` |

Design rules for the library:

- **Generated per VM where it helps.** Encoders, disassemblers, and verifiers
  come from `vm` declarations (`language.md` §3.3). The library supplies the
  generic machinery they plug into.
- **Arena-first.** Front-end data (tokens, AST, IR) lives in arenas and is
  freed in one go.
- **Fast paths inline, slow paths `cold`, effects declared.** The library
  follows the same discipline it asks of users.
- **Everything that owns memory can give it back.** Each std type that
  holds heap memory has a `free()` method (`Vec`, `Arena`, `HashMap`,
  `StrBuf`, `Interner`, `OpTable`, `Keywords`, `Pratt`, `Parser`,
  `SourceMap`, `Diag`, `Diagnostics`, `Code`, `Pool`, `Heap`, `Shapes`,
  `MegaCache`, `Ics`, `Model`, `Ephemerons`, `Bench`, `Counters`), and `io.read_file`'s result
  is released with `io.free_file`. Freeing a container does not free what
  its elements point to.

---

## `std.core`: built-in types and operations

There is no `std/core.lm`: these are built into the compiler and need no
import.

- **Types**: `i8`..`i64`, `u8`..`u64`, `isize`, `usize`, any-width
  `iN`/`uN` (1..=64), `f32`, `f64`, `bool`, `str` (a `[]u8` slice), slices
  `[]T` (`.len`, `.ptr`, `s[a..b]`), arrays `[T; N]`, pointers `*T`,
  `?T`, `Result<T, E>`, tuples (language.md §1.1).
- **Integer arithmetic**: `+ - *` trap on overflow in debug builds and
  wrap in release builds; `+% -% *%` always wrap;
  `a.add_checked(b)`, `a.sub_checked(b)`, `a.mul_checked(b)` return `?T`.
  The `(T, bool)` forms are functions in `std.num` (`num.add_overflow(a,
  b)` etc., `i64` only).
- **Bit operations**: `x.clz()`, `x.ctz()`, `x.popcount()` on any
  integer type; shifts `<< >>` (language.md §1.2).
- **Floats**: `x.sqrt()`, `x.floor()`, `x.to_bits() -> u64`,
  `f64.from_bits(u64)`; `as` between integers and floats. For floor
  division and the rest, see `std.num`.
- **Options**: `o.is_some()`, `o.is_none()`, `o.unwrap()` (traps on
  None), `o.unwrap_or(d)`, `o ?? d`, and `?` (also on `Result`).
  `o ?? p` with `p: ?T` is `?T`: `o` if it is `Some`, else `p`, which is
  evaluated only then (Rust's `o.or_else(|| p)`; chains:
  `a ?? b ?? c`).
  `Result` has no methods: use `match`, `if let` or `?`.
- **Pointers**: `p.offset(n)` (`i64` elements), `p.slice(n) -> []T`,
  `p + n`, `p[i]`, `*p`.
- **Other built-ins**: `size_of<T>() -> u64`; `likely(c)`,
  `unlikely(c)`, `assume(c)`; `trap() -> never` and `unreachable`;
  `new(heap, Obj { .. }, n)` and `gc_mark(heap, v)` with `std.gc`
  imported (§8 of language.md).
- **String equality** is `a.eq(b)`, defined in `std.hash` (`import
  std.hash;`); `==` does not compare slices.

### Design (not built)

Not built: `fmt` as compile-time checked format strings (`text.StrBuf`
has `fmt`/`add` with run-time `{}` placeholders, see `std.text`), `assert` / `debug_assert` (write `if !c { trap(); }`),
`rotl`, `bswap`, `bitcast` (use `to_bits`/`from_bits` and `as`), the f64
functions `fma`, `copysign`, `trunc`, `nextafter` (and `abs`, `ceil`,
`is_nan`), `a.add_overflow(b)` as a method, `f64.to_i32_js()`, and
saturating conversions. The guest-arithmetic helpers that were planned
here (`floor_div`, `floor_mod`, `trunc_div`, `trunc_rem`,
`to_i64_exact`) are built, in `std.num`.

## `std.mem`

`std/mem.lm`:

- `mem.alloc<T>(n) -> *T` (zeroed; out of memory is a panic, which
  aborts unless caught, see `std.rt`), `mem.try_alloc<T>(n) -> ?*T`
  (None when out of memory), `mem.grow<T>(p, n) -> *T`, `mem.release(p)`,
  `mem.copy(dst, src, n)` (overlapping allowed).
- Byte strings (any `[]u8`, so `str` too; libc underneath):
  `mem.find_byte(s, c) -> ?u64` (`memchr`), `rfind_byte` (`memrchr`),
  `find_bytes(s, needle) -> ?u64` (`memmem`), `compare_bytes(a, b) -> i32`
  (-1, 0, 1; a prefix sorts first), `equal_bytes(a, b)`. For a raw
  pointer, `p.slice(n)` first.
- `Vec<T> { ptr, len, cap }`: `Vec.new()`, `with_capacity(n)`,
  `reserve(extra)`, `push`, `pop -> ?T`, `get(i) -> ?T`, `at(i)`,
  `ptr_at(i)`, `set(i, x)`, `last -> ?T`, `items -> []T`,
  `truncate(n)`, `clear`, `free`, `sort_by(cmp)`.
  **An empty `Vec` allocates nothing**: `Vec.new()` (and
  `with_capacity(0)`) has capacity 0 and a static sentinel pointer, and
  the first push allocates. `free()` leaves the `Vec` empty and usable
  (freeing twice is fine). `mem.release` and `mem.grow` accept the
  sentinel, so code that takes over `v.items().ptr` works for empty
  vectors too; do not pass it to C's `free`.
- **Objects with optional resources.** A `*T` field cannot be null and
  most structs need a value for every field, so an object built before
  its parts exist (a compiled regex that may carry an error instead)
  needs placeholders. Make each optional part a `Vec<T>` (an empty one
  is free to create, overwrite and drop) or a `?*T` (None until
  allocated, released with `if let Some(p) = x.p { mem.release(p); }`),
  never a `mem.alloc(1)` dummy, which leaks when overwritten.
- `mem.sort_by(p, n, cmp)`: a stable merge sort; `cmp` is a
  `block fn(*T, *T) -> i32`, a closure capturing locals or a plain
  function.
- `Arena`: `Arena.new()`, `alloc_bytes(n, align) -> *u8`, `str(s) -> str`
  (copy a string into the arena), `free()`. 64 KiB chunks; larger requests
  get their own chunk.

### Design (not built)

`Arena.reset`, scoped sub-arenas and typed arena allocation
(`arena.new(T)`, `arena.slice(T, n)`); `Pool<T>` (fixed-size free-list
allocator); `Id<T>` / `IndexVec<Id<T>, T>` typed indices; `SmallVec<T, N>`.

## `std.hash` and `std.map`

`std/hash.lm`, `std/map.lm`:

- `hash.Hash` is a trait: `fn hash(k: Self) -> u64` and
  `fn eq(a: Self, b: Self) -> bool`, implemented for `u64`, `i64`,
  `u32`, `i32` and `str`. A key type defines both methods and declares
  `impl hash.Hash for Key;`. `hash.mix(x)` (a 64-bit mixer) and
  `hash.bytes(s)` are the building blocks.
- `map.HashMap<K: hash.Hash, V>` (linear probing, at most 3/4 full):
  `HashMap.new()`, `insert(k, v) -> bool` (true if the key was new),
  `get(k) -> ?V`, `get_ptr(k) -> ?*V`, `contains`, `remove -> bool`,
  `len` (a field), `clear()` (keeps the table's memory), iteration with
  `var i: u64 = 0; while let Some(s) = m.next(&i) { .. s.key, s.value .. }`,
  and `free`.

### Design (not built): `std.collections`

Everything here except `HashMap` is design only:

- `HashSet<T>`, SwissTable-style probing.
- `IndexMap`: insertion-ordered, for guest dicts with ordering guarantees
  (Python and JS objects).
- `BitSet`, `Deque`, `BinaryHeap`.
- `std.collections.Table`: a **Lua-style hybrid table**. A dense array part
  plus a hash part, with array-part migration. Keys are `Value`s through a
  user-supplied hash/eq interface, so it works with any `value` type.
  (Mini, Scheme and MiniJS each write their own.)
- A per-process random hash seed (to resist hash flooding) and a
  streaming hash API.

## `std.text`

`std/text.lm`:

- `StrBuf`, a growable byte string: `StrBuf.new()`, `push(byte)`,
  `push_str(s)`, `push_int(i64)`, `push_f64(f64)` (shortest `%g` that
  reads back as the same number), `as_str()`
  (valid until the next push), `len()`, `clear()`, `free()`,
  `push_u64(u64)`. `std.num` has more float formats.
- Formatting, for messages: `b.fmt2("expected {} values, got {}", n, m)`
  appends the format string with each `{}` replaced by the next
  argument (`fmt` takes one argument, `fmt2` to `fmt4` two to four;
  `{{` and `}}` are literal braces). `b.add(x)` appends one value. Both
  return the buffer, so calls chain:
  `b.fmt("unknown word `{}`", name).add(" at line ").add(line).as_str()`.
  The format string is not checked at compile time: a `{}` without an
  argument stays `{}`, and an argument without a `{}` is appended at the
  end. Arguments are any type that implements the trait `text.Fmt`
  (`fn fmt(x: Self, b: *StrBuf)`): `str`, all the `i*`/`u*` widths up to
  64 bits (`u8` prints as a number; `push` appends a byte), `f64` (as
  `push_f64`) and `bool`. A type of your own joins with
  `fn T.fmt(x: T, b: *text.StrBuf) { .. }` and `impl text.Fmt for T;`.
- `Interner`: `Interner.new()`, `intern(s) -> u32` (ids 0, 1, 2, ...;
  the string is copied into the interner's arena), `find(s) -> ?u32`,
  `name(id) -> str`, `free()`.
- Helpers: `starts_with(s, prefix)`, `find(s, byte) -> ?u64` (`memchr`),
  `is_digit`, `is_alpha` (includes `_`), `is_space`, `parse_int(s) -> ?i64`
  (decimal), `parse_f64(s) -> ?f64` (C syntax, via strtod). `std.num` has
  the exact parsers with error kinds.

### Design (not built)

`String` (host-owned), `Gc<Str>` (a GC string object with cached hash,
byte length and code-point length; interpreters declare their own string
`object`), `Symbol` as a type and compile-time pre-seeded symbols
(`intern!("__init__")`), rope concatenation, UTF-8 and UTF-16 iteration
and validation, and case mapping.

## `std.num`

`std/num.lm`. Parsing returns `Result<_, NumError>` with `Invalid` and
`Overflow` kept apart (`num.error_message(e)`):

- `parse_int(s)` (sign, decimal or `0x` hex, exact `i64` range),
  `parse_i64(s, radix)` / `parse_u64(digits, radix)` (radix 2 to 36),
  `parse_u64_wrapping` (Lua hex literals), `parse_f64(s)` (strict decimal
  syntax checked first, then correctly rounded by strtod),
  `parse_number(s) -> Number` (`Number.Int(i64)` or `Number.Float(f64)`;
  decimal overflow becomes a float, as in Lua), `decimal_len(s)` (the
  extent of a numeral, for lexers), `digit_value(c)`, `is_hex_digit(c)`.
- Formatting appends to a `text.StrBuf`: `write_g(b, x, 14)` (Lua's
  `%.14g`), `write_shortest(b, x)` (shortest round-trip, Python `repr`
  notation, by trying `%.{1..17}e` with snprintf), `shortest(x)` (the
  digits and exponent) and `write_js(b, x)` (ECMAScript
  Number::toString), `mark_float(b, start)` (append `.0` to
  integer-looking output) and `write_hex(b, x)`.
- Guest arithmetic: `floor_div`, `floor_mod`, `trunc_div`, `trunc_rem`
  (`?i64`: None on a zero divisor; `MIN / -1` wraps), `floor_div_f64`,
  `floor_mod_f64`, `add_overflow` / `sub_overflow` / `mul_overflow`
  returning `(i64, bool)`, and `to_i64_exact(f64) -> ?i64`.

### Design (not built)

`BigInt` with a small-int fast path (Python- and Ruby-style integers);
Ryu formatting and Eisel-Lemire parsing (snprintf and strtod are used
instead); binary literals, digit separators and per-language literal
configuration.

## `std.io` and `std.os`

`std/io.lm`: `print(s)`, `println(s)`, `print_int(i64)`, `print_f64(f64)`
(`%.14g`), `print_bool(b)`, their stderr forms `eprint`, `eprintln`,
`eprint_int`, `eprint_f64`, and `read_file(path) -> ?str` / `free_file(s)`.

`std/os.lm`: `arg_count() -> u64`, `arg(i) -> ?str` (0 is the program),
`quit(code) -> never`.

### Design (not built): `std.io`, `std.os`, `std.ffi`

`std.ffi` does not exist (`extern "C" fn` and `export "C"` are in the
language, language.md §1.5). Not built: buffered or formatted output,
stdin, environment variables, clocks, `mmap`, dynamic loading.

## `std.lex`

`std/lex.lm`. There is no comptime, so there is no lexer generator: the
module is a `Cursor` toolkit, and a lexer is a short function that tries
the scanners in order:

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
  byte: `"?!"`, `"-"`). Presets: `lex.lua()`, `c_like()`, `js()`, `none()`.
  The cursor turns the identifier classes into 256-bit sets
  (`cursor.is_ident_start(b)`, `cursor.is_ident_char(b)`).
- `Cursor.new(src, file, syntax)`: `peek`, `peek_at`, `bump`, `advance`,
  `eat`, `eat_str`, `starts_with`, `eat_while(fn(u8) -> bool)`, `since`,
  `span_from`, `save`/`restore`, `at_end`, `pos` and `line` (fields),
  `error_at(start, msg)`, `error_here(msg)`. Character classes are ASCII
  (`lex.is_digit`, `is_alpha`, `is_ident_start`, `is_ident_char`,
  `is_hex_digit`, `is_space`).
- Scanners: `skip_trivia`, `ident -> ?str`, `at_number`/`number` (via
  std.num, with `3abc` reported as malformed), `quoted(&buf)` (escapes
  decoded into a `StrBuf`, including `\ddd`, `\xHH`, `\u{..}`, `\z`, line
  continuation; after a bad escape it still scans to the closing quote so
  the lexer can continue), `long_bracket_level`/`long_string`,
  `lex.push_utf8(buf, cp)`. There is no "word" scanner for
  whitespace-delimited languages: use `eat_while`.
- `OpTable<K>`: `new`, `add(text, kind)`, `match_at(s, pos)`; operator
  strings to token kinds, maximal munch through per-first-byte chains
  sorted longest first. Matching is `cursor.op(&ops) -> ?K` (or the older
  `ops.eat(&cursor)`).
- `Keywords<K>`: `new`, `add(word, kind)`, `get(word) -> ?K`; a
  `HashMap<str, K>` wrapper.
- Errors are `lex.Error { span, msg }` with a static message;
  `e.diag()` turns one into a `diag.Diag`.

A complete lexer for Mini's token set is about 50 lines
(`tests/run/lex_toy.lm`).

### Design (not built)

A comptime lexer generator: a token specification turned into a DFA at
compile time, with longest match, priority by declaration order, and
keyword tables via perfect hashing:

```loimi
const Tok = std.lex.define(.{
    .skip     = .{ "[ \t\r\n]+", "--[^\n]*" },
    .tokens   = .{ .Num = "[0-9]+(\\.[0-9]+)?([eE][+-]?[0-9]+)?",
                   .Name = "[A-Za-z_][A-Za-z0-9_]*",
                   .Str  = std.lex.quoted('"', escapes = .c) },
    .keywords = .{ .from = .Name, "if", "then", "else", "end" },
    .punct    = .{ "+", "-", "*", "/", "==", "~=", "<", "<=", "=", "(", ")" },
});
```

Also not built: UTF-8 aware character classes.

## `std.parse`

`std/parse.lm`:

- `Pratt<P, K, E, X>` over parser state `P`, a payload-free token-kind
  enum `K` (used as a table index via `k as u64`), expression result `E`
  and error type `X`. `Pratt.new(peek, advance, atom, unary, binary)`
  takes the hooks as function pointers; the optional `before_rhs` hook is
  a field (`pr.before_rhs = Some(f)`). The table is filled
  at run time (`infix_left(k, bp)`, `infix_right(k, bp)`,
  `infix(k, l, r)`, `prefix(k, bp)`), and `parse(p, min_bp)` (or
  `expr(p)`) calls the user's `peek`, `advance`, `atom`, `unary` and
  `binary` functions and `before_rhs` (where a single-pass
  compiler puts the left operand in a register or emits the jump of
  `and`/`or`). `infix_of(k)` and `prefix_of(k)` read the table.
- Operators that are not `lhs op rhs` get a per-kind `led` hook
  (`led(k, left, right, hook)`): when the operator binds, the driver
  consumes it and the hook parses the rest and returns the result. That
  covers mixfix (`c ? a : b`), postfix (`n!`) and right-hand sides with
  their own grammar (assignment with a target check, `x => body`); the
  hook parses operands with `pr.parse(p, pr.infix_of(k).right)`. MiniJS
  parses assignment, `?:` and arrows this way. Postfix chains such as
  calls and indexing can stay in `atom`. A hook that needs to recurse
  (parentheses) reaches the table through a pointer in `P`.
- `Parser<P: Hooks<K, E, X>, K, E, X>` is the same table and driver with
  the hooks as methods of the parser state: `P` declares
  `impl parse.Hooks<K, E, X> for P;` and defines `fn P.peek`, `advance`,
  `atom`, `unary` and `binary`, and optionally `before_rhs` (default:
  unchanged) and `led` (one method for all kinds registered with
  `pr.led(k, left, right)`, dispatching on the kind; the default traps).
  The calls are direct, so they inline.
- The hooks' types have no effects, so `parse` has none and runs from any
  function: hooks may allocate but not collect (collect between parses).

`Span` and `SourceMap` live in `std.diag`.

### Design (not built)

`TokenStream` (lookahead, `expect`, and error recovery to synchronization
tokens, `recover_to(.{ .Semi, .End })`); an operator table built at
compile time.

## `std.diag`

`std/diag.lm`:

- `Span { file, lo, hi }`, `diag.span(file, lo, hi)`, `a.to(b)`, `len`.
- `SourceMap` (borrowed names and text): `new`, `add(name, text) -> u32`,
  `name(id)`, `text(id)`, `snippet(span)`, `line_count(id)`,
  `line_col(id, pos) -> LineCol` (binary search in a line-start table
  built on first use), `line_text(id, line)`, `line_span(id, line)` (for
  errors that only know a line), `write_location(&buf, span)`,
  `render(&d, &buf)`, `eprint(&d)`, `free`.
- `Diag`: `Diag.new(severity, ?span, msg)`, `diag.error(span, msg)`,
  `diag.warning(span, msg)`, built up with `with_label`, `with_note`,
  `with_note_at`, `with_help`. `Severity` is `Error`, `Warning`, `Note`,
  `Help`. Rendered in the same format as the Loimi compiler's own errors:
  a snippet with carets, tabs preserved under the caret line, spans
  clipped to their first line, lines over 160 bytes cut to a window
  around the span.
- `Diagnostics`: a sink that copies the messages pushed into it into an
  arena (so they can be built in a temporary `StrBuf`): `push(d)`,
  `error(span, msg)`, `warning(span, msg)`, `len`, `at(i)`,
  `error_count`, `warning_count`, `has_errors`, `summary(&buf)`
  (`1 warning and 2 errors generated.`), `render(&sm, &buf)`,
  `eprint(&sm)`, `clear`, `free`.

### Design (not built)

Fix-it hints, ANSI color, multi-line spans, guest tracebacks.

## `std.bc`: bytecode construction

`std/bc.lm`, a library over the generated encoders (language.md §3.3).
Everything is generic over the vm's code word `W` through two functions
passed in: `size: fn(W) -> u64` (the generated `Vm.op_size`) and
`patch: fn(W, i64) -> ?W` (set a jump word's offset, `target - pc`, or
None if it does not fit: the generated `Vm.set_label`, or a few lines
over `Vm.opcode` and `bc.Field` for a custom encoding). The test
`tests/run/std_bc.lm` compiles a small language to a toy register vm
with all of it.

- `Code<W>`: `Code.new(Vm.op_size, Vm.set_label)`; `op(w)` emits an
  instruction and zeroes its inline-cache words, `word(w)` a raw word;
  `pc`, `len`, `at`, `set`, `words`, `copy_into`, `clear`, `free`. Source
  positions: `set_loc(loc)` (returns the previous one) tags the following
  instructions; positions are kept as runs, read with `loc_at(pc)`,
  expanded per word with `locs_into(out)`, or encoded with
  `encode_locs(&bytes)` (delta-encoded LEB128 runs, about 2 bytes per
  line change) and read back with `bc.line_at(bytes, pc)` on error paths.
- Labels: `label()`, `bind(l)`, `here()` (a placed loop head),
  `jump(w, l)`, `fixup(at, l)` (for a jump word that is not the first
  word emitted), `label_pc`. Jump lists for Lua-style compilers that pass
  pending exits around: plain `mem.Vec<u64>` of jump pcs with
  `patch_list(&l, target)` and `patch_here(&l)`; `patch(at, target)` and
  `jump_to(w, target)` for single jumps.
- Errors are sticky: an offset that does not fit records `JumpRange` with
  the pc and its source position, `check() -> ?Error` returns the first
  error (also `Unbound` for a label with pending jumps that was never
  placed), and `Error.message()` describes it. `fail(kind, pc)` records
  one. Compilers that stop at the first error can still check `patch`'s
  bool result.
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
  `len`, `items`, `clear`, `free`. For a nanboxed `Value`, `fn V.hash`
  over the bits and `fn V.eq(a, b) { a == b }` keep `0.0`, `-0.0` and
  `0` apart. There is no call that hands over the items and frees only
  the index.
- `Regs`: `Regs.new(limit)`; `alloc(n)` (past the limit it sets `failed()`
  and still returns, so the error can be reported later) or
  `try_alloc(n) -> ?u32`; `free(r)` (top only, locals ignored, false if
  out of order), `free2(a, b)`, `is_temp`, `mark`/`reset` (save and
  restore the free register), `end_stmt`, `activate(n)` (registers
  become locals), `enter`/`leave` (blocks), `frame_size(min)`.

Nothing in `std.bc` collects, so GC references in a `Pool` are not
roots: compile between collections, as Mini and Scheme do.

### Design (not built)

The design had a generated `Builder` per vm that this library would plug
into. Not built:

- emitters as `b.ADD(r1, r2, r3)` methods (the generated `Vm.ADD(..)`
  encoders plus `code.op` cover it);
- branch relaxation (a too-long jump is an error, reported by `check`);
- an exception table (ranges to handler pc) for `on raise`;
- `Proto` assembly that calls the verifier and returns `Verified<V>` (each
  interpreter's `Proto` and verify context differ; the compiler calls
  `Vm.verify` and `Vm.optimize` itself);
- serialization of protos (bytecode cache files).

## `std.gc`

`std/gc.lm`; details and measurements in
[`stdlib/gc.md`](stdlib/gc.md#as-implemented). The language side
(`object`, `Gc<T>`, `new`, tracing, effects, write barriers) is in
`language.md` §5 and §8.

- `Heap.new()`, `heap.free()`. Allocation is `new(&heap, Obj { .. })`
  (or with a trailing-array length); `gc.alloc` is the low-level call
  under it.
- Collection happens only where you call it: `gc.collect(&heap)` or
  `gc.maybe_collect(&heap)` (collects if `heap.allocated >
  heap.threshold`, or in stress mode), both `uses gc`. Allocation never
  collects.
- Roots: traced locals are rooted by the compiler at `uses gc` calls.
  Everything else (globals, VM stacks, state registers, host tables) is
  marked by a root scanner, `heap.add_scanner(data, scan)`, where
  `scan: fn(*Heap, *u8)` calls `gc_mark(h, v)` on each reference.
  Scanners have no effects: they cannot collect.
- `heap.add_sweeper(data, sweep)`: runs after marking; with
  `gc.is_marked(p)` it drops references to dead objects (weak tables).
  `heap.is_minor()` tells it only young objects can have died.
- Tuning: `set_growth(percent, min_heap)` (default 100% and 1 MiB),
  `set_max_budget(bytes)`, `set_generational(on)`, `set_nursery(bytes)`,
  `set_stress(on)` (collect at every `maybe_collect`), `set_verify(on)`
  (check write barriers at each collection). `LOIMI_GC_STATS=1` prints a
  line per collection.
- Weak references: `heap.new_weak(x) -> Gc<gc.Weak<T>>`, `w.get() ->
  ?Gc<T>`, `w.set(..)`. Ephemerons: `gc.Ephemerons.new(&heap) ->
  *Ephemerons<K, V>`, `get`, `set`, `remove`, `len`, `free`.
- Write barriers: the compiler emits `gc.barrier(obj)` after every store
  of a reference into an object. After a store through a raw pointer,
  call `gc.write_barrier(&heap, obj)`; `gc.needs_barrier(obj)` and
  `gc.remember(obj)` log several objects at once.

Every object has a 16-byte header (slot size, type id, mark bits, and
the trailing-array length, which compiled code reads at object - 8), so
a two-word object such as a Lisp pair takes a 32-byte slot.

**Growth policy.** After a collection, `percent`% of the live bytes (at
least `min_heap`) may be allocated before the next one. The default,
100% and 1 MiB, collects again after allocating as much as survived (the
heap peaks at about twice the live data). MiniJS uses 300% with an 8 MiB
minimum (as fast as 100% / 64 MiB, a quarter of the memory on small
heaps; see `examples/minijs/README.md`). The default stays because no
setting was better for every interpreter (best of 3-7, wall clock,
loaded machine): Mini at 200% runs binary_trees 17% faster (1.21 to
1.01 s) and the rest within noise; Scheme at 200% runs nboyer, graphs
and mperm 4-10% faster but string 27% slower (41 to 52 ms) and primes
and array1 3-4% slower; an 8 MiB minimum makes Scheme's small
allocation-heavy programs (deriv, divrec, cpstak) 6-10% slower, as the
heap leaves the CPU caches. `set_max_budget(bytes)` caps the budget (0,
the default, is no cap), so that a large growth factor bounds the heap
at the live data plus `bytes` rather than at a multiple of it once the
live data is large. Scheme uses 400%, a 2 MiB minimum and a 256 MiB cap
(see `bench/scheme/README.md`).

### Design (not built)

Full design: [`stdlib/gc.md`](stdlib/gc.md). Not built:

- heap policies as a compile-time parameter (`#![heap = ...]`) and the
  `Incremental` policy (tri-color, snapshot-at-the-beginning barrier);
- lazy sweeping, card marking, aging before promotion, a moving collector;
- roots generated from `vm` state kinds and frame structs (write a root
  scanner), `Root<T>`, `GcVec<T>`;
- finalizers, poisoning, heap snapshots;
- inline caches in bytecode as weak roots (caches cannot hold GC
  references; language.md §7.1).

## `std.obj`: objects and shapes

`std/obj.lm`; the API is described in
[`stdlib/obj.md`](stdlib/obj.md#as-implemented). In short: shapes with
weak transitions and ids that stay stable across collections (an id
names a transition path), inline and overflow slots, dictionary mode,
per-property attribute bits, prototype chains with validity cells (plain
ids, so a prototype hit is a few plain words), the inline-cache state
machine (`obj.Site`, `read_miss`/`write_miss`: refill, polymorphic,
megamorphic, cached property adds) and allocation templates. Keys are
u64 symbol ids. Use the ready-made generic `obj.Obj<V>` / `obj.Slots<V>`,
or declare your own object type (fields `shape`, `dict`, `overflow`,
optionally `proto`, and trailing `inline: [Value]`) and slot-array type;
either way, instantiate `obj.Model<O, S, V>` with a function that
allocates slot arrays. An inline-cache hit is two loads, a compare and
the slot load (`bench/obj`).

### Design (not built)

Full design: [`stdlib/obj.md`](stdlib/obj.md). Not built: a compile-time
`Model` configuration (attributes, prototypes, ordering, element storage
and thresholds chosen per guest language), elements (integer-indexed
storage), property cells for globals, `SetError`, and lookup tables shared
along a transition chain.

## `std.rt`: runtime support

`std/rt.lm` has `rt.protect(f: fn(*u8), data: *u8) -> bool` and
`rt.throw() -> never`, a setjmp/longjmp pair (the runtime half is
generated by `src/rtgen.rs`). They let a library return an error from
deep inside code that has no error path, such as Mini's recursive-descent
parser (`compile.try_compile`), instead of exiting the host process.
`throw` goes to the innermost `protect` on the thread (it panics if there
is none) and restores the shadow-stack top. The frames in between are
abandoned: their `defer`s do not run and what they `mem.alloc`'d leaks.
`f` has a function type without effects, so it cannot collect, and no GC
roots are lost.

Panics have the same machinery: `rt.catch_panic(f, data) -> bool` runs
`f(data)` and returns false if it panicked, and `rt.panic_message() -> str`
is the message of the last panic caught on the thread
(`"lib.lm:12:5: index out of bounds"`). `rt.panic(msg)` panics with your
own message (a catch keeps a copy of its first 512 bytes, so it may live
in a buffer the unwinding abandons). A `throw` passes through a
`catch_panic` to the enclosing `protect`, and a panic
passes through a `protect`. On an `export "C"` function the attribute
`#[catch_panic(v)]` does the same at the C boundary; the guarantees are
in `language.md` §10.1.

A VM stack is something you allocate yourself (`mem.alloc<Value>(n)`)
and keep in state registers; frames are your own structs; the GC sees
the stack through a root scanner. The example interpreters
(`examples/mini`, `examples/stack`, the tutorial's step 7 onwards) show
the pattern.

### Design (not built)

- `ValueStack<T>`: the VM stack. It would reserve virtual address space up
  front and commit pages on demand, so it never moves, and `StackTop`
  pointers stay valid. Overflow is a single compare against the limit,
  done once per call.
- `FrameStack<F>`: guest call frames, with entry markers for reentrant calls.
- `Upvalues`: open-upvalue list and closing (Lua-style), for closure capture by
  reference with stack-allocated locals.
- `Fiber`: stackful coroutines (separate host stack), for guest coroutines
  that must suspend across host frames.
- A C-stack depth guard for `reenter` (the compiler's `Vm.depth` counter
  is built; checking it is the interpreter's job).

## `std.ast`

**Not built.** Design: arena-allocated trees with `Id<T>` links, a
generic visitor via comptime reflection, and scope and resolver helpers:
`Scopes<Symbol, Binding>` with local, upvalue, and global classification,
which is the core of Lua-, JS-, and Python-style closure compilation.

## `std.prof`

`std/prof.lm`, for timing and counting inside an interpreter or its
benchmark driver. Nothing in it runs unless called, and the vm-stats
access is compiled in only when the program uses it.

- **Clock**: `prof.now_ns() -> u64` (`CLOCK_MONOTONIC`), and a stopwatch
  `prof.Timer.start()` with `t.ns()`, `t.ms() -> f64`, `t.restart()`.
  `prof.fmt_ns(&buf, ns)` appends "812 ns", "3.25 us", "41.7 ms".
- **Benchmarks**: `var b = prof.Bench.new(warmup, runs)`, then
  `b.add(name, f, data)` per variant (`f: fn(*u8) uses reenter`, so a
  variant may run an interpreter that collects; `run` then needs the
  effect too), and `b.run()`: `warmup` untimed rounds, then `runs`
  rounds that each time every variant once, starting from a different
  one each round, so drift in the machine's speed hits all variants
  alike. It prints name, minimum, median and minimum relative to the
  first variant on stderr (`b.report(&buf)` writes the table);
  `b.min_ns(i)` and `b.median_ns(i)` give the numbers. `b.free()`.
- **Counters**: `prof.Counters.new()`, `c.add(name) -> id`, `c.inc(id)`,
  `c.bump(id, n)` (inlined), `c.get(id)`, `c.report()` (stderr),
  `c.free()`: for events `--vm-stats` does not see (cache misses,
  slow-path entries).
- **Vm stats**: in a `loimi build --vm-stats` build,
  `prof.op_counts("Vm") -> ?prof.OpCounts` gives the vm's counters
  (None without the flag or for an unknown vm; `prof.vm_stats_enabled()`).
  `c.len()` ops in declaration order (supers included), `c.name(i)`,
  `c.count(i)`, `c.count_of("ADD")`, `c.total()`, `c.reset()` (op counts,
  pairs and sequences: measure one phase, or skip a warmup; the report
  at exit covers what ran since) and `c.report()` (the exit report, now).

### Design (not built)

A sampling profiler that reads the synced `pc` from a timer signal
(made accurate by an instrumentation variant that syncs on backward
jumps and calls) and maps it through the line table to guest source
lines, with folded stacks for flame graphs; `suggest-supers` as a
library over the pair counts; confidence intervals in `Bench`.

## `std.test`

`std/test.lm` and the `loimi test` command (language.md §1.2).

```loimi
import std.test;

#[track_caller]
fn squares(x: i64, want: i64) { test.eq(x * x, want); }

#[test]
fn arithmetic() {
    test.eq(2 + 2, 4);
    squares(3, 10);
}

#[test]
fn runs_a_program() uses gc { .. }
```

```text
$ loimi test calc_test.lm            # or: .. arith  (names containing "arith")
test arithmetic ... FAILED
    calc_test.lm:9:5: test.eq failed
        got:  9
        want: 10
test runs_a_program ... ok

failures:
    arithmetic

test result: FAILED. 1 passed; 1 failed; 0 filtered out
```

- **Running.** `loimi test FILE.lm` replaces FILE.lm's `main` and runs
  each `#[test]` function of FILE.lm in source order, in FILE.lm's
  directory. Arguments: filter words (a test runs if its name contains
  one), `--list`, `--update`, and the build options (`--release`,
  `--dispatch=`, `--sanitize=address`, `-W error`, ..). A test fails on
  any panic: an assertion, a bounds check, `unwrap` of None; the frames
  it abandons leak (`language.md` §10.1), and the rest of the run goes
  on. A crash that is not a panic (a segfault in unchecked code) ends
  the run. The generated `main` is plain Loimi over `test.Runner`
  (`Runner.new()` reads the arguments, `r.run(name, f)`,
  `r.finish() -> i32`), which a hand-written `main` can use as well.
- **Assertions** fail the test with the location of their caller
  (`#[track_caller]`; a helper of your own marked `#[track_caller]`
  passes the test's line through): `test.assert(c)`,
  `test.assert_msg(c, msg)`, `test.eq(got, want)` and `test.ne(a, b)`
  (types with `fn T.eq` and `impl test.Eq for T;`: `str`, `bool`, `f64`
  and the integers are built in, and `text.Fmt` to print them),
  `test.some(opt) -> T`, `test.none(opt)`, `test.ok(result) -> T`,
  `test.err(result) -> E`, `test.has(text, part)`,
  `test.panics(f, data) -> str` (the panic's message; fails if `f`
  returns), `test.fail(msg)`. `test.caller()` is the location inside a
  `#[track_caller]` function.
- **Output and golden files.** `test.capture_start()` sends stdout and
  stderr (descriptors 1 and 2) to a temporary file until
  `test.capture_end(&buf)` appends what was written; a test that fails
  while capturing shows the captured output. `test.golden(text, path)`
  compares with the file at `path` and names the first differing line;
  `loimi test --update` (or `LOIMI_TEST_UPDATE=1`) writes the file
  instead.

Conformance tests that run a built interpreter on script files remain
shell scripts (`examples/*/check.sh`); bytecode fuzzers are ordinary
Loimi programs (tutorial chapter 9, `tests/fuzz/`).

### Design (not built)

- A golden-file runner over `tests/**/*.<ext>` scripts with `-- expect:`
  comments, `--update` and parallel runs (`check.sh` does this per
  example today).
- A mode matrix: each test under every dispatch strategy, value repr and
  GC policy asked for (`--matrix dispatch=*,gc=MarkSweep,stress`).
- Differential testing against a reference interpreter binary.
- Tests collected from imported modules, and tests in parallel or in
  separate processes (so that a crash fails one test, not the run).
