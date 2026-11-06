# `std.math`: division by a constant

It expands the `std.math` row of [`../stdlib.md`](../stdlib.md). The
motivation and measurements are item 3 of
[`../design/speed-audit.md`](../design/speed-audit.md#3-division-by-a-verified-constant).

A 64-bit `idiv` takes 40–90 cycles on Skylake; a multiply and a shift
take about 4. When an interpreter knows a divisor before the op runs
(`X mod 10` in Prolog, `i % 3` in Lua or JS), it can prepare the divisor
once and divide by multiplying. `math.Divisor` is that prepared divisor
(Granlund and Montgomery, PLDI 1994, as libdivide does it), sized to sit
in an inline cache.

## As implemented

`std/math.lm`, tested by `tests/run/std_math.lm`: every divisor 1..1100
against every dividend in -1100..1100; for those divisors and for
powers of two, their neighbours and 3000 random divisors up to 2^32 - 1,
edge dividends (0, ±1, ±2^31, ±2^47, ±(2^63 - 1), multiples of the
divisor and their neighbours, the largest multiple below 2^63) and
random dividends of every magnitude, each against `/` and `%`; and the
cache layout in a vm.

| | |
|---|---|
| `math.Divisor.new(d: i64) -> ?Divisor` | prepare `d`; None unless 1 <= d < 2^32. Costs two 64-bit divisions (no call): do it when emitting the op, or once when it first runs, not every time |
| `dv.set(d: i64) -> bool` | the same, in place (a cache filled when its op first runs) |
| `dv.store(w: []u32)` | write the 4 words to `w[0..4]` (an emitter filling the op's cache words) |
| `dv.divisor() -> i64` | `d` |
| `dv.udiv(n: u64)`, `dv.urem(n: u64)` | floor(n / d) and n mod d, for n < 2^63 |
| `dv.udiv31(n: u64)`, `dv.urem31(n: u64)` | the same for n < 2^31 (non-negative i32 values) with a 64-bit `imul` instead of the 128-bit product |
| `dv.div_trunc(x: i64)`, `dv.rem_trunc(x: i64)` | toward zero, the remainder with the sign of x: C and JS `/` `%`, Prolog `//` and `rem` |
| `dv.div_floor(x: i64)`, `dv.rem_floor(x: i64)` | floor, the remainder in 0..d: Lua and Python `//` `%`, Prolog `div` and `mod` |
| `math.DIVISOR_WORDS` | 4, the words a `Divisor` takes as a cache in a u32 vm |

The methods take `*Divisor` and are `#[inline(always)]`.

**Domain.** Divisors are positive and below 2^32. Dividends of the
signed methods are any `x` with |x| < 2^63: every i32 and i48 payload,
every i64 except i64.min. The unsigned ones take n < 2^63. Negative and
zero divisors are the caller's slow path: an interpreter emits its
constant-divisor op only when `Divisor.new` gives one, or tests the
constant's sign on the fast path (MiniJS, below).

**The arithmetic.** With l = ceil(log2 d) and m = ceil(2^(63+l) / d)
(64 bits for every d >= 1), floor(n / d) = hi(2n · m) >> l: `add`,
`mul` (the 128-bit product), `shr`. The error m·d - 2^(63+l) is below
d <= 2^l, so n·(m·d - 2^(63+l)) < 2^(63+l) for n < 2^63, Granlund and
Montgomery's condition (theorem 4.2) for 63-bit dividends. The signed
methods divide |x| and fix the sign with selects (no branches); the
remainder is n - q·d. The 31-bit forms use m' = floor(m / 2^32) + 1 (at
most 2^32, so n·m' < 2^63) and shift by 31 + l: m'·d - 2^(31+l) <= d,
and n·d < 2^(31+l) for n < 2^31. They are what a handler on i32 values
wants: Mini's measured 2% slower on closures with the 128-bit form.

**Layout.** The fields are four `u32` words (`m_lo`, `m_hi`, `d`, `l`),
so the struct is an inline-cache layout in any vm with words of 32 bits
or more, and `&cache` is a `*Divisor` (language.md §7.1). LLVM merges
the two halves of m into one 64-bit load. Zeroed words (a cache not yet
filled) hold `d == 0`: test `cache.d` where an op can run before its
cache is filled.

### A constant-divisor op

The emitter prepares the divisor and stores it in the op's cache words;
the op is one line:

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
cache DivK = math.Divisor;
op MODK(dst: reg, a: reg) : AB + DivK { dst = (&cache).rem_floor(a); next }
op IDIVK(dst: reg, a: reg) : AB + DivK { dst = (&cache).div_trunc(a); next }
```

With `std.bc`, `let at = code.op(w);` then
`dv.store(code.words()[at + 1..at + 5])`.

### In the examples

- **Prolog** (`examples/prolog`): `X // K`, `X mod K`, `X div K` and
  `X rem K` with an integer constant 1 <= K < 2^31 compile to `IDIVK`,
  `MODK`, `DIVK` and `REMK`, whose `DivK` cache the clause compiler
  fills. The four ops are one `inline for` row each:
  `d = Term.Int((&cache).#method(x as i64) as i48)`. Every i48
  dividend, negative ones included, takes the fast path; bignums and
  errors go to the evaluator. Before, these were `R3` ops dividing by a
  register (`u32` division when both operands were small and
  non-negative, else a 64-bit `idiv`).
- **Mini** (`examples/mini`): `MODK` and the register `MOD` keep a
  `math.Divisor` cache filled when the op first runs with a positive
  divisor (`(&cache).set(y)`, in a `cold` block so that the two
  divisions do not give the handler a stack frame); `MOD` keys it by
  `cache.d`. The hot path is `(&cache).urem31(x)`. They replace the
  hand-written `mod_magic` (a 31-bit magic and shift packed in a
  `u64`).
- **MiniJS** (`examples/minijs`): `MODK` with an Int constant gets its
  `ModK` cache from the emitter. Its fast path checks
  `y > 0 and (y as u32) == cache.d`, so an unfilled cache (or a
  constant <= 0) goes to the `idiv` and float paths as before; integral
  floats below 2^53 use `rem_trunc` too.
- **Python** was not ported: its slice `idiv` divides by the slice
  step, a run-time value, and steps of ±1 already skip the division.

## Not built

- **`const` cache fields** (`roadmap.md` Q10): fields that the emitter
  sets, that `Vm.deopt` keeps and the verifier may read. Today a
  divisor cache is an ordinary cache: `Vm.deopt` zeroes it, so an op
  whose cache an emitter fills must not be deoptimized (none of the
  examples deoptimizes these ops), or must test `cache.d` and refill as
  Mini does.
- A verify-time hook that fills a cache from a constant operand.
- Negative divisors, divisors of 2^32 and more, and i64.min / u64
  dividends of 2^63 and more (libdivide's 65-bit "add" form would cover
  them at two more instructions).
