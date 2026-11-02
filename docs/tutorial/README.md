# Writing a fast interpreter in Loimi

This tutorial builds a small dynamically typed language, **Pip**, from an
empty file to a bytecode interpreter with NaN-boxed values, inline
caches, quickening, a garbage collector and a bytecode fuzzer. Every
stage is a complete program in [`examples/tutorial/`](../../examples/tutorial),
and every stage has tests.

It is written for someone who knows C or Rust and may have read an
interpreter or two (Lua, CPython, a Scheme), but has not written one in
Loimi. Each chapter introduces the Loimi features the interpreter needs
at that point and says what they replace: the macros, `Protect()` calls,
computed gotos and hand-written disassemblers that C interpreters are
built from.

## Contents

1. [Loimi in ten minutes](01-basics.md): functions, structs, enums,
   `match`, options and results, modules, `std.io`, `loimi run`.
2. [A calculator](02-calculator.md): a lexer from `std.lex`, a Pratt
   parser from `std.parse`, and a tree-walking evaluator.
3. [A first vm](03-first-vm.md): `vm`, `state`, operands, a fixed
   encoding, ops, `next`, entries, and what the compiler generates from
   them: encoders, the disassembler and the verifier, whose
   `Verified<Calc>` is the only code `run` accepts. Compiling to
   bytecode with `std.bc`.
4. [Values, slow paths and errors](04-values.md): `value ... repr
   nanbox`, matching on `(Int, Int)`, `cold` blocks, `inline for`,
   `raise` and `on raise`.
5. [Variables and control flow](05-variables.md): locals in registers,
   globals behind an inline cache (`cache`, `family`, `become`),
   compare-and-branch with `then JMP`.
6. [Functions and calls](06-functions.md): prototypes, frames, CALL and
   RET, register windows, verify clauses that look at the instruction.
7. [Heap objects and the GC](07-gc.md): `object`, `Gc<T>`, root
   scanners, effects (`uses gc`, `sync`), and GC stress testing.
8. [Making it fast](08-fast.md): measuring, `--dispatch`, reading the
   generated code, constant operands, float quickening, `--vm-stats` and
   `super`, and the checklist from Mini's performance log.
9. [Testing an interpreter](09-testing.md): conformance tests across
   dispatch modes and GC stress, a bytecode fuzzer with a step-budget
   `variant`, and clean-room benchmarks.

## The language

Pip, at the end of the tutorial:

```text
fn fib(n) {
    if n < 2 { return n; }
    return fib(n - 1) + fib(n - 2);
}
print fib(20);

let words = ["one", "two", "three"];
let i = 0;
while i < len(words) {
    print "word " + i + ": " + words[i];
    i = i + 1;
}
```

It has integers (32-bit, overflowing into floats), floats, booleans,
`nil`, strings, fixed-size arrays and functions; global variables
(`let` at the top level), local variables (`let` in a block, and
parameters), `if`/`else`, `while`, `fn`, `return`, `print`, and two
built-ins, `len(x)` and `array(n)`. It has no closures, no `and`/`or`,
and no `for`; those are good exercises once you have finished.

## Running the steps

You need the Loimi compiler (`cargo build --release` at the top of the
repository) and clang 19. Each step is a directory with a `main.lm`:

```sh
./target/release/loimi run examples/tutorial/step7/main.lm -- examples/tutorial/step7/tests/arrays.pip
./target/release/loimi build examples/tutorial/step9/main.lm --release -o pip
./pip examples/tutorial/step9/tests/functions.pip
```

`examples/tutorial/check.sh` builds every step (in all three dispatch
modes for the steps that have a vm) and runs its tests; `cargo test` runs
it, and also checks that every Loimi snippet in these chapters appears in
the step files.

| Step | Directory | New in this step |
|---|---|---|
| 1 | `step1` | the core language |
| 2 | `step2` | lexer, parser, tree evaluator |
| 3 | `step3` | the `Calc` vm, `std.bc` |
| 4 | `step4` | the `Pip` vm, `Value`, errors |
| 5 | `step5` | variables, `if`, `while` |
| 6 | `step6` | functions |
| 7 | `step7` | strings, arrays, the GC |
| 8 | `step8` | optimizations, benchmarks |
| 9 | `step9` | the fuzzer |

## What is not covered

Loimi's design ([`docs/language.md`](../language.md)) describes more
than is built ([`docs/status.md`](../status.md)). This tutorial uses only
what is implemented. Closures in Loimi itself, `suspend`,
C-ABI entry points and the generated `Builder` do not exist yet, and
nothing here pretends otherwise.
