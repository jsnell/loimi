# Writing a fast interpreter in Loimi

This tutorial builds a small dynamically typed language, **Pip**. It
starts from an empty file and ends with a fast bytecode interpreter that
has NaN-boxed values, inline caches, a garbage collector and a bytecode
fuzzer. Every step is a complete, tested program in
[`examples/tutorial/`](../../examples/tutorial).

It is for someone who knows C or Rust and may have read the source of
an interpreter or two (Lua, CPython, a Scheme), but has not used Loimi.
Each chapter introduces the Loimi features the interpreter needs at
that point, and says what they replace in a C interpreter.

## Contents

| Chapter | What you build | What you learn |
|---|---|---|
| 1. [Loimi in ten minutes](01-basics.md) | Small examples | The core language |
| 2. [A calculator](02-calculator.md) | A lexer, a parser and a tree-walking evaluator | `std.lex` and `std.parse` |
| 3. [A first vm](03-first-vm.md) | A bytecode compiler and vm for the calculator | The `vm` block, the generated encoders, disassembler and verifier, and `std.bc` |
| 4. [Values, slow paths and errors](04-values.md) | Dynamically typed values and run-time errors | `value` types, `cold` blocks, `inline for`, `raise` |
| 5. [Variables and control flow](05-variables.md) | Locals, globals, `if` and `while` | Inline caches, quickening, compare-and-branch |
| 6. [Functions and calls](06-functions.md) | Function calls | Frames, register windows, verifying calls |
| 7. [Heap objects and the GC](07-gc.md) | Strings and arrays | `object`, `Gc<T>`, roots, effects, GC stress testing |
| 8. [Making it fast](08-fast.md) | A faster interpreter | Measuring, dispatch modes, reading the generated code, superinstructions |
| 9. [Testing an interpreter](09-testing.md) | Unit tests and a bytecode fuzzer | `std.test`, variants, clean-room benchmarks |

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

Pip has:

- values: integers (32-bit, overflowing into floats), floats, booleans,
  `nil`, strings, fixed-size arrays and functions
- global variables (`let` at the top level) and local variables (`let`
  in a block, and parameters)
- `if`/`else`, `while`, `fn`, `return` and `print`
- two built-in functions, `len(x)` and `array(n)`

It has no closures, no `and`/`or` and no `for`. Those are good
exercises once you have finished.

## Running the steps

You need clang 19 and the Loimi compiler, which you build by running
`cargo build --release` at the top of the repository. Each step is a
directory with a `main.lm`:

```sh
./target/release/loimi run examples/tutorial/step7/main.lm -- examples/tutorial/step7/tests/arrays.pip
./target/release/loimi build examples/tutorial/step9/main.lm --release -o pip
./pip examples/tutorial/step9/tests/functions.pip
```

`examples/tutorial/check.sh` builds every step and runs its tests (in
all three dispatch modes, for the steps that have a vm). `cargo test`
runs it too, and also checks that every Loimi snippet in these chapters
appears in the step files.

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

These Loimi features are not used in the tutorial. The
[language reference](../language.md) and the
[standard library reference](../stdlib.md) describe them.

- closures in Loimi code
- embedding the interpreter in a C program (`loimi build --lib`)
- threads, and heaps shared between threads (`std.gil`)
- the generational GC mode
- resume points for coroutines (`Vm.Resume`)
- accumulator registers
- cross-compiling for AArch64 (`--target`)
- the copy-and-patch JIT (`--jit-stencils`)
