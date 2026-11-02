# A small Scheme in Loimi

The second proof interpreter: a subset of R7RS-small, compiled to register
bytecode and run on a Loimi `vm`. It follows the architecture of
[`examples/mini`](../mini): a NaN-boxed `value`, GC `object`s, a `vm` with
cold paths, a `sync` block, an `on raise` handler, a quickened family and
superinstructions, and a compiler written in Loimi that emits code through
the generated `Scheme.X` encoders and checks it with the generated verifier.

```sh
cargo build --release
./target/release/loimi build examples/scheme/main.lm --release -o scheme
./scheme program.scm            # a second argument turns on GC stress
./scheme program.scm disasm     # print the compiled bytecode instead
cargo test --release --test golden scheme_programs
bench/scheme/compare.py         # benchmarks vs chibi-scheme (see bench/scheme/README.md)
```

## Files

| File | Contents |
|---|---|
| `value.lm` | the `V` value type, heap objects (pairs, strings, symbols, vectors, closures, prototypes), printing, `eqv?`/`equal?`, the symbol table |
| `read.lm` | the reader, on `std.lex` (cursor, comments) and `std.num` (numerals) |
| `compile.lm` | s-expressions → AST (derived forms expanded, variables resolved) → inlining, contification → bytecode |
| `vm.lm` | the `vm Scheme` declaration: ops, calls and tail calls, error handler |
| `rt.lm` | the interpreter context (stacks, globals) and the GC root scanner |
| `builtins.lm` | the builtin procedures (natives) and numeric helpers |
| `prelude.lm` | builtins written in Scheme (`map`, `for-each`, `vector-map`, ...) |
| `main.lm` | driver |
| `tests/*.scm` | test programs with `; expect:` lines, run by `tests/golden.rs` normally and under GC stress |

Benchmarks live in [`bench/scheme`](../../bench/scheme/README.md): the
public r7rs-benchmarks suite, run unmodified against chibi-scheme.

## What is supported

- **Data.** Fixnums (32-bit), flonums, booleans, characters, strings,
  interned symbols, pairs and lists, vectors, records, procedures,
  unspecified.
- **Reader.** Lists with `()` or `[]`, dotted pairs, `'` `` ` `` `,` `,@`,
  `#t`/`#f`/`#true`/`#false`, `#\a` `#\space` `#\x41` characters, strings
  with `\n \t \r \a \0 \\ \" \x41;` escapes and line continuations,
  vectors `#(...)`, `;` comments, nested `#| |#` and `#;` datum comments.
- **Special forms.** `define` (variables, procedure shorthand, internal
  defines with `letrec*` semantics), `lambda` (fixed, rest and variadic
  parameters), `if`, `set!`, `let`, `let*`, `letrec`, `letrec*`, named
  `let`, `begin` (splicing at top level and in bodies), `cond` (with `else`
  and `=>`), `case` (with `else` and `=>`), `and`, `or`, `when`, `unless`,
  `do`, `quote`, `quasiquote`/`unquote`/`unquote-splicing` (nested, and in
  vectors), `define-record-type` (top level and in bodies). Keywords can be
  shadowed by local variables. A top-level `(import ...)` is accepted and
  ignored: every procedure the program can name is built in.
- **Builtins** (126 natives plus the prelude): numbers (`+ - * / quotient
  remainder modulo = < > <= >= zero? positive? negative? even? odd? number?
  integer? exact? inexact? exact-integer? abs min max gcd lcm sqrt expt exp
  log sin cos tan asin acos atan floor ceiling round truncate
  exact->inexact inexact->exact exact inexact square number->string
  string->number`), pairs and lists (`cons car cdr` and
  `c[ad]{2,4}r`, `set-car! set-cdr! list length append reverse list-tail
  list-ref list-copy last-pair memq memv member assq assv assoc map
  for-each apply`), predicates (`null? pair? list? eq? eqv? equal? not
  symbol? string? procedure? vector? boolean? char?`), vectors
  (`make-vector vector vector-ref vector-set! vector-length vector->list
  list->vector vector-fill! vector-map vector-for-each`), strings and chars
  (`string-length string-ref substring string-append string=? string<?`
  and friends, `string-copy string->list list->string make-string string
  string-set! symbol->string string->symbol char->integer integer->char
  char=? char<? char-upcase char-alphabetic?` ...), output (`display write
  newline write-string write-char flush-output-port current-output-port`;
  the port arguments are accepted and ignored), `read` (from standard
  input), `eof-object eof-object?`, `current-second current-jiffy
  jiffies-per-second`, `values call-with-values`,
  `call-with-current-continuation`/`call/cc` (escaping only, see below),
  `error`, `exit`.
- **Proper tail calls** everywhere (`if`, `cond`, `case`, `and`, `or`,
  `when`, `let`..., `apply`, calls to builtins and to procedures with rest
  parameters). The frame stack holds 131072 frames, so non-tail recursion
  100000 deep works; deeper recursion stops with `stack overflow`.
- **Errors.** Run-time errors print `error: line N: <message>` on stderr
  and exit with status 1. The line is that of the failing expression, or,
  for errors inside prelude code such as `map`, of the nearest caller in the
  program. Syntax errors are reported the same way.

### Deviations and choices

- **Fixnum overflow produces a flonum** (`(* 65536 65536)` ⇒
  `4294967296.0`). There are no bignums and no rationals: `(/ 1 3)` is
  `0.3333333333333333`, while `(/ 6 3)` stays exact. Integer literals that
  do not fit in 32 bits read as flonums. 32 bits is what a nanbox payload
  holds (see "Language features missed").
- **Strings are byte strings.** `string-length` and `string-ref` count
  bytes; characters are code points, and `#\λ` or `\x3bb;` are encoded as
  UTF-8 when put into a string.
- **Inline builtins.** Calls to `+ - * / quotient remainder modulo = < > <=
  >= zero? eq? eqv? not null? pair? car cdr cons set-car! set-cdr!
  vector-ref vector-set! symbol? number? exact-integer? string? vector?`
  and `cXr` are compiled to ops, and calls to the other builtins (except
  `apply` and `call/cc`) call the native directly (`PRIMCALL`), unless the
  program defines or `set!`s that name somewhere (then every call goes
  through the global). The prelude always uses the inline versions.
- **Inlined procedures.** A top-level procedure that is defined once and
  never `set!` (and not named like a builtin) is a known procedure, and so
  is an unassigned `let`/`letrec` lambda: small ones are inlined at their
  calls (see "Compiler"). A call of an inlined top-level procedure that
  could run before its definition is guarded, so it still fails with
  `unbound variable`.
- Symbols are never collected. `eq?` on flonums compares bits.
- **Continuations are escape-only.** A continuation can be invoked while
  the `call/cc` that made it is still active (early exit from loops,
  searches and recursions, as in the `ctak`, `fibc` and `puzzle`
  benchmarks). Invoking it after that `call/cc` has returned is an error
  (`continuation invoked after its extent ended`), not a re-entry.
- **Multiple values** are a list tagged with a private marker unless
  there is exactly one value; only `call-with-values` takes them apart.
- **Records** are vectors with a hidden type field: `vector?` is false for
  them and `equal?` compares them by identity, but the inline
  `vector-ref` op does not check the type.
- `current-jiffy` counts microseconds of wall-clock time from its first
  call.
- Not supported: `dynamic-wind`, `syntax-rules`/`define-syntax` (they are
  reported as unsupported), re-entrant continuations, file and string
  ports, `delay`, bytevectors, bignums, rationals, complex numbers.

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

A nanbox has room for seven tagged variants, which is two fewer than the
natural Scheme set, so the payload-free immediates share `Imm(i32)` and are
told apart by the payload (`#f` is `Imm(1)`, the character `a` is
`Imm(CHAR_BASE + 97)`, builtin number 39 (`car`) is `Imm(PRIM_BASE + 39)`).
Truthiness is a single compare against the bits of `Imm(1)`, and `null?`
against those of `Imm(0)`. Boxes for assigned captured variables reuse the
vector object (one slot), since they are never visible to programs.

### Why a register VM

The compiler is a two-pass compiler over an AST, so it knows every
variable's lifetime and can assign registers directly: parameters are
R0..Rn, `let` variables get the next registers, and temporaries go above.
Compared with a stack machine that saves most of the loads and stores that
move locals to the top of the stack and back (`(+ n 1)` is one `ADDI` that
reads `n`'s register), so each source operation is usually one dispatch.
Calls need no copying: the caller evaluates the procedure and its
arguments into consecutive registers at the top of its window, and the
callee's window starts right there, so the arguments arrive as R0..Rn-1.
Mini made the same choice and its idioms carry over directly, which was
part of the point of the exercise.

### Compiler

`compile.lm` works in passes:

1. **Build** (s-expressions → AST): expands derived forms (`let*`, named
   `let`, `do`, `cond`, `case`, `when`, `unless`, quasiquote, internal
   defines) into a small core (`Const Ref GRef Set GSet GDef If Lambda Seq
   Call Prim Let Letrec And Or`). It resolves every variable to a `Var`,
   records which ones are assigned, and notes the top-level definitions
   and `set!`s of each global.
2. **Inline** (`Comp.inl`): a call of a known procedure (see above) is
   replaced by a copy of its body, with the parameters bound by a `let` or
   replaced by the argument when that is a constant, an unassigned local or
   a known procedure. Bodies of up to 12 nodes are inlined, or up to 40
   when an argument is a lambda or a known procedure: then calls of that
   parameter in the copy are known too and are inlined in turn, so
   `(map (lambda (x) ...) l)` becomes a loop with the lambda's body in it
   (`map` and `for-each` with one list use the prelude's one-list versions).
   Recursive bodies are not inlined, and nesting stops at depth 3.
3. **Contify** (`contify`): a `letrec` lambda whose every use is a tail call
   from the letrec body or from another contified lambda of the same letrec
   (every named-let loop, mutually tail-recursive local procedures) is
   compiled inline in the enclosing procedure: its parameters become
   registers, its calls become moves and jumps, and its "returns" go to the
   letrec's continuation. No closure is made and its free variables are
   the enclosing procedure's registers.
4. **Resolve** (`reown`): gives every variable its real owner (the
   procedure whose frame holds it) and recomputes which variables are
   captured and each lambda's free variables, as in Lua's upvalue
   resolution. A variable that is assigned and used only by contified code
   is no longer boxed.
5. **Generate** (AST → bytecode): allocates registers and emits code with
   the generated encoders (`Scheme.ADD(dst, a, b)`), in value-into-register
   and effect-only contexts and in tail position with a continuation
   (return, or a register or nothing followed by a jump to a join point,
   for contified bodies). Conditions compile to branches through `not`,
   `and` and `or`; in tail position `and`/`or` of predicates jump to a
   `#t`/`#f` exit. Each finished prototype is checked with the generated
   `Scheme.verify` and peephole-fused with `Scheme.optimize`.

### Closures

Closures are **flat**: `CLOSURE` copies the values of the free variables
into the new closure, following capture descriptors stored in the
prototype ("register r of the parent" or "free slot i of the parent").
Variables that are both captured and `set!` are **assignment-converted**:
they live in a box (`MKBOX`, `BOXGET`, `BOXSET`). Free variables are read
with `GETFREE`.

`letrec` (and internal defines) of lambdas, the common case for recursive
local procedures and named `let`, are not boxed: the closures are created
in order and their slots for variables initialized later are patched with
`SETFREE` once those have values. A `letrec` variable is boxed only if a
closure created by a non-lambda initializer could capture it before it is
initialized.

### Calls and tail calls

Frames live on a separate frame stack (`rt.Frame`: closure, register base,
return address, constants). `CALL base nargs` handles the common case in
its hot path (a closure with exactly that many fixed parameters): clear
the callee's remaining registers, push a frame, and jump. Everything else
(rest parameters, arity errors, builtins, `apply`, non-procedures) goes
through a cold path, the vm function `call_slow`, which sets up the state
and returns where to continue.

`TAILCALL` moves the arguments down to R0.. and replaces the current
frame's closure, so tail calls never grow either stack. A tail call of a
procedure to itself, through its own `letrec` variable or the global of a
known top-level procedure, compiles to moves into the parameter registers
and a `JMP` to the start, with arguments that are single ops computed
straight into their parameter when no other argument reads it. A non-tail
call of a procedure to itself with its number of parameters is
`CALLSELF`, which calls the running closure with no checks.

`apply` is handled in `call_slow` by spreading the list into registers and
dispatching again, so it is properly tail-recursive too. Builtins that take
procedures (`map`, `for-each`, ...) are written in Scheme in the prelude,
so natives never call back into the VM.

### Globals

Each symbol used as a global gets a slot in `ctx.gvals` when the compiler
first sees it. The compiler resolves names to slots, so a global read is
`GETGLOBAL dst, slot`: a load and an "unbound" check. That is what an inline
cache would converge to, and there is nothing to miss, so there is no IC.
`GETGLOBAL` is a `family` instead: once a slot is seen bound it stays bound
(there is no undefine), so the first successful read quickens the
instruction into `GETGLOBAL_B`, which skips the check.

### GC

Mark-sweep from `std.gc` (16-byte object headers, so a pair takes 32
bytes). A collection runs after allocating 4 times the live data (at
least 2 MiB, at most 256 MiB; `Heap.set_growth`, `set_max_budget`).
Roots come from `rt.scan_roots`: symbols,
globals, the pending error, and every active frame's closure and whole
register window. Every op that allocates (`CONS`, `CLOSURE`, `MKBOX`, and
the call path, which covers builtins and rest lists) runs the collector,
if due, *before* allocating, in a `cold` block, with the `sync` block
publishing the frame pointer. Allocation never collects, so natives and
the compiler allocate freely. Registers are cleared on entry to a frame,
so the scanner never sees stale values.

### Ops

`LOADK LOADI MOVE`, `GETGLOBAL`(+`_B`) `CHKGLOBAL`(+`_B`) `SETGLOBAL
DEFGLOBAL`, `GETFREE SETFREE MKBOX BOXGET BOXSET CLOSURE`, `JMP TEST
TESTT`, `ADD SUB MUL ADDI DIV QUO REM MOD` (fixnum fast paths with
overflow checks, flonum fast paths, everything else cold), `NUMEQ LT GT
LE GE` and their compare-with-constant forms `NUMEQI LTI GTI LEI GEI`,
`ZEROP EQ EQV NOT NULLP PAIRP SYMBOLP NUMBERP FIXNUMP STRINGP VECTORP CAR
CDR CONS SETCAR SETCDR VREF VSET`, `CALL CALLSELF PRIMCALL TAILCALL RET`,
and 29 superinstructions picked from `--vm-stats` over the benchmarks
(compare-and-branch pairs, `MOVE_MOVE`, `MOVE_JMP`, `CDR_CDR`, `ADDI_CALL`,
...).

## Escape continuations; why no full call/cc, dynamic-wind or syntax-rules

`call/cc` is handled in `call_slow`, like `apply`. It allocates a
continuation, a closure of a fixed prototype (`vm.cont_proto`) whose one
free slot is a frame depth, and pushes a frame of that prototype (or, for
a tail call, replaces the current frame) with R0 = the procedure and R1 =
the continuation. The prototype's code is `CALL R0 1; RET R0`: a non-tail
call, so the frame stays where it is while the procedure runs, and a normal
return passes the result straight through. Invoking the continuation
(its `fixed` arity never matches, so it always reaches `call_slow`) checks
that the frame at its depth still holds this very continuation as its
closure, which is true exactly while that `call/cc` is active, and then
returns the argument from that frame, dropping everything above it. Both
the capture and the escape are O(1), and ordinary calls pay nothing.

The rest is left out to keep the core small; none of it is blocked by the
language.

- **Re-entrant `call/cc`.** Frames live on an explicit frame stack and
  registers in an explicit value stack, never on the C stack (natives do
  not call back into the VM), so a full continuation can be captured by
  copying both stacks into a heap object, and invoking it would copy them
  back and `jump` to the saved `resume` address. That is the simple "stack
  copying" strategy; a segmented stack (as in Chez) would make capture
  O(1).
- **`dynamic-wind`** needs a wind list in the context, and continuation
  invocation would run the `after`/`before` thunks between the current and
  target wind lists. Running a thunk from the middle of a continuation
  switch means re-entering the VM from a native (`uses reenter`) or
  writing the switch as a small Scheme procedure in the prelude.
- **`syntax-rules`** is a front-end feature: a pattern matcher and
  template expander running on s-expressions before the build pass, plus
  hygiene (renaming introduced bindings, which the `gensym` helper used by
  `do` and `case` already does in a limited way). It does not touch the VM.

## Benchmarks

The benchmarks are the public
[r7rs-benchmarks](https://github.com/ecraven/r7rs-benchmarks) suite, used
unmodified and downloaded at a pinned commit, and the reference is
chibi-scheme 0.12 (a bytecode VM) built from source; see
[`bench/scheme/README.md`](../../bench/scheme/README.md) for how they are
run, the inputs, all results and the skipped programs. 41 of the 57
programs the suite's `bench` script runs are included; of the other 16,
equal, pi, chudnovsky, mbrotZ and bv2string would need a sharing-aware
`equal?`, bignums, complex numbers or bytevectors, and 11 need file ports.

Geometric mean of ours / reference over the 41 programs (wall time,
whole process; Intel i7-6700, 2026-11-02): Gauche 0.29, Guile 3 without
its JIT 0.47, Guile 3 with its JIT 1.02 (chibi: 0.10 before the compiler
passes). Ours is faster than Gauche and than Guile's bytecode VM on every
program; the per-program tables, the timed-loop times and the changelog
of the optimizations are in the benchmark README.

The programs this directory used before were reproduced from memory by
the interpreter's author; they were removed in favour of the public suite.
