# `std.math`: division by a constant

`math.Divisor` makes division by a constant fast. This page is for
interpreter authors who want to speed up ops like `X mod 10` or
`i % 3`.

A 64-bit `idiv` instruction takes 40–90 cycles on Skylake; a multiply
and a shift take about 4. When an interpreter knows the divisor before
the op runs, it can prepare the divisor once and then divide by
multiplying. A `Divisor` is that prepared divisor, sized to fit in an
inline cache. The method is Granlund and Montgomery's (PLDI 1994), as
used by libdivide.

## API

| | |
|---|---|
| `math.Divisor.new(d: i64) -> ?Divisor` | Prepare `d`. `None` unless 1 <= d < 2^32. Costs two 64-bit divisions, so do it when emitting the op, or once when it first runs, not every time. |
| `dv.set(d: i64) -> bool` | The same, in place (for a cache filled when its op first runs) |
| `dv.store(w: []u32)` | Write the 4 words to `w[0..4]` (for an emitter filling the op's cache words) |
| `dv.is_valid() -> bool` | Whether the words are what `Divisor.new(dv.divisor())` would prepare. For the `verify` clause of a `const` cache; costs what `new` does. |
| `dv.divisor() -> i64` | `d` |
| `dv.udiv(n: u64)`, `dv.urem(n: u64)` | floor(n / d) and n mod d, for n < 2^63 |
| `dv.udiv31(n: u64)`, `dv.urem31(n: u64)` | The same for n < 2^31 (non-negative `i32` values), with a cheaper 64-bit multiply instead of a 128-bit one |
| `dv.div_trunc(x: i64)`, `dv.rem_trunc(x: i64)` | Rounding toward zero, remainder with the sign of x: C and JS `/` `%`, Prolog `//` and `rem` |
| `dv.div_floor(x: i64)`, `dv.rem_floor(x: i64)` | Rounding down, remainder in 0..d: Lua and Python `//` `%`, Prolog `div` and `mod` |
| `math.DIVISOR_WORDS` | 4: the number of words a `Divisor` takes as a cache in a vm with `u32` words |

The methods take `*Divisor` and are `#[inline(always)]`.

**Domain.**

- Divisors must be positive and below 2^32.
- The signed methods take any `x` with |x| < 2^63: every `i32` and
  `i48` payload, and every `i64` except `i64.min`.
- The unsigned methods take n < 2^63.

Negative and zero divisors are the caller's slow path. Either emit the
constant-divisor op only when `Divisor.new` returns one, or test the
divisor's sign on the fast path.

**Layout.** A `Divisor` is four `u32` words (`m_lo`, `m_hi`, `d`, `l`),
so it works as an inline-cache layout in any vm with words of 32 bits or
more, and `&cache` is a `*Divisor`. Zeroed words (a cache not yet
filled) have `d == 0`: test `cache.d` where an op can run before its
cache is filled.

## A constant-divisor op

The emitter prepares the divisor and stores it in the op's cache words.
The cache is `const`: handlers only read it, `Vm.deopt` keeps it, and
the verifier checks it with `is_valid`. Each op is one line:

<!-- snippet: run "7 -14"
import std.io;
import std.math;
vm V {
    state { pc: *u32, regs: *i64 }
    operand reg = regs[it];
    encoding fixed(u32) { format AB { op: 8, a: 8, b: 8 } }
{snippet}
    op RET(a: reg) : AB { exit a }
    entry run(code: *u32, r: *i64) -> i64 { regs = r; jump code }
}
fn main() {
    var code: [u32; 16] = [0; 16];
    var at = V.emit.MODK(code[..], 0, 2, 0);       // r2 = r0 mod 10
    let Some(ten) = math.Divisor.new(10) else { trap() };
    ten.store(code[1..5]);
    at = V.emit.IDIVK(code[..], at, 3, 1);          // r3 = r1 // 7
    let Some(seven) = math.Divisor.new(7) else { trap() };
    seven.store(code[at - 4..at]);
    at = V.emit.RET(code[..], at, 2);
    var r: [i64; 4] = [-123, -100, 0, 0];
    io.print_int(V.run(&code[0], &r[0])); io.print(" "); io.print_int(r[3]); io.println("");
}
-->
```loimi
cache DivK = const math.Divisor verify it.is_valid();
op MODK(dst: reg, a: reg) : AB + DivK { dst = (&cache).rem_floor(a); next }
op IDIVK(dst: reg, a: reg) : AB + DivK { dst = (&cache).div_trunc(a); next }
```

With `std.bc`, emit the op with `let at = code.op(w);`, then
`dv.store(code.words()[at + 1..at + 5])`.

**A cache filled at run time.** If the divisor is only known when the op
first runs, call `(&cache).set(y)` in a `cold` block, so that its two
divisions don't give the handler a stack frame. Mini's `MOD` op does
this and keys the cache by `cache.d`.

## In the examples

| Example | Ops | How |
|---|---|---|
| Prolog | `X // K`, `X mod K`, `X div K`, `X rem K` with a constant 1 <= K < 2^31 | `IDIVK`, `MODK`, `DIVK`, `REMK`, with a `const` cache that the clause compiler fills and the verifier checks. Every `i48` dividend, negative ones included, takes the fast path. |
| Mini | `MODK`, and `MOD` by a register | A cache filled when the op first runs with a positive divisor. The hot path is `(&cache).urem31(x)`. |
| MiniJS | `MODK` with an integer constant | A `const` cache filled by the emitter (zero words for other constants). The fast path checks `y > 0 and (y as u32) == cache.d`, so an unfilled cache goes to the slow path. |

Python doesn't use it: its only integer division in a hot path divides
by a slice step, which is not known ahead of time.

## How it works

With l = ceil(log2 d) and m = ceil(2^(63+l) / d), which fits in 64 bits
for every d >= 1:

floor(n / d) = hi(2n · m) >> l

That is an add, a 128-bit multiply and a shift. The error
m·d − 2^(63+l) is below d <= 2^l, which meets Granlund and Montgomery's
condition (theorem 4.2) for 63-bit dividends. The signed methods divide
|x| and fix the sign with selects, not branches; the remainder is
n − q·d.

The 31-bit forms use m' = floor(m / 2^32) + 1 (at most 2^32, so
n·m' < 2^63) and shift by 31 + l. That fits a 64-bit multiply, which is
what a handler on `i32` values wants: Mini measured 2% slower on
closures with the 128-bit form.

## Limitations

`Divisor` does not handle negative divisors, divisors of 2^32 or more,
or dividends of `i64.min` (or `u64` values of 2^63 and more).
libdivide's 65-bit "add" form would cover them at the cost of two more
instructions.
