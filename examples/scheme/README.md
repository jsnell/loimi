# A small Scheme in Loimi

This example is an interpreter for a subset of R7RS-small Scheme, written
in Loimi. This README describes what it supports and how it is built.

Programs are compiled to register bytecode and run on a Loimi `vm`. The
design follows [`examples/mini`](../mini):

- a NaN-boxed `value` type and GC `object`s;
- a `vm` with cold paths, a `sync` block and an `on raise` handler;
- a quickened instruction family and superinstructions;
- a compiler written in Loimi that emits code through the generated
  `Scheme.X` encoders and checks it with the generated verifier.

## Build, run and test

```sh
cargo build --release
./target/release/loimi build examples/scheme/main.lm --release -o scheme
./scheme program.scm
cargo test --release --test golden scheme_programs
bench/scheme/compare.py         # benchmarks against other Schemes
```

| Command line | What it does |
|---|---|
| `scheme PROGRAM` | Runs the program. |
| `scheme PROGRAM disasm` | Prints the compiled bytecode instead of running. |
| `scheme PROGRAM stress` | Runs with a garbage collection at every safepoint (for testing). |
| `scheme PROGRAM verify` | As `stress`, with a generational heap whose write barriers are checked at each collection. |

## Files

| File | Contents |
|---|---|
| `bignum.lm` | Exact integers beyond the fixnums, on `std.bigint`: promotion, normalization back to fixnums, exact comparison with flonums, radix I/O. |
| `value.lm` | The `V` value type; heap objects (pairs, strings, symbols, vectors, closures, prototypes); printing; `eqv?` and `equal?`; the symbol table. |
| `read.lm` | The reader, on `std.lex` (cursor, comments) and `std.num` (numerals). |
| `compile.lm` | S-expressions to AST (derived forms expanded, variables resolved), then inlining and contification, then bytecode. |
| `vm.lm` | The `vm Scheme` declaration: ops, calls and tail calls, the error handler. |
| `rt.lm` | The interpreter context (stacks, globals) and the GC root scanner. |
| `builtins.lm` | The builtin procedures (natives) and numeric helpers. |
| `prelude.lm` | Builtins written in Scheme (`map`, `for-each`, `vector-map`, ...). |
| `main.lm` | The driver. |
| `tests/*.scm` | Test programs with `; expect:` lines, run by `tests/golden.rs` normally and under GC stress. |

## The language

### Data types

- exact integers: 32-bit fixnums, and bignums beyond them;
- flonums;
- booleans, characters, strings, interned symbols;
- pairs and lists, vectors, records;
- procedures;
- the unspecified value.

### Reader

- lists with `()` or `[]`, and dotted pairs;
- `'`, `` ` ``, `,` and `,@`;
- `#t`, `#f`, `#true`, `#false`;
- characters: `#\a`, `#\space`, `#\x41`;
- strings with `\n \t \r \a \0 \\ \" \x41;` escapes and line
  continuations;
- vectors `#(...)`;
- `;` comments, nested `#| |#` comments and `#;` datum comments;
- number prefixes `#x #o #b #d #e #i`, and `+inf.0`, `-inf.0`, `+nan.0`.

### Special forms

- `define`: variables, the procedure shorthand, and internal defines
  with `letrec*` semantics;
- `lambda`, with fixed, rest and variadic parameters;
- `if`, `set!`, `quote`;
- `let`, `let*`, `letrec`, `letrec*`, named `let`;
- `begin`, splicing at top level and in bodies;
- `cond` and `case`, both with `else` and `=>`;
- `and`, `or`, `when`, `unless`, `do`;
- `quasiquote`, `unquote` and `unquote-splicing`, nested and in vectors;
- `define-record-type`, at top level and in bodies.

Local variables can shadow keywords. A top-level `(import ...)` is
accepted and ignored: every procedure a program can name is built in.

### Builtins

There are 149 natives, plus the procedures in the prelude.

| Group | Procedures |
|---|---|
| Numbers | `+ - * / quotient remainder modulo floor/ truncate/ floor-quotient floor-remainder truncate-quotient truncate-remainder exact-integer-sqrt = < > <= >= zero? positive? negative? even? odd? number? integer? exact? inexact? exact-integer? abs min max gcd lcm sqrt expt exp log sin cos tan asin acos atan floor ceiling round truncate exact->inexact inexact->exact exact inexact square number->string string->number` |
| Pairs and lists | `cons car cdr`, `c[ad]{2,4}r`, `set-car! set-cdr! list length append reverse list-tail list-ref list-copy last-pair memq memv member assq assv assoc map for-each apply` |
| Predicates | `null? pair? list? eq? eqv? equal? not symbol? string? procedure? vector? boolean? char?` |
| Vectors | `make-vector vector vector-ref vector-set! vector-length vector->list list->vector vector-fill! vector-map vector-for-each` |
| Strings and characters | `string-length string-ref substring string-append string=? string<?` and the other comparisons, `string-copy string->list list->string make-string string string-set! symbol->string string->symbol char->integer integer->char char=? char<? char-upcase char-alphabetic?`, ... |
| Output | `display write newline write-string write-char flush-output-port current-output-port` (port arguments are accepted and ignored) |
| Input | `read` (from standard input), `eof-object eof-object?` |
| Time | `current-second current-jiffy jiffies-per-second` (`current-jiffy` counts microseconds of wall-clock time from its first call) |
| Control | `values call-with-values`, `call-with-current-continuation` and `call/cc` (escape only), `error`, `exit` |

### Tail calls and errors

- **Proper tail calls** everywhere: `if`, `cond`, `case`, `and`, `or`,
  `when`, `let` and the other binding forms, `apply`, calls to builtins
  and to procedures with rest parameters.
- **Recursion depth.** The frame stack holds 131072 frames, so non-tail
  recursion 100000 deep works. Deeper recursion stops with
  `stack overflow`.
- **Errors.** Run-time errors print `error: line N: <message>` on stderr
  and exit with status 1. The line is that of the failing expression. For
  errors inside prelude code such as `map`, it is the line of the nearest
  caller in the program. Syntax errors are reported the same way.

### Differences from R7RS

- **Exact integers are fixnums or bignums.** A fixnum result that
  overflows becomes a bignum, and a bignum result that fits a fixnum
  becomes a fixnum again.
  - Comparisons between bignums and flonums are exact:
    `(< (+ (expt 2 53) 1) 9007199254740992.0)` is `#f`.
  - `exact->inexact` rounds correctly.
  - `number->string` and `string->number` take radix 2, 8, 10 or 16.
- **No rationals.** `(/ 1 3)` is `0.3333333333333333`. `(/ 6 3)` and
  `(/ (expt 10 30) (expt 10 28))` stay exact. An exact negative exponent
  in `expt` gives a flonum.
- **Strings are byte strings.** `string-length` and `string-ref` count
  bytes. Characters are code points, and `#\λ` or `\x3bb;` are encoded as
  UTF-8 when put into a string.
- **Continuations are escape-only.** A continuation can be invoked while
  the `call/cc` that made it is still active: early exit from loops,
  searches and recursions, as in the `ctak`, `fibc` and `puzzle`
  benchmarks. Invoking it after that `call/cc` has returned is an error
  (`continuation invoked after its extent ended`).
- **Multiple values** are a list tagged with a private marker, unless
  there is exactly one value. Only `call-with-values` takes them apart.
- **Records** are vectors with a hidden type field. `vector?` is false
  for them and `equal?` compares them by identity, but the inline
  `vector-ref` op does not check the type.
- Symbols are never collected. `eq?` on flonums compares bits.

### Not supported

- `dynamic-wind`;
- `syntax-rules` and `define-syntax` (reported as unsupported);
- re-entrant continuations;
- file and string ports;
- `delay`;
- bytevectors, rationals, complex numbers.

## Architecture

### Values

```loimi
pub value V repr nanbox {
    Num(f64) unboxed,    // flonums
    Int(i32),            // fixnums
    Imm(i32),            // '(), #f, #t, unspecified, undefined, eof, builtins, chars
    P(Gc<Pair>), Sym(Gc<Sym>), S(Gc<Str>), Vec(Gc<Vect>), Clo(Gc<Closure>),
}
```

The NaN box has room for seven tagged variants, two fewer than the
natural Scheme set. So the immediates share `Imm(i32)` and are told apart
by the payload: `#f` is `Imm(1)`, the character `a` is
`Imm(CHAR_BASE + 97)`, and builtin number 39 (`car`) is
`Imm(PRIM_BASE + 39)`. A truth test is a single compare against the bits
of `Imm(1)`, and `null?` against those of `Imm(0)`.

Boxes for assigned captured variables reuse the vector object with one
slot, since programs never see them.

Bignums (`Big`) are `std.bigint` GC objects. Keeping small results as
fixnums means `eqv?` on small integers stays a word compare, and the
fixnum fast paths never see a small bignum. Overflow, like mixed
operands, goes to the op's cold path, which is also a safepoint, so a
loop of bignum arithmetic with no call still collects.

### Why a register VM

The compiler works on an AST, so it knows every variable's lifetime and
can assign registers directly:

- parameters are R0..Rn;
- `let` variables get the next registers;
- temporaries go above them.

Compared with a stack machine, this saves most of the loads and stores
that move locals to the top of the stack and back. `(+ n 1)` is one
`ADDI` that reads `n`'s register, so each source operation is usually one
dispatch.

Calls need no copying: the caller evaluates the procedure and its
arguments into consecutive registers at the top of its window, and the
callee's window starts right there, so the arguments arrive as R0..Rn-1.

### Compiler

`compile.lm` works in five passes.

1. **Build** (s-expressions to AST). Expands derived forms (`let*`, named
   `let`, `do`, `cond`, `case`, `when`, `unless`, quasiquote, internal
   defines) into a small core: `Const Ref GRef Set GSet GDef If Lambda
   Seq Call Prim Let Letrec And Or`. Resolves every variable, records
   which ones are assigned, and notes the top-level definitions and
   `set!`s of each global.
2. **Inline** (`Comp.inl`). Replaces a call of a *known procedure* with a
   copy of its body (see below).
3. **Contify** (`contify`). A `letrec` lambda whose every use is a tail
   call from the letrec body, or from another contified lambda of the
   same letrec, is compiled inline in the enclosing procedure. This
   covers every named-let loop and mutually tail-recursive local
   procedures. Its parameters become registers, its calls become moves
   and jumps, and its returns go to the letrec's continuation. No closure
   is made, and its free variables are the enclosing procedure's
   registers.
4. **Resolve** (`reown`). Gives every variable its real owner (the
   procedure whose frame holds it) and recomputes which variables are
   captured and each lambda's free variables, as in Lua's upvalue
   resolution. A variable that is assigned and used only by contified
   code is no longer boxed.
5. **Generate** (AST to bytecode). Allocates registers and emits code
   with the generated encoders (`Scheme.ADD(dst, a, b)`). Conditions
   compile to branches through `not`, `and` and `or`. Each finished
   prototype is checked with the generated `Scheme.verify` and fused into
   superinstructions with `Scheme.optimize`.

**Known procedures.** A top-level procedure that is defined once, never
`set!` and not named like a builtin is known, and so is an unassigned
`let` or `letrec` lambda.

- Bodies of up to 12 nodes are inlined at their calls, or up to 40 nodes
  when an argument is a lambda or a known procedure. Then calls of that
  parameter in the copy are known too and are inlined in turn, so
  `(map (lambda (x) ...) l)` becomes a loop with the lambda's body in it.
- Recursive bodies are not inlined, and nesting stops at depth 3.
- A call of an inlined top-level procedure that could run before its
  definition is guarded, so it still fails with `unbound variable`.
- Other calls of a known top-level procedure without free variables,
  whose definition has certainly run, compile to `CALLK`: the call takes
  the closure from the constant pool, with no global load and no arity
  check. A definition has certainly run if it is in an earlier form, or
  no earlier form can reach the body the call is in
  (`Comp.reachability`).
- A global defined once, never `set!`, as another name for a builtin or
  a known procedure (`(define make-node vector)`) is an alias: its calls
  compile as calls of the original.

**Inline builtins.** Calls to these builtins compile to ops:

```
+ - * / quotient remainder modulo = < > <= >= zero? eq? eqv? not null?
pair? car cdr cons set-car! set-cdr! vector-ref vector-set! symbol?
number? exact-integer? string? vector? memq assq list vector length odd?
even? positive? negative? vector-length reverse string-ref cXr call/cc
```

Two-list `append` and record constructors, accessors, modifiers and
predicates do too. `list` becomes a chain of `CONS`, and `vector` and
record constructors become `VECTOR` and `RECORD`. The other ops have a
fast path and leave every other case to the native. Calls to the
remaining builtins, except `apply`, call the native directly
(`PRIMCALL`).

If the program defines or `set!`s a builtin's name anywhere, every call
of that name goes through the global instead. The prelude always uses
the inline versions.

### Closures

Closures are flat. `CLOSURE` copies the values of the free variables into
the new closure, following capture descriptors stored in the prototype
("register r of the parent" or "free slot i of the parent").

- Variables that are both captured and `set!` live in a box (`MKBOX`,
  `BOXGET`, `BOXSET`).
- Free variables are read with `GETFREE`. Those that a contified loop
  uses are read once into registers before the loop.
- `letrec` and internal defines of lambdas are not boxed. The closures
  are created in order, and their slots for variables initialized later
  are patched with `SETFREE` once those have values. A `letrec` variable
  is boxed only if a closure created by a non-lambda initializer could
  capture it before it is initialized.

### Calls and tail calls

Frames live on a separate frame stack (`rt.Frame`: closure, register
base, return address, constants).

- **`CALL base nargs`** handles the common case in its hot path: a
  closure with exactly that many fixed parameters. It checks the stack,
  pushes a frame and jumps. Everything else (rest parameters, arity
  errors, builtins, `apply`, non-procedures) goes through a cold path,
  the vm function `call_slow`.
- **`TAILCALL`** moves the arguments down to R0 and replaces the current
  frame's closure, so tail calls never grow either stack.
- **Self tail calls** (through the procedure's own `letrec` variable or
  the global of a known top-level procedure) compile to moves into the
  parameter registers and a `JMP` to the start.
- **`CALLSELF`** is a non-tail call of a procedure to itself with its
  number of parameters. It calls the running closure with no checks.
- **`apply`** is handled in `call_slow` by spreading the list into
  registers and dispatching again, so it is properly tail-recursive too.

Builtins that take procedures (`map`, `for-each`, ...) are written in
Scheme in the prelude, so natives never call back into the VM.

### Globals

Each symbol used as a global gets a slot in `ctx.gvals` when the compiler
first sees it, so a global read is `GETGLOBAL dst, slot`: a load and an
"unbound" check. An inline cache would give nothing more.

Once a slot is bound it stays bound, since there is no undefine. So
`GETGLOBAL` is a family: the first successful read quickens the
instruction into `GETGLOBAL_B`, which skips the check.

### GC

The collector is mark-sweep from `std.gc`, with 16-byte object headers,
so a pair takes 32 bytes. A collection runs after allocating 4 times the
live data, at least 2 MiB and at most 256 MiB.

- **Roots** come from `rt.scan_roots`: symbols, globals, the pending
  error, and every active frame's closure and whole register window.
- **When it runs.** Every op that allocates (`CONS`, `CLOSURE`, `MKBOX`,
  and the call path, which covers builtins and rest lists) runs the
  collector if due, *before* allocating, in a `cold` block, with the
  `sync` block publishing the frame pointer. The ops have no stack frame.
- Allocation itself never collects, so natives and the compiler allocate
  freely.
- **Stale registers.** Registers are not cleared on entry to a frame. The
  stack is a `std.rt.ValueStack` with a high-water mark: every slot below
  it holds a value that is safe to mark, and the scanner clears the slots
  above the top window and lowers the mark. A call whose window passes
  the mark raises it, on the cold path that also checks for stack
  overflow.

### Ops

| Group | Ops |
|---|---|
| Moves and constants | `LOADK LOADI MOVE` |
| Globals | `GETGLOBAL` (and `_B`), `CHKGLOBAL` (and `_B`), `SETGLOBAL DEFGLOBAL` |
| Closures and boxes | `GETFREE SETFREE MKBOX BOXGET BOXSET CLOSURE` |
| Jumps | `JMP TEST TESTT` |
| Arithmetic | `ADD SUB MUL ADDI DIV QUO REM MOD` (fixnum fast paths with overflow checks, flonum fast paths, everything else cold) |
| Comparisons | `NUMEQ LT GT LE GE`, and with a constant `NUMEQI LTI GTI LEI GEI` |
| Inline builtins | `ZEROP EQ EQV NOT NULLP PAIRP SYMBOLP NUMBERP FIXNUMP STRINGP VECTORP CAR CDR CONS SETCAR SETCDR VREF VSET VREFI VSETI VECTOR RECORD MEMQ ASSQ LENGTH ODDP EVENP POSP NEGP VLEN REVERSE APPEND2 SREF` |
| Records | `RREF RSET RECP` (type and field as a constant operand) |
| Calls | `CALL CALLSELF CALLK TAILCALLK CALLCC PRIMCALL TAILCALL RET` |

There are also 35 superinstructions picked with `--vm-stats` over the
benchmarks: compare-and-branch pairs, `MOVE_MOVE`, `MOVE_JMP`, `CDR_CDR`,
`ADDI_CALL`, and others.

### Escape continuations

`call/cc` is the `CALLCC` op. (Called as a value, it is handled in
`call_slow`, like `apply`.)

1. It allocates a continuation: a closure of a fixed prototype
   (`vm.cont_proto`) whose one free slot is a frame depth.
2. It pushes a frame of that prototype (or, for a tail call, replaces the
   current frame) with R0 = the procedure and R1 = the continuation.
3. The prototype's code is `CALL R0 1; RET R0`. That is a non-tail call,
   so the frame stays where it is while the procedure runs, and a normal
   return passes the result through.

Invoking the continuation always reaches `call_slow`, because its arity
never matches. It checks that the frame at its depth still holds this
continuation as its closure, which is true exactly while that `call/cc`
is active. Then it returns the argument from that frame, dropping
everything above it. Both capture and escape take constant time, and
ordinary calls pay nothing.

Full continuations, `dynamic-wind` and `syntax-rules` were left out to
keep the core small; the language does not block any of them:

- **Re-entrant `call/cc`.** Frames and registers live on explicit
  stacks, never on the C stack, so a full continuation could copy both
  stacks into a heap object, and invoking it would copy them back and
  `jump` to the saved return address.
- **`dynamic-wind`** needs a wind list in the context. Invoking a
  continuation would run the `after` and `before` thunks, either by
  re-entering the VM from a native (`uses reenter`) or with a small
  Scheme procedure in the prelude.
- **`syntax-rules`** is a front-end feature: a pattern matcher and
  template expander on s-expressions before the build pass, plus hygiene.
  It does not touch the VM.

## Performance

The benchmarks are 43 programs from the public
[r7rs-benchmarks](https://github.com/ecraven/r7rs-benchmarks) suite, used
unmodified. Of the other 14 programs in the suite, 11 need file ports,
and `equal`, `mbrotZ` and `bv2string` need a sharing-aware `equal?`,
complex numbers or bytevectors.

In the latest run the interpreter was faster than Gauche on every
program, and faster than Guile 3's bytecode VM on all but `pi` (whose
bignum arithmetic Guile does with GMP). It took 0.22 of Gauche's time, 0.38 of
Guile's without its JIT, and 0.80 of Guile's with its JIT, by geometric
mean. The numbers are in [`docs/results.md`](../../docs/results.md), and
how the benchmarks are run is in `bench/scheme/README.md`.
