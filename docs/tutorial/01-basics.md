# 1. Loimi in ten minutes

Loimi's core is a small statically typed language at about the level of
C: structs, tagged unions, pointers and slices, with no garbage
collector for your own data and no hidden allocation. The interpreter
features in later chapters are built on top of it. This chapter shows
the core by example.

The full program is
[`examples/tutorial/step1/main.lm`](../../examples/tutorial/step1/main.lm).
Run it with:

```sh
./target/release/loimi run examples/tutorial/step1/main.lm
```

The commands you will use:

| Command | What it does |
|---|---|
| `loimi run FILE` | Compiles to LLVM IR, builds it with clang, and runs the result |
| `loimi build FILE -o OUT` | Builds an executable and keeps it |
| `--release` | Builds with `-O3` and without overflow checks |
| `-- ARGS` | Passes the arguments after `--` to the program |

## Functions and bindings

```loimi
fn fib(n: i64) -> i64 {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}
```

Blocks are expressions. So `if` returns a value, and the last
expression of a function body is its result.

You write out the types of parameters and return values. The types of
local variables are inferred:

```loimi
    let n = 20;
    var total = 0;
    var i = 0;
    while i < 10 { total += i; i += 1; }
```

`let` makes an immutable binding, and `var` a mutable one.

## Integer overflow

Integer arithmetic traps on overflow in debug builds and wraps in
release builds. When you want a specific behavior, say so:

- `a +% b` always wraps.
- `a.add_checked(b)` returns an option, which is `None` on overflow.

```loimi
    let big: i32 = 2147483647;
    match big.add_checked(1) {
        Some(v) => io.print_int(v as i64),
        None => io.print("overflow"),
    }
```

A checked add compiles to an add followed by a jump on overflow, so it
is cheap. You will use it in every arithmetic op of the interpreter:
it is how a 32-bit integer overflows into a float.

## Structs, methods and modules

Step 1 has a second file, `shapes.lm`. Each file is a module, and items
marked `pub` are visible to files that import it:

```loimi
/// A plain struct. Fields are laid out like a C struct.
pub struct Point { x: f64, y: f64 }

/// A method is a function whose name is qualified by the type. The first
/// parameter is the receiver; `p.norm2()` passes `&p` automatically.
pub fn Point.norm2(p: *Point) -> f64 { p.x * p.x + p.y * p.y }
```

`main.lm` imports it:

```loimi
import std.io;
import std.mem;
import std.text;
import shapes;
import shapes.{Point, Shape};
```

- `import shapes;` lets you write `shapes.area`.
- `import shapes.{Point, Shape}` lets you write `Point` and `Shape`
  without the module name.

The compiler looks for modules next to the importing file, and for
`std.*` in the standard library.

Pointers are written `*T`. You access fields through a pointer with `.`,
as with a struct (there is no `->`). There is no borrow checker. You
manage memory by hand with `mem.alloc` and `mem.release`, or, much more
often in an interpreter, with an arena that frees all of one phase's
data at once.

## Enums and `match`

```loimi
/// An enum is a tagged union: each variant can carry a payload.
pub enum Shape {
    Circle(f64),
    Rect(f64, f64),
    Empty,
}

/// `match` is exhaustive: leave out a variant and the compiler says so.
pub fn area(s: Shape) -> f64 {
    match s {
        Circle(r) => 3.0 * r * r,
        Rect(w, h) => w * h,
        Empty => 0.0,
    }
}
```

You write a variant as `Shape.Rect(2.0, 3.5)`, or as `.Circle(1.0)`
where the type is already known. Patterns can be nested, and can match
tuples. Matching a tuple is how the interpreter will look at two
operands at once (chapter 4).

## Options and slices

`?T` is an option: either `Some(value)` or `None`. A `?*T` is a pointer
that may be null, and takes one word.

`[]T` is a slice: a pointer and a length. Indexing a slice is
bounds-checked.

```loimi
fn find(xs: []i64, want: i64) -> ?u64 {
    for i in 0..xs.len {
        if xs[i] == want { return Some(i); }
    }
    None
}
```

`for i in a..b` counts from `a` up to `b - 1`. When you don't need the
index, `for x in xs` loops over the elements of an array or slice:
`for x in xs { s += x; }`.

`if let`, `let ... else` and `while let` take apart options and enums:

```loimi
    let xs = [5, 7, 9, 11];
    if let Some(at) = find(xs[..], 9) { io.print("9 is at "); io.print_int(at as i64); io.println(""); }
    if find(xs[..], 4).is_none() { io.println("4 is missing"); }
```

## Errors

Errors are ordinary values. A function that can fail returns
`Result<T, E>`, which is `Ok(value)` or `Err(error)`. Writing `?` after
a call returns early if the call returned `Err`:

```loimi
fn digit(c: u8) -> Result<i64, str> {
    if c >= '0' and c <= '9' { Ok((c - '0') as i64) } else { Err("not a digit") }
}

fn parse_number(s: str) -> Result<i64, str> {
    var n: i64 = 0;
    for i in 0..s.len {
        n = n * 10 + digit(s[i])?;
    }
    Ok(n)
}
```

Loimi has no exceptions. Errors in the guest language, the one your
interpreter runs, are handled by a separate feature (chapter 4).

## The standard library

The modules this chapter uses:

| Module | What it has |
|---|---|
| `std.io` | Printing to stdout (`print`, `println`, `print_int`, `print_f64`) and stderr (`eprint`, `eprintln`, `eprint_int`, `eprint_f64`), and reading files |
| `std.os` | The command line |
| `std.mem` | `Vec<T>` (a growable array) and `Arena` |
| `std.text` | `StrBuf` (a growable string) and `Interner` |

```loimi
    var v: mem.Vec<i64> = mem.Vec.new();
    for k in 0..5 { v.push(k * k); }
    var b = text.StrBuf.new();
    b.push_str("squares:");
    for k in 0..v.len { b.push(' '); b.push_int(v.at(k)); }
    io.println(b.as_str());
    b.free();
    v.free();
```

Generic types are compiled separately for each type they are used with,
like C++ templates or Rust generics.

There are no destructors, so you free things yourself. To keep the
cleanup next to the allocation, write `defer x.free();`: it runs at the
end of the scope.

The rest of the standard library is aimed at interpreters, and the
next chapters use most of it. [`docs/stdlib.md`](../stdlib.md)
describes every module.

## What else is in the core

The core also has these, which this chapter doesn't show:

- generics, with static traits as bounds (checked when the generic is
  used with a type)
- function pointers
- `inline for`, which repeats code once per item at compile time
  (chapter 4)
- module-level `var`s
- closures (`|x| x + 1`), passed as a `block fn` argument, or kept as a
  `closure fn` allocated in an arena or the GC heap (the tutorial does
  not need them)

The core has no destructors, exceptions or borrow checker. That is on
purpose: the complexity goes into the parts of an interpreter that C
makes hard. Those start in chapter 3.

Next: [a calculator](02-calculator.md).
