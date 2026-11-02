# Implementation status

What exists today, and where the implementation differs from the design
in [`language.md`](language.md). Updated 2026-11-02.

## Built

| Area | Status |
|---|---|
| Compiler | Rust, no dependencies. Emits textual LLVM IR and drives `clang-19` (`loimi build`, `loimi run`, `--emit-llvm`, `--release`, `--dispatch=`). |
| Core language | `inline for` (module level and in `vm` blocks), structs (literals may be module-qualified: `diag.Span { .. }`), enums with payloads, `?T`, `Result` with `?`, tuples, arrays, slices, pointers and pointer arithmetic, methods (`fn Type.name`), modules and imports, `match` with exhaustiveness checking (through nested patterns and tuples), `let ... else`, `if let` chains, loops with `break` values, `defer`, consts, module-level `var`s (see below), casts (including integer to fieldless enum, checked in debug builds; `Kind.COUNT` gives the number of variants; `x as u64 < y` parses as a comparison), local inference across `if`/`match` branches (`if c { Ok(e) } else { Err(m) }` needs no annotation), checked/wrapping arithmetic, overflow and bounds checks (overflow traps in debug builds). |
| Globals | `var NAME: T = init;` at module level (with `pub`, readable and assignable from other modules as `mod.NAME` or after `import mod.{NAME}`). Plain LLVM globals; `#[thread_local]` makes one copy per thread (`thread_local(initialexec)`). The initializer must be a constant expression: literals, consts, `[x; N]`, struct/tuple/array literals, enum variants without payload, `None`/`Some(..)`/`Ok(..)`/`Err(..)` of constants, and functions (for hooks). Globals are not GC roots: a global holding `Gc`/`Value` must be marked by a root scanner (`heap.add_scanner`). |
| Generics | Monomorphized generic structs, enums, functions and methods; type arguments inferred from arguments and the expected type. Methods resolve per instance, so `HashMap<K, V>` works with any key type that has `hash`/`eq` methods (no traits needed yet). A generic function can be used as a function pointer where a function type is expected (`Pratt.new(peek, ...)` with `fn peek<E>(p: *P<E>) -> T`). Methods can have their own type parameters after the method name, on plain receivers (`fn Cursor.op<K>(c: *Cursor, ops: *OpTable<K>) -> ?K`) and generic ones (`fn Box<T>.map<U>(b: *Box<T>, f: fn(T) -> U) -> Box<U>`); the receiver's arguments come from the receiver, the method's are inferred from the arguments and the expected type. Type arguments can also be given explicitly on calls: `mem.alloc<u8>(n)`, `id<i64>(x)`, `c.op<K>(&ops)`, `Cursor.cast<u8>(&c)` (a method's own parameters only; not on a generic type's static functions such as `Vec.new`). `f<T>(` is recognized when `<` directly follows the name and a balanced, type-shaped `<...>` is followed by `(`; otherwise `<` is a comparison. |
| `value` types | `nanbox`, `lowtag`, `boxed` representations; NaN canonicalization; tuple patterns merged into one test (xor/or/mask, umax). |
| `vm` declarations | state, context, operand kinds, fixed-width formats, inline caches, `family` + `become`, `super`, vm functions, entries, `on raise`/`raise`, `cold`, `sync`. |
| Dispatch | `tailcall` (`preserve_none` + `musttail`, instruction word passed in a register), `goto` (one function, replicated `indirectbr`), `switch`. |
| Generated per vm | encoders (`Vm.OP(..)`), emitters (`Vm.emit.OP(..)`), opcode constants, `op_size`, `op_name`, the superinstruction peephole (`Vm.optimize`), and the verifier (`Vm.verify`). |
| Instrumentation | `variant NAME wraps all|[OPS] { before { .. } }`, switched with `use_variant`. `loimi build --vm-stats` counts executed ops and adjacent op pairs, and prints frequent pairs at exit as `super` declarations. |
| GC | `object` types (with trailing arrays), `Gc<T>`, `new(heap, ..)`, `gc_mark`; `std.gc` mark-sweep written in Loimi; compiler-generated tracing (including pointers inside values); shadow-stack frames for traced locals; effects (`uses sync/gc/reenter`), also on function types (`fn(*Heap) uses gc`), so calls through function pointers are checked and rooted. |
| Standard library | `std.io`, `std.os`, `std.mem` (`Vec`, `Arena`, allocation), `std.hash`, `std.map` (`HashMap`), `std.text` (`StrBuf`, `Interner`, number parsing/printing), `std.gc`, and the front-end modules below. |
| `std.num` | Exact numeral parsing with overflow reporting (`parse_int`, `parse_i64`/`parse_u64` in radix 2 to 36, `parse_u64_wrapping`, `parse_f64` via strtod with strict syntax, `parse_number` for literals, `decimal_len`), float formatting (`write_g` for `%.14g`, `write_shortest` for shortest round-trip, `mark_float` for Lua's `3.0`, `write_hex`), `add/sub/mul_overflow`, `floor_div`/`floor_mod`/`trunc_div`/`trunc_rem` with the `MIN / -1` case defined, float `floor_div_f64`/`floor_mod_f64`, `to_i64_exact`. |
| `std.diag` | `Span`, `SourceMap` (borrowed files, lazily built line-start table, binary-searched `line_col`, `line_text`, `write_location`), `Diag` (severity, message, primary span with label, notes and help lines, built with `with_label`/`with_note`/`with_note_at`/`with_help`), rendering with a snippet and carets in the compiler's format, and a `Diagnostics` sink that copies messages into an arena, counts errors and warnings, and prints a summary. |
| `std.lex` | A scanner toolkit rather than a generator: `Cursor` (peek, bump, line tracking, marks, spans, errors), `Syntax` presets (`lua()`, `c_like()`, `none()`) for line, block, nested and Lua long-bracket comments, escape and overflow rules; `skip_trivia`, `ident`, `number` (via std.num), `quoted` (escapes into a `StrBuf`, including `\ddd`, `\xHH`, `\u{..}`, `\z`, line continuation), `long_string`, `OpTable<K>` (maximal munch), `Keywords<K>`. Errors are `lex.Error` (span and message), convertible to a `diag.Diag`. A complete lexer for Mini's token set is about 50 lines (`tests/run/lex_toy.lm`). |
| `std.parse` | `Pratt<P, K, E, X>`: binding-power table indexed by token kind (`infix_left`, `infix_right`, `infix`, `prefix`) and a driver that calls the user's `peek`, `advance`, `atom`, `unary`, `binary` and optional `before_rhs` hooks through function pointers. |
| Example | `examples/mini/`: a Lua-like interpreter (lexer, compiler, VM, GC'd strings and tables), tested with GC stress. |
| Tests | `cargo test`: golden programs (every vm test in all three dispatch modes), expected compile errors, and the Mini scripts with and without GC stress. |

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
  so it needs no stack frame. Handler locals the cold code uses travel
  through thread-local slots (`initialexec`), written only on the cold path.
  `if c { cold { .. } }` followed by code ending in a control word gets that
  continuation copied into the cold block, so it can be outlined too. Other
  cold blocks, and all cold blocks in goto/switch mode, stay inline and are
  marked unlikely.
- **State kinds** (`Cursor`, `Window`, `StackTop`) are plain pointers.
- **vm functions** can read and write state but cannot use control words.
- **Effects** must be declared; they are not inferred for private functions.
  Function types carry them (`fn(*Heap) uses gc`), so calls through function
  pointers are checked, synced and rooted like direct calls. There are no
  effect variables: a generic function cannot take "a callback with any
  effects" and inherit them.
- **Verifier**: operand checks only. Stack-effect checking for stack VMs
  and "cannot fall off the end" are not implemented.
- **Instrumentation**: switching tables happens from vm code (`use_variant`),
  not through an external `set_dispatch`.
- **Front-end stdlib** (`std.lex`, `std.parse`, `std.diag`, `std.num`) is
  smaller than the design: there is no comptime, so `std.lex` is a cursor
  toolkit instead of a DFA generator and the Pratt table is filled at run
  time; there are no `TokenStream` or error-recovery helpers, fix-it hints,
  ANSI color, `BigInt`, or Ryu/Eisel-Lemire (formatting and parsing use
  snprintf and strtod). See the notes in [`stdlib.md`](stdlib.md).
- **`inline for`** is token-level expansion: values are token sequences
  (so operators like `+` work), and `#var` makes an identifier from a
  string or identifier value.
- **Not yet built**: traits/interfaces and closures (see the proposal in
  [`design/closures-traits.md`](design/closures-traits.md)), a moving,
  incremental or generational GC, weak references, ephemerons, finalizers,
  `std.bc`/`obj`/`rt`/`prof`/`test`, line tables,
  variable-length encodings, atomic `become` for shared code, and the C
  embedding header.
