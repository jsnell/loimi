# Implementation status

What exists today, and where the implementation differs from the design
in [`language.md`](language.md). Updated 2026-11-02.

## Built

| Area | Status |
|---|---|
| Compiler | Rust, no dependencies. Emits textual LLVM IR and drives `clang-19` (`loimi build`, `loimi run`, `--emit-llvm`, `--release`, `--dispatch=`, `--sanitize=address`, `--align-functions=N` to start every function on an N-byte boundary so that unrelated code changes do not move vm handlers, which makes A/B benchmarks steadier). Named warnings (`match_fallout`, `super_overlap`), silenced with `#[allow(name)]` on an item, vm item or match arm or `#![allow(name)]` per file; `-W error` / `--deny-warnings` makes them errors. |
| Libraries and embedding | `loimi build --lib` (static archive of `-fPIC` code) and `--shared` (shared object), with no `main`; `export "C"` functions are the API. `--header out.h` writes a C header for them (integers, floats, `bool`, pointers, opaque struct pointers, nullable `?*T`, C function pointers, `str` as `loimi_str { const char *ptr; size_t len; }`, payload-less enums as a typedef plus `E_Variant` constants; other types are rejected with the reason). No init function. Runtime thread-locals are initial-exec, local-dynamic in `--shared` builds (`--tls=` overrides). `std.rt.protect`/`throw` recover from errors deep in code without an error path. `examples/mini/embed.lm` makes Mini a library (`mini_new`, `mini_dostring`, `mini_error`, globals, C natives, a rooted value stack, `mini_call`); `examples/embed/host.c` uses it. C natives may call back into Mini (`mini_call`, `mini_dostring`), as nested runs. See `language.md` §10.1. |
| Core language | type aliases (`type Name = T;`, `pub type`, generic `type Map<V> = HashMap<str, V>;`; transparent), `inline for` (module level, in `vm` blocks, and as a statement in any block), structs (literals may be module-qualified: `diag.Span { .. }`), enums with payloads, `?T`, `Result` with `?`, tuples, arrays, slices, pointers and pointer arithmetic, methods (`fn Type.name`), modules and imports, `match` with exhaustiveness checking (through nested patterns and tuples) and guards (`pat if cond =>`; in vm code, a warning for an arm whose body is an `if` without `else` when a later arm could match), `let ... else`, `if let` chains, loops with `break` values, `for i in 0..n` ranges typed by the non-literal bound or an annotation (`for i: u8 in 0..10`), `defer`, consts, module-level `var`s (see below), casts (including integer to fieldless enum, checked in debug builds; `Kind.COUNT` gives the number of variants; `x as u64 < y` parses as a comparison), local inference across `if`/`match` branches (`if c { Ok(e) } else { Err(m) }` needs no annotation), checked/wrapping arithmetic, arbitrary-width integers `iN`/`uN` (1..=64), overflow and bounds checks (overflow traps in debug builds). |
| Globals | `var NAME: T = init;` at module level (with `pub`, readable and assignable from other modules as `mod.NAME` or after `import mod.{NAME}`). Plain LLVM globals; `#[thread_local]` makes one copy per thread (`thread_local(initialexec)`). The initializer must be a constant expression: literals, consts, `[x; N]`, struct/tuple/array literals, enum variants without payload, `None`/`Some(..)`/`Ok(..)`/`Err(..)` of constants, and functions (for hooks). Globals are not GC roots: a global holding `Gc`/`Value` must be marked by a root scanner (`heap.add_scanner`). |
| Generics | Monomorphized generic structs, enums, functions and methods; type arguments inferred from arguments and the expected type. Qualified variants of generic enums (`Lookup.Missing`, `Lookup.Own(i)`, also in patterns) take the instance from the expected type or the payload. Methods resolve per instance. Static traits (`trait Hash { fn hash(k: Self) -> u64; .. }`, `impl Hash for Sym;`) with default methods, type parameters (`trait Conv<T>`, bounds `P: Hooks<K, E>`), generic methods (`fn fold<U>(..)`), static functions (`T.zero()`) and generic impls (`impl<T: Show> Show for Vec<T>;`). Bounds (`HashMap<K: hash.Hash, V>`, `fn f<T: A + B>`; methods of a generic type inherit the type's) are checked at instantiation, with the error at the use site, and generic bodies are checked against them: a bounded parameter has only its bounds' methods (opaque in a second check of the body, once per combination of the unbounded parameters' arguments); unbounded parameters stay duck-typed. Static dispatch only. A generic function can be used as a function pointer where a function type is expected (`Pratt.new(peek, ...)` with `fn peek<E>(p: *P<E>) -> T`), or with explicit type arguments anywhere (`scan<Gc<Box>>`); passed to another generic function, its type arguments can come from the parameter types alone (`apply(double, 21)` with `fn apply<T, U>(f: fn(T) -> U, x: T)`). Methods can have their own type parameters after the method name, on plain receivers (`fn Cursor.op<K>(c: *Cursor, ops: *OpTable<K>) -> ?K`) and generic ones (`fn Box<T>.map<U>(b: *Box<T>, f: fn(T) -> U) -> Box<U>`); the receiver's arguments come from the receiver, the method's are inferred from the arguments and the expected type. Type arguments can also be given explicitly on calls: `mem.alloc<u8>(n)`, `id<i64>(x)`, `c.op<K>(&ops)`, `Cursor.cast<u8>(&c)` (a method's own parameters only; not on a generic type's static functions such as `Vec.new`). `f<T>(` is recognized when `<` directly follows the name and a balanced, type-shaped `<...>` is followed by `(`; otherwise `<` is a comparison. |
| `value` types | `nanbox`, `lowtag`, `boxed` representations; NaN canonicalization; tuple patterns merged into one test (xor/or/mask, umax). Integer payloads up to 48 bits (nanbox) or 61 bits (lowtag); more than 7 (nanbox) or 8 (lowtag) tagged variants when payloads are narrow, with tags sized to the payloads. Tag pinning (`= tag 0b001`) and `lowtag(bits = N)` are not implemented: lowtag uses 3-bit primary tags. |
| `vm` declarations | state, context, operand kinds, fixed-width formats, inline caches (fields of any integer, float, `bool` or pointer type, wider ones spanning several code words with unaligned access; no GC references), `then OP` (typed operands of the following instruction), `family` + `become`, `super` (any number of parts, including family members; `become` in a part re-targets the super), vm functions (including `-> never` ones with control words, and `c: cache Name` parameters that receive the calling op's cache), entries, `on raise`/`raise`, `cold`, `sync`, `reload` (run after each `reenter` call in vm code), nested entries (`entry call(..) uses reenter`: a run above an active one, for natives calling guest code; callers need the effect; a generated thread-local `Vm.depth` counts them), in all three dispatch modes. |
| Dispatch | `tailcall` (`preserve_none` + `musttail`, instruction word passed in a register), `goto` (one function, replicated `indirectbr`), `switch`. |
| Generated per vm | encoders (`Vm.OP(..)`), emitters (`Vm.emit.OP(..)`), opcode constants, `op_size`, `op_name`, the superinstruction peephole (`Vm.optimize`), deoptimization (`Vm.deopt`: every word back to its family's generic op, supers split, caches zeroed), the verifier (`Vm.verify`, with any number of `verify_context` parameters), label helpers (`Vm.set_label(w, off) -> ?W`, range-checked; `Vm.label(w) -> ?i64`), per-format field readers (`Vm.ABC_a(w)`) and per-op operand metadata (`Vm.operand_count(w)`, `Vm.operand_kind(w, i)`, `operand_bits`, `operand_signed`, `kind_name`, `kind_is_label`, `NUM_KINDS`), the disassembler (`Vm.disasm(code, &out)`, `Vm.disasm_insn(code, i, &out) -> size`, generic over the output buffer; operand kinds can declare `display "r{}"`). The items are named `Vm__X` in the vm's module: an op named like one of them (`label`, `verify`) is an error, and operand names starting with `__` are reserved. |
| Instrumentation | `variant NAME wraps all|[OPS] { before { .. } }`, switched with `use_variant`. `loimi build --vm-stats` counts executed ops and adjacent op pairs, and prints frequent pairs at exit as `super` declarations. |
| GC | `object` types (with trailing arrays), generic `object` types (`object Obj<V> { .., inline: [V] }`; one type id and trace function per instance), `Gc<T>` (also over a type parameter, checked per instance), `new(heap, ..)`, `gc_mark`; `std.gc` mark-sweep written in Loimi; compiler-generated tracing (including pointers inside values); shadow-stack frames for traced locals; `unsafe { p as Gc<T> }` from a pointer or address; effects (`uses sync/gc/reenter`), also on function types (`fn(*Heap) uses gc`), so calls through function pointers are checked and rooted. |
| Closures | Non-escaping `block fn(A) -> R uses E` parameters and `|a, b| expr` literals, capturing by reference; only as call arguments, never stored or returned (checked). Effects of the body are checked against the type. Lowered to a separate function with an environment of slot pointers in the enclosing frame (small `let`s copied in); code and environment passed as two arguments, so LLVM specializes callees for a closure literal; plain functions convert through a trampoline. Not allowed in vm code. |
| Standard library | `std.io`, `std.os`, `std.mem` (`Vec` with `sort_by`, `Arena`, allocation), `std.hash`, `std.map` (`HashMap`), `std.text` (`StrBuf`, `Interner`, number parsing/printing), `std.gc`, `std.rt` (`protect`/`throw` only), and the front-end and back-end modules below. |
| `std.num` | Exact numeral parsing with overflow reporting (`parse_int`, `parse_i64`/`parse_u64` in radix 2 to 36, `parse_u64_wrapping`, `parse_f64` via strtod with strict syntax, `parse_number` for literals, `decimal_len`), float formatting (`write_g` for `%.14g`, `write_shortest` for shortest round-trip, `shortest` digits and `write_js` for ECMAScript's Number::toString, `mark_float` for Lua's `3.0`, `write_hex`), `add/sub/mul_overflow`, `floor_div`/`floor_mod`/`trunc_div`/`trunc_rem` with the `MIN / -1` case defined, float `floor_div_f64`/`floor_mod_f64`, `to_i64_exact`. |
| `std.diag` | `Span`, `SourceMap` (borrowed files, lazily built line-start table, binary-searched `line_col`, `line_text`, `write_location`), `Diag` (severity, message, primary span with label, notes and help lines, built with `with_label`/`with_note`/`with_note_at`/`with_help`), `line_span` (for errors that only know a line), rendering with a snippet and carets in the compiler's format (lines over 160 bytes cut to a window around the span), and a `Diagnostics` sink that copies messages into an arena, counts errors and warnings, and prints a summary. |
| `std.lex` | A scanner toolkit rather than a generator: `Cursor` (peek, bump, line tracking, marks, spans, errors), `Syntax` presets (`lua()`, `c_like()`, `none()`) for line, block, nested and Lua long-bracket comments, escape and overflow rules; `skip_trivia`, `ident`, `number` (via std.num), `quoted` (escapes into a `StrBuf`, including `\ddd`, `\xHH`, `\u{..}`, `\z`, line continuation), `long_string`, `OpTable<K>` (maximal munch), `Keywords<K>`. Errors are `lex.Error` (span and message), convertible to a `diag.Diag`. A complete lexer for Mini's token set is about 50 lines (`tests/run/lex_toy.lm`). |
| `std.parse` | `Pratt<P, K, E, X>`: binding-power table indexed by token kind (`infix_left`, `infix_right`, `infix`, `prefix`) and a driver that calls the user's `peek`, `advance`, `atom`, `unary`, `binary` and optional `before_rhs` hooks through function pointers. `Parser<P: Hooks<K, E, X>, K, E, X>`: the same table and driver with the hooks as methods of the parser state (trait `Hooks`, with defaults for `before_rhs` and `led`). |
| `std.bc` | Bytecode construction over any vm's generated encoders: `Code<W>` (given `Vm.op_size` and a jump-patch function: `op` reserves inline-cache words, labels with fixups, jump lists, range-checked patching with sticky errors, a run-length position table with per-word expansion and a delta-encoded form, `last`/`truncate` for peepholes, jump threading), `Field` (encoded bit fields), `Pool<K: hash.Hash>` (deduplicating constant pool with a limit) and `Regs` (Lua-style locals/temporaries register allocator with maximum tracking). |
| `std.obj` | Hidden-class objects over a generic `obj.Obj<V>` or the interpreter's own object type: shapes in a transition tree with weak edges (cleared by a sweeper) and never-reused ids, inline and overflow slots, dictionary mode, prototype chains with validity cells, monomorphic cache fill helpers (shape id + SlotRef in two cache words; a hit is two loads, a compare and the slot load), `Poly`, a global `MegaCache`, allocation templates. No attributes, elements or globals cells. |
| Example | `examples/mini/`: a Lua-like interpreter (single-pass compiler on std.lex, std.parse, std.diag and std.bc; VM; GC'd strings and tables; `table.sort` with Lua comparators through re-entry), tested with GC stress, and embeddable in C (`embed.lm`), with natives that call back. `examples/scheme/`: an R7RS-small subset (reader on std.lex and std.num, code generator on std.bc). `examples/minijs/`: a JavaScript subset on std.lex, std.parse, std.bc, std.obj and std.num (shapes, mono/poly/mega inline caches, prototype validity cells), tested with GC stress. |
| Tests | `cargo test`: golden programs (every vm test in all three dispatch modes), expected compile errors, the Mini, Scheme and MiniJS test programs and conformance suites with and without GC stress, a few seconds of bytecode fuzzing (below), and C hosts built against `--lib` and `--shared` libraries and their generated headers (`tests/embed/`, and embedded Mini with two states, natives calling back, and GC stress). Re-entry: `tests/run/vm_reenter.lm` (all three modes, a stack that moves on every callback), Mini's sort scripts in all three modes. |

## Bytecode fuzzing (M5)

M5 asks for a bytecode fuzzer that finds no crashes in verified code. `tests/fuzz/run.sh` builds the fuzzers with `--sanitize=address`
and runs them for a time or seed budget, saving failing seeds:

- `tests/fuzz/fuzzvm.lm`: a test vm using every verifier feature
  (register, constant and context-indexed operands, labels of three
  widths, a call operand checked with `is_insn_boundary`, two verify
  contexts, inline caches, families with `become`/`redo`, supers with up to
  four parts and family-member parts, `then JMP`, `-> never` vm functions,
  cold blocks, variants), declared once per dispatch mode. Random programs
  (mostly well-formed, some mutated, some random words) are verified and,
  if they pass, run in all three modes with exactly-sized heap buffers and
  a step budget. Besides crashes it checks that the modes agree and that
  code still verifies after running (quickening) and after `Vm.optimize`.
- `tests/fuzz/minifuzz.lm`: random code against Mini's own `verify`
  clauses and `Mini.run`, on a stack exactly as large as the frame.

ASan works in all three dispatch modes. `--sanitize=address` turns off
use-after-return detection: its fake-stack fallback is a dynamic alloca,
which LLVM cannot combine with stack realignment in `preserve_none`
functions, so tail-call builds would not compile. `musttail` survives the
instrumentation.

**Run so far: 2 h 10 min** under ASan (2026-11-02), before and after the
n-ary supers / `then` changes, 20 minutes of it on the final verifier:
149M test-vm programs (55M verified and run in all three modes) and 11.6M
Mini programs (3.0M run), in debug and `--release` builds. `cargo test` runs a few seconds of both
without ASan.

Bugs found and fixed (regression tests in `tests/run/vm_verify_*.lm`):

- Verified code could run off its end: a last instruction using `next`
  read past the buffer, and empty code verified. The verifier now rejects
  both (see `language.md` §6).
- Before the n-ary super rewrite: a super's later words were checked only
  as the declared part's operands, so after the first part quickened they
  ran as unverified instructions; and `Vm.optimize` produced overlapping
  supers that failed verification.
- Found by review while fuzzing: `verify` clauses are pasted into the
  generated verifier, whose locals shadowed verify contexts of the same
  name (`verify_context w: u32` compared with the instruction word).

Holes in Mini's verify clauses (`examples/mini/vm.lm`), now fixed:
CALL's `nres` let the callee's return write past the stack; SETLIST's,
RETN's and CONCAT's counts (and CALL's argument count) read past the
frame; CONCAT with fewer than 2 values and GETINDEXI with index 0
underflowed; SELF wrote `R[a+1]` with only `a < nregs` checked. Each
count now has its own operand kind whose `verify` clause reads the base
register from the instruction word (`Mini.ABC_a(*pc) + it <= nregs`);
the kinds decode to the plain field, so handlers are unchanged.
`tests/fuzz/mini_holes.lm` checks that all nine reproducers are
rejected (run by `cargo test`).

## Differences from the design

- **Collection happens only at safepoints.** Allocation never collects;
  only functions with `uses gc` (such as `gc.collect`) do. The effect rules
  are as designed, but "may collect" is now explicit and rare. As a result,
  the only roots the compiler has to provide are traced locals that are in
  scope at those calls, registered in shadow-stack frames. Traced
  *temporaries* are not roots: bind a value to a local before a call that
  may collect.
- **`sync { ... }`** is a new vm item. It holds the code the compiler runs
  before an effectful call from a handler, for example publishing `fp` or
  the stack top for the GC's root scanner. The design's fixed `ctx.sync` area
  became user-defined code.
- **Cold blocks are outlined with `musttail`, not with `preserve_most`
  calls.** In tail-call mode, a `cold` block in an op handler that ends in
  control words moves to its own function with the handler's signature,
  and the handler reaches it with `musttail`. The hot path makes no call,
  so it needs no stack frame. The instruction word travels in its
  register, and the cold function decodes the operand fields it uses
  again; other handler locals it uses travel through thread-local slots
  (`initialexec`), written only on the cold path.
  `if c { cold { .. } }` followed by code ending in a control word gets that
  continuation copied into the cold block, so it can be outlined too. This
  includes superinstruction parts: an outlined `next` from a part other
  than the last dispatches the next part's word on its own. Other
  cold blocks, and all cold blocks in goto/switch mode, stay inline and are
  marked unlikely.
- **State kinds** (`Cursor`, `Window`, `StackTop`) are plain pointers.
- **`unsafe`** is required only for `p as Gc<T>`. Raw pointer
  dereference, indexing and arithmetic are allowed anywhere (the design
  draft required `unsafe` for them); see `language.md` §1.3 for why.
- **Operand fields** decode to `u64` / `i64` in handlers and verify
  clauses, whatever their width (the design left them at the field width).
- **`-> never` vm functions** are inlined into each call site, except
  that in tail-call mode, calls from `cold` blocks (and recursive calls)
  `musttail` to one shared copy that receives its arguments through
  thread-local slots. They cannot use `become` or the caller's operands;
  the op's inline cache can be passed as a `c: cache Name` parameter (also
  to plain vm functions), which a shared copy receives as an address.
- **Re-entry** (`language.md` §10): the design's `Mini.call` is not
  generated whole. The compiler provides the nested entry (an entry
  declared `uses reenter`, which callers must be allowed to make), the
  `reload` block after `reenter` calls in vm code, and `Vm.depth`; the
  interpreter writes where the new frame goes, the sentinel that makes
  the callee's return leave the run, the depth limit (Mini: 200, "C
  stack overflow"), and restoring its frame registers after the call
  (Mini's `vm.call_value`), because only it knows its frame layout.
  Derived pointers held across a `reenter` call (`let p = regs + a`)
  are not checked. Measured on Mini, a nested run per callback costs
  about 20 ns over a native comparison and makes a native `table.sort`
  with a Lua comparator 1.2x slower than the same sort written in Lua
  (`bench/reenter/`, `lowering.md` §4), so the Scheme and MiniJS
  preludes keep their higher-order builtins.
- **Effects** must be declared; they are not inferred for private functions.
  Handlers and plain vm functions are the exception: their effectful calls
  are synced in place (in a vm function, through the state pointers it
  sees), and callers of such a vm function root their traced locals there.
  Function types carry them (`fn(*Heap) uses gc`), so calls through function
  pointers are checked, synced and rooted like direct calls. There are no
  effect variables: a generic function cannot take "a callback with any
  effects" and inherit them.
- **Verifier**: opcodes, extents, operand `verify` clauses, superinstruction
  part words, and "cannot fall off the end" (an op may continue if its
  handler uses `next` or reads `pc`; the scan is over tokens, so it errs
  toward "may continue"). Stack-effect checking for stack VMs is not
  implemented, and there is no `Verified<V>` type: `Vm.run` takes a raw
  code pointer, so nothing ties the code it runs, or the run-time state
  (register window size, the closure's prototype), to what was verified.
- **Instrumentation**: switching tables happens from vm code (`use_variant`),
  not through an external `set_dispatch`.
- **Front-end stdlib** (`std.lex`, `std.parse`, `std.diag`, `std.num`) is
  smaller than the design: there is no comptime, so `std.lex` is a cursor
  toolkit instead of a DFA generator and the Pratt table is filled at run
  time; there are no `TokenStream` or error-recovery helpers, fix-it hints,
  ANSI color, `BigInt`, or Ryu/Eisel-Lemire (formatting and parsing use
  snprintf and strtod). See the notes in [`stdlib.md`](stdlib.md).
- **Back-end stdlib** (`std.bc`, `std.obj`): `std.bc` is a library that
  takes the generated `Vm.op_size` and a jump-patch function rather than
  a generated `Builder`; it has no branch relaxation, exception table,
  proto assembly or serialization. `std.obj` has no comptime `Model`:
  it offers a generic `Obj<V>` (or the interpreter declares its own
  object and slot-array types) and a generic `Model` over them; keys are u64 symbol ids; attributes,
  elements and global property cells are not built. Shape ids are never
  reused, so inline caches need no reset after a collection. See the
  "As implemented" notes in [`stdlib.md`](stdlib.md) and
  [`stdlib/obj.md`](stdlib/obj.md#as-implemented).
- **`inline for`** is token-level expansion: values are token sequences
  (so operators like `+` work), and `#var` makes an identifier from a
  string or identifier value. In statement position the copies are
  statements of the enclosing block, with no scope of their own (see
  `language.md` §1.4). Inside a `vm`, a loop variable named like a state
  variable is an error. Errors in expanded code point at the body or the
  element in the source, but do not say which copy they come from.
- **Traits and closures** (see [`design/closures-traits.md`](design/closures-traits.md)):
  no `dyn`, associated types or constants, supertraits, orphan rule or
  overlap check between impls; default bodies are checked when an
  instance uses them, not at the trait. Closures do not escape (no
  stored or returned closures), and cannot appear in vm code.
- **Not yet built**: escaping closures, a moving,
  incremental or generational GC, weak references, ephemerons, finalizers,
  `std.rt` (beyond `protect`/`throw`), `std.prof`/`test`, variable-length encodings, and atomic `become` for shared code.
- **Generated tooling not built** (described in `language.md` and
  `lowering.md`, marked there): a `Builder` type (emitters plus `std.bc`
  do its job), `Verified<V>`, stack effects, generated C-ABI entry points
  (entries are internal functions; embedders write `export "C"`
  wrappers), `loimi vm suggest-supers` (`--vm-stats` prints the
  suggestions itself), handler ordering from profiles, the
  `#![heap = ..]` policy parameter, and `Root<T>` handles.
