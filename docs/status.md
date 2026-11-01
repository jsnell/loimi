# Implementation status

What exists today, and where the implementation differs from the design
in [`language.md`](language.md). Updated 2026-11-01.

## Built

| Area | Status |
|---|---|
| Compiler | Rust, no dependencies. Emits textual LLVM IR and drives `clang-19` (`loimi build`, `loimi run`, `--emit-llvm`, `--release`, `--dispatch=`). |
| Core language | structs, enums with payloads, `?T`, `Result` with `?`, tuples, arrays, slices, pointers and pointer arithmetic, methods (`fn Type.name`), modules and imports, `match` with exhaustiveness checking, `let ... else`, `if let` chains, loops with `break` values, `defer`, consts, casts, checked/wrapping arithmetic, overflow and bounds checks (overflow traps in debug builds). |
| Generics | Monomorphized generic structs, enums, functions and methods; type arguments inferred from arguments and the expected type. Methods resolve per instance, so `HashMap<K, V>` works with any key type that has `hash`/`eq` methods (no traits needed yet). |
| `value` types | `nanbox`, `lowtag`, `boxed` representations; NaN canonicalization; tuple patterns merged into one test (xor/or/mask, umax). |
| `vm` declarations | state, context, operand kinds, fixed-width formats, inline caches, `family` + `become`, `super`, vm functions, entries, `on raise`/`raise`, `cold`, `sync`. |
| Dispatch | `tailcall` (`preserve_none` + `musttail`, instruction word passed in a register), `goto` (one function, replicated `indirectbr`), `switch`. |
| Generated per vm | encoders (`Vm.OP(..)`), emitters (`Vm.emit.OP(..)`), opcode constants, `op_size`, `op_name`, the superinstruction peephole (`Vm.optimize`), and the verifier (`Vm.verify`). |
| Instrumentation | `variant NAME wraps all|[OPS] { before { .. } }`, switched with `use_variant`. |
| GC | `object` types (with trailing arrays), `Gc<T>`, `new(heap, ..)`, `gc_mark`; `std.gc` mark-sweep written in Loimi; compiler-generated tracing (including pointers inside values); shadow-stack frames for traced locals; effects (`uses sync/gc/reenter`). |
| Standard library | `std.io`, `std.os`, `std.mem` (`Vec`, `Arena`, allocation), `std.hash`, `std.map` (`HashMap`), `std.text` (`StrBuf`, `Interner`, number parsing/printing), `std.gc`. |
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
- **`cold` blocks are not outlined yet.** They are marked unlikely (a
  `cold` call-site attribute on `llvm.sideeffect`), which moves them out of
  the hot path's layout, but they still share the handler's register
  allocation.
- **State kinds** (`Cursor`, `Window`, `StackTop`) are plain pointers.
- **vm functions** can read and write state but cannot use control words.
- **Effects** must be declared; they are not inferred for private functions.
- **Verifier**: operand checks only. Stack-effect checking for stack VMs
  and "cannot fall off the end" are not implemented.
- **Instrumentation**: switching tables happens from vm code (`use_variant`),
  not through an external `set_dispatch`.
- **Not yet built**: `inline for`, traits/interfaces, a moving,
  incremental or generational GC, weak references, ephemerons, finalizers,
  `std.lex`/`parse`/`diag`/`bc`/`obj`/`num`/`rt`/`prof`/`test`, line tables,
  variable-length encodings, atomic `become` for shared code, and the C
  embedding header.
