# Lox in Loimi: a usability study and a comparison with clox

This directory is a bytecode VM for Lox, the language of Bob Nystrom's
*Crafting Interpreters*, written in Loimi by a first-time user (an AI
agent) from Loimi's README, `docs/`, `std/`, the tutorial and its
examples. It was compared with the book's C implementation, clox, as a
black box: clox's tests and benchmarks were used, its C source was not
read (clean room). This file is the report.

```sh
bench/lox/install.sh                                   # fetch + build clox, tests, benchmarks
loimi build examples/lox/main.lm --release -o lox
bench/lox/runtests.py ./lox                            # the book's test suite
bench/lox/compare.py -n 5 lox=./lox                    # the book's benchmarks vs clox
loimi test examples/lox/lox_test.lm                    # unit tests (std.test)
./lox FILE.lox [--disasm] [--disasm-after] [--gc-stress]
```

## Follow-up (what has been done since)

This report is kept as written. Since then:

| # | Status |
|---|---|
| 1 | Fixed: `poly_get_at`, `mega_get_at` and `read_miss_at` return the hit's depth (0 own, n inherited, `obj.ABSENT`); it was already in the cache entry, so a hit costs a register (MiniJS unchanged). Lox binds when the depth is nonzero; the `Method` variant is gone (`stdlib/obj.md`, "Where a value came from"). |
| 2 | Fixed: `gc.WeakInterner.key(s)` pins an interned string and returns its address, a key for good; `stdlib/obj.md` has a "Keys for guest strings" section (pinned strings, or `text.Interner` ids as MiniJS does). Lox pins every property-name constant. |
| 3 | Fixed: the hit helpers (`poly_get`, `mega_get`, their `_at` forms, `proto_load`, `load_ref`) are `#[inline]` (the attribute existed; the report missed it). `trees` 0.58x -> 0.55x of clox; MiniJS within noise. |
| 4 | Fixed: a nanbox holds up to 56 `Gc` variants; when the seven 48-bit blocks run out, the last-declared `Gc` variants share one, told apart by the pointer's low 3 bits (the shared layout, lowering.md section 2; `block 45` pins it). Lox is unchanged: with `Method` gone it needs six pointer variants, and `Native(u32)` (an index) costs nothing a `Gc` variant would save. |
| 5 | Fixed: `gc.WeakInterner<X>` over the interpreter's string object (or the ready-made `gc.Str`): `intern`, `find`, `key`, a sweeper drops dead strings. Lox's 60 hand-written lines are gone. |
| 6 | Fixed (opt-in): lint `hot_gc`, `-W hot_gc`, warns about `uses gc` calls on an op's fast path outside `cold` (calls inside plain vm functions are not seen). |
| 7 | Unchanged (by design; tutorial chapter 9). |
| 8 | Fixed: branch relaxation in std.bc (`set_relax(Vm.label, invert, Vm.JMP(0))`, `relax()`): a JF/JT that does not fit becomes the opposite jump over a JMP. Lox uses it for forward jumps and checks clox's loop limit itself (a loop spans at most 32767 words), reporting `Error at '}': Loop body too large.`: 246 of 246 tests pass. |
| 9 | Fixed: `Cursor.skip_space()`, an infallible `skip_trivia` for syntaxes without block comments. Numerals and strings are still hand-scanned. |
| 10 | Unchanged (not Loimi's). |

Lox is now in the golden tests (`tests/golden.rs`): `lox_test.lm`, and
`examples/lox/tests/*.lox` (own programs in the official format, with
`// expect-exit:` and `// expect-stderr:`) run normally and with
`--gc-stress`. Benchmarks after these changes (best of 5, interleaved, `--align-functions=64`):
geomean 0.45x of clox's time, as before; `trees` 0.57x -> 0.55x (inlined
polymorphic hits), `fib` 0.43x -> 0.40x (code layout: nothing it runs changed), `instantiation` 0.69x -> 0.71x
(identical handler code and GC work; not explained), the rest within 1%.
All 246 tests pass, also with `--gc-stress` and under AddressSanitizer.

## Results

**Test suite**: 245 of 246 tests pass (all of `test/` except the
early-chapter directories `scanning/` and `expressions/`, which the
book's own runner skips for clox; clox itself passes 246). The same 245
pass in all three dispatch modes (`tailcall`, `goto`, `switch`), in a
debug build (overflow checks), under AddressSanitizer, and with
`--gc-stress` (a collection at every safepoint) in every one of those
builds. The failure is `limit/loop_too_large.lox`, a test of clox's
encoding limit (a loop body over 65535 bytes of clox bytecode): this VM
has its own limit (a conditional jump spans at most 32767 words) and
reports `[line 2] Error: jump too long` instead of clox's
`Error at '}': Loop body too large.`. The other clox-specific limits
(256 constants per chunk counted as clox counts them, 256 locals, 256
upvalues, 255 parameters and arguments, stack overflow) are emulated and
pass.

**Benchmarks**: this VM takes 0.45× clox's time by geometric mean, and
less than clox on every benchmark (0.26× to 0.69×).

| benchmark | clox (s) | Loimi Lox (s) | Loimi / clox |
|---|---:|---:|---:|
| binary_trees | 2.332 | 0.606 | 0.26 |
| equality | 3.110 | 0.952 | 0.31 |
| fib | 0.954 | 0.388 | 0.41 |
| instantiation | 0.520 | 0.360 | 0.69 |
| invocation | 0.277 | 0.141 | 0.51 |
| method_call | 0.175 | 0.081 | 0.46 |
| properties | 0.415 | 0.187 | 0.45 |
| trees | 2.350 | 1.478 | 0.63 |
| zoo | 0.308 | 0.155 | 0.50 |
| zoo_batch (s per 1000 batches) | 1.923 | 0.922 | 0.48 |
| **geomean** | | | **0.45** |

Best of 5 runs, interleaved (each round starts with a different
binary), wall-clock seconds for the whole process, on a shared 8-core
x86-64 machine. clox was built by its own makefile (`make clox`: `-O3
-flto`, `NAN_BOXING` on, every `DEBUG_*` off). `zoo_batch` runs for a
fixed 10 s of `clock()` and counts batches; its row is seconds per 1000
batches. `string_equality` is left out: it does not compile under clox
(`Too many constants in one chunk.`, since every global name use is a
constant in clox), and this VM, which emulates clox's constant limit,
rejects it the same way. Program outputs (other than timings) match
clox's on every benchmark.

Nothing in the comparison relies on optimizations clox could not do
too: there is no constant folding, and expression statements whose
value is unused (`a1 == a2;`, all of `equality.lox`) are compiled and
executed. The differences are the usual ones of a register VM with
inline caches against a stack VM with hash-table lookups: no
push/pop traffic, compare-and-branch ops, constant operands, std.obj's
shapes and inline caches for fields and methods (clox looks up a hash
table for every field and method access), global variables behind a
slot cache, and std.gc's size-class allocator.

**Size**: 2,132 lines of Loimi without comments and blank lines (2,563
with), plus 98 lines of unit tests: `vm.lm` 636 (the vm, its slow
paths, root scanning), `compile.lm` 586 (code generation), `parser.lm`
576 (parser, name resolution and every compile error), `value.lm` 139
(values, objects, the string table), `driver.lm` 98, `lexer.lm` 82,
`main.lm` 15. Harness: `bench/lox/runtests.py` 143 lines (a port of the
book's Dart test runner), `compare.py` 90, `install.sh` 29.

**Time per phase** (wall clock, from file and commit timestamps):

| phase | time |
|---|---|
| fetch and build clox, write the test runner (`bench/lox`) | 2 min |
| read README, tutorial, stdlib.md, std.obj, language.md (parts) | 4 min |
| write lexer, parser/resolver, values, vm, compiler, main | 8 min |
| first build to 245/246 tests (the first build compiled without errors) | 2 min |
| measure, optimize (K forms for `==`, RETNIL, field-count hints, supers) | 5 min |
| driver module, unit tests with std.test, ASan and dispatch-mode runs | 3 min |
| benchmarks and this report | 10 min |

## Design in brief

- **Values**: `value Value repr nanbox` with `Num(f64) unboxed`, `Nil`,
  `Bool`, and seven pointer variants (`Str`, `Fun`, `Class`, `Inst`,
  `Bound`, `Method`, `CellV`); natives are `Native(u32)`, an index.
- **Registers**: a function's local slot *i* (numbered as clox numbers
  its stack slots) is register *i*; slot 0 is the callee, or `this`.
  A call's window starts at the callee's register, so arguments are
  never copied and the result comes back in the callee's slot.
- **Closures**: captured variables are known before code generation
  (the parser resolves names and marks captured locals, so the
  compiler works from a resolved tree). A captured local lives in a heap
  cell from its declaration on (`NEWCELL`); closures copy cell
  references. This gives the same semantics as clox's open/closed
  upvalues (a fresh variable per block execution, the `for` loop
  variable shared by the iterations) with no open-upvalue list.
- **Objects**: instances are std.obj objects whose prototype is the
  class's method table (itself an std.obj object; inherited methods are
  copied in). Property reads, writes and method calls (`INVOKE`, which
  does not allocate a bound method) use std.obj's inline-cache state
  machine (monomorphic, polymorphic, megamorphic, cached adds). Methods
  are stored as `Method(closure)` so that a read can tell a method
  (bound to the receiver) from a field holding a function. Instances
  get as many inline slots as the class's methods assign distinct
  `this.x` fields (a static count, grown at run time if exceeded).
- **Strings** are all interned in a weak table (a std.gc sweeper drops
  dead ones), so equality is a word compare. Property names are keyed by
  the address of their interned string.
- **Errors**: `on raise` prints clox's traceback format; the parser
  reproduces clox's messages and panic-mode recovery.

## Friction, most important first

### 1. std.obj's caches can't tell an own-property hit from a prototype hit (design, ~10 min)

*Tried*: instances with the class's method table as their std.obj
prototype, so that `obj.name` finds fields, then methods, through one
cache. *Problem*: Lox binds a method found through the class to the
receiver, but a function stored in a field is returned as it is.
`read_miss` tells the state (`OwnIn` vs `Proto`), but the polymorphic
and megamorphic hits (`poly_get`, `mega_get`) return only the value.
*Did*: a `Method(Gc<Closure>)` value variant that only ever lives in
method tables, as a tag. It costs a nanbox variant (see 4). *Expected*:
`poly_get`/`mega_get` variants that return the hit's depth (0 = own),
or a documented pattern for "methods vs fields" (Python, Ruby and JS
all distinguish them in some way). *Suggested fix*: add
`poly_get_hit`/`mega_get_hit` returning `(V, depth)`, or say in
`stdlib/obj.md` how an interpreter should do this.

### 2. Property keys in bytecode: no path from a name in an 8-bit operand to a std.obj key (design, ~5 min)

std.obj keys are `u64` "symbol ids (`text.Interner`) or other
integers". A `GETPROP dst, obj, name` in a 32-bit `ABC` word has 8 bits
for the name, so the name has to be a constant index, and constants are
`Value`s. *Did*: the key is the address of the interned name string,
which is safe here only because prototypes, and so their name
constants, are never freed; if a name string could die, a later string
at the same address would silently alias the property in every shape.
Nothing in the docs warns about this; `stdlib/obj.md` says keys "are not
traced" and must "stay meaningful while shapes use them", which is the
right rule but easy to miss. *Suggested fix*: a worked example (MiniJS
does something here, but it is off limits to this study) of keys for an
interpreter whose names are guest strings, or a `Sym` operand kind
pattern in the tutorial (a per-proto symbol table read through a verify
context).

### 3. `poly_get` / `mega_get` are not inlined into handlers (performance, ~5 min)

`objdump --disassemble=vm.Lox.op.GETPROP_POLY` shows a stack frame
(`push %rbp; sub $0x10,%rsp`) and a `call` to
`std.obj.Model<..>.poly_get` on the hit path. `trees.lox`, whose sites
are polymorphic (leaf and inner nodes have different shapes), is the
benchmark where this VM gains least but one (0.63× clox's time; instantiation, bound by allocation and the `init` call, is 0.69×), against
0.3-0.5× elsewhere. The handler cannot ask for the call to be
inlined; the attribute would have to be on the std function.
*Suggested fix*: `#[inline]` on `poly_get`, `mega_get`, `proto_load`,
`load_ref`, or a call-site `inline` form.

### 4. Seven pointer variants in a nanbox (minor, ~2 min)

Lox needs eight reference types here (string, closure, class,
instance, bound method, method, cell, native); the nanbox holds seven.
The docs say so clearly and the error is good:
`error: no room for variant `P8` in the nanbox's NaN space (seven 48-bit
blocks for pointers and 48-bit integers, one 47-bit block)`. Natives
became `Native(u32)`, an index. *Suggested fix*: a pointer in the 47-bit
block would fit x86-64 user-space addresses (47 bits), if the target is
known.

### 5. Interned strings: a weak intern table is hand-written (~5 min, 60 lines)

clox-speed string equality needs every string interned, with dead
strings dropped. std.gc has the pieces (`add_sweeper`, `is_marked`),
std.text's `Interner` is for host strings, and `gc.Weak` /
`Ephemerons` are per-object. The table here is open addressing over
raw addresses with `unsafe { a as Gc<LStr> }` to get the reference back.
*Suggested fix*: a `gc.WeakSet<T>`/interner in std (stdlib.md already
lists `Gc<Str>` with a cached hash as design).

### 6. Effectful helpers on a handler's hot path are silent (minor)

`GETPROP_POLY` first called `bind(ctx, o, v)`, a `uses gc` function
(it may allocate a bound method), on every hit. That is legal and
correct, and the compiler synced and rooted around it on the hot path
without a word. Moving the call into `if let Method(_) = v { cold { .. } }`
is the right shape. (Here it made no measurable difference, because the
`poly_get` call dominates; see 3.) *Suggested fix*: an opt-in lint for
effectful calls outside `cold` in handlers.

### 7. Metadata the verifier cannot see (minor)

`CLOSURE` reads its capture list (`(is_local, index)` pairs) from the
prototype, not the bytecode, so the verifier cannot check it; the
handler checks each index at run time (cold raise). The tutorial
(chapter 9) says exactly this; it is noted only because Lox's closures
make it come up immediately.

### 8. No branch relaxation (minor; the one failing test)

`loop_too_large.lox` fails with `bc`'s `jump too long` because `JF` has
a signed 16-bit offset. `std.bc` documents that branch relaxation is
not built. A `JF` with an `sJ`-style wider form, or relaxation in
`bc.Code`, would remove the limit; matching clox's message would need
mimicking clox's byte encoding, which is not worth it.

### 9. std.lex fits Lox only partly (minor, ~2 min)

`Cursor` with `lex.none()` plus `line_comment = "//"`, `ident()`,
`OpTable` and `Keywords` did most of the lexer. Numerals and strings
are hand-scanned: Lox's `1.` is a number then a dot and its strings
have no escapes and may span lines, which `number()`/`quoted()` do not
express. `skip_trivia()` returns a `Result` even when no configured
comment can be unterminated (`let _ = c.skip_trivia();`).

### 10. Pratt's error model vs Lox's panic mode (minor, not Loimi's fault)

`std.parse` hooks return `Result`, so the first syntax error unwinds
the expression. clox reports and keeps parsing (with later messages
suppressed) until it synchronizes at a statement boundary. They differ
only where clox's continued parsing changes where recovery starts:
`fun f() 123;` must still report `Expect '}' after block.` at the end,
so `consume` failures in a function header are reported and parsing
goes on. Everything else worked with `Result<*Node, ()>` and a sticky
"panic" flag in the parser state.

### Not friction, but worth recording

- The environment's sandbox refused shell loops and `cd`, which cost
  more time than anything in Loimi; not a Loimi issue.
- Uncertain, not tested: whether a zero-filled trailing `[Gc<Cell>]`
  may be traced before it is filled. `CLOSURE` fills it before anything
  can collect, so the question did not matter.

## What I liked

- **It compiled the first time.** About 2,000 lines written from the
  docs built without a single compile error and passed 241 of 246 tests
  on the first run; four of the five failures were details of Lox's
  parser (a parenthesized name or `this` is not an assignment target;
  clox keeps parsing after a bad function header), fixed in minutes. That says a lot about how well the tutorial's code transfers.
- **The tutorial is the right template.** Step 9's vm (`then JMP`
  compare-and-branch, the `inline for` templates, the `GETGLOBAL`
  family, register windows, `on raise` with a line table) carried over
  almost unchanged, and the Lox-specific parts (closures, classes) were
  straightforward to add in the same style.
- **std.obj** gave fields and methods inline caches with polymorphic
  and megamorphic states in about 150 lines of handlers, following
  `tests/run/std_obj_sites.lm`.
- **GC integration is effortless**: effects caught nothing because
  nothing was wrong, and `--gc-stress` in every dispatch mode and ASan
  found no bug, on the first try. `uses gc` on the entry and the
  `sync` block are all it took.
- **Generated tools**: the disassembler (`--disasm`, `--disasm-after`
  to see quickening), `--vm-stats` with its super suggestions,
  `Lox.CW_PropIC` to compute return addresses past inline caches, the
  verifier.
- **`loimi test`** with `test.capture_start()` made in-process tests of
  whole programs (normal and GC stress) a dozen lines.
- **Performance came for free**: the first version, with no tuning at
  all, ran the benchmarks in about half of clox's time.
