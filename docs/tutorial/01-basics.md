# 1. Loimi in ten minutes

Loimi's core is a small statically typed language at roughly C's level:
structs, tagged unions, pointers, slices, no garbage collector for host
data and no hidden allocation. Everything interpreter-specific (the `vm`
block, `value` types, `object`s, effects) builds on it. This chapter is
the core, by example. The full program is
[`examples/tutorial/step1/main.lm`](../../examples/tutorial/step1/main.lm).

```sh
./target/release/loimi run examples/tutorial/step1/main.lm
```

`loimi run` compiles to LLVM IR, hands it to clang, and runs the result.
`loimi build FILE -o OUT` keeps the executable; add `--release` for
`-O3` without overflow checks. Arguments after `--` go to the program.

## Functions and bindings

```loimi
fn fib(n: i64) -> i64 {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}
```

Blocks are expressions, so `if` returns a value and the last expression
of a function body is its result. Parameter and return types are
written out; local types are inferred:

```loimi
    let n = 20;
    var total = 0;
    var i = 0;
    while i < 10 { total += i; i += 1; }
```

`let` is immutable and `var` mutable. Integer arithmetic traps on
overflow in debug builds and wraps in release builds; the explicit forms
are `a +% b` (wrapping) and `a.add_checked(b)`, which returns an option:

```loimi
    let big: i32 = 2147483647;
    match big.add_checked(1) {
        Some(v) => io.print_int(v as i64),
        None => io.print("overflow"),
    }
```

Checked arithmetic lowers to LLVM's `*.with.overflow` intrinsics, an add
and a `jo`. You will see it again in every arithmetic handler of the
interpreter, where it is how a 32-bit integer overflows into a float.

## Structs, methods and modules

Step 1 has a second file, `shapes.lm`. A file is a module; `pub` items
are visible to files that import it:

```loimi
/// A plain struct. Fields are laid out like a C struct.
pub struct Point { x: f64, y: f64 }

/// A method is a function whose name is qualified by the type. The first
/// parameter is the receiver; `p.norm2()` passes `&p` automatically.
pub fn Point.norm2(p: *Point) -> f64 { p.x * p.x + p.y * p.y }
```

```loimi
import std.io;
import std.mem;
import std.text;
import shapes;
import shapes.{Point, Shape};
```

`import shapes;` makes `shapes.area` available; `import
shapes.{Point, Shape}` brings names in unqualified. Modules are found
next to the importing file, and `std.*` in the standard library.

Pointers are `*T`. Field access goes through them without `->`, and
there is no borrow checker: host memory is managed by hand (`mem.alloc`,
`mem.release`) or, much more often in an interpreter, by an arena that
frees a whole phase's data at once.

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

Variants are written `Shape.Rect(2.0, 3.5)`, or `.Circle(1.0)` where the
type is known. Patterns nest, and work on tuples, which is how an
interpreter matches two operands at once (chapter 4).

## Options, results and slices

`?T` is an option (a `?*T` is a nullable pointer, one word). `[]T` is a
slice: a pointer and a length, bounds-checked.

```loimi
fn find(xs: []i64, want: i64) -> ?u64 {
    for i in 0..xs.len {
        if xs[i] == want { return Some(i); }
    }
    None
}
```

Errors are values. `Result<T, E>` with `?` returns early on `Err`:

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

There are no exceptions in host code. The interpreter's guest exceptions
are a vm feature (`raise`, chapter 4).

`if let`, `let ... else` and `while let` destructure options and enums:

```loimi
    let xs = [5, 7, 9, 11];
    if let Some(at) = find(xs[..], 9) { io.print("9 is at "); io.print_int(at as i64); io.println(""); }
    if find(xs[..], 4).is_none() { io.println("4 is missing"); }
```

## The standard library

`std.io` prints (`print`, `println`, `print_int`, `print_f64`, and the
`eprint` forms for stderr: `eprint`, `eprintln`, `eprint_int`,
`eprint_f64`) and reads files; `std.os` has the command
line; `std.mem` has `Vec<T>` and `Arena`; `std.text` has `StrBuf` and an
`Interner`:

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

Generic types are monomorphized, like C++ templates or Rust generics.
There are no destructors; `defer x.free();` runs at the end of the scope
if you want the cleanup next to the allocation.

The rest of the standard library is aimed at interpreters: `std.lex`,
`std.parse`, `std.diag`, `std.num`, `std.bc`, `std.gc`, `std.obj`. The
next chapters use most of them. [`docs/stdlib.md`](../stdlib.md)
describes each one: first the API that exists, then a "Design (not
built)" part you should not plan around.

## What you get, and what you don't

The core language has generics, static traits (bounds checked when a
generic is instantiated), function pointers, `defer`, `inline for`
(chapter 4) and module-level `var`s. It does not have closures,
destructors, exceptions or a borrow checker. That is deliberate: the
complexity budget goes to the parts of an interpreter that C makes hard,
which start in chapter 3.

Next: [a calculator](02-calculator.md).
