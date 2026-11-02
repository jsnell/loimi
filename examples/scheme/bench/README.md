# Scheme benchmarks

Classic, public Scheme benchmark programs, from the Gabriel benchmarks
(R. P. Gabriel, *Performance and Evaluation of Lisp Systems*, MIT Press, 1985)
as carried into the r7rs-benchmarks suite (github.com/ecraven/r7rs-benchmarks),
and the Kernighan–Van Wyk set. They were reproduced from those well-known
sources; only the driver loops (`run`) and the input sizes were adapted so
that each program runs about a second. Each file ends with its expected
output as `; expect:` lines.

| Program | Origin | What it exercises |
|---|---|---|
| `fib` | r7rs-benchmarks `fib` (fib 32 = 2178309) | non-tail calls, fixnum arithmetic |
| `fibfp` | r7rs-benchmarks `fibfp` | the same on flonums |
| `tak` | Gabriel `TAK` (tak 18 12 6 = 7), r7rs `tak` (tak 32 16 8 = 9) | calls |
| `cpstak` | r7rs-benchmarks `cpstak` | tail calls, closure creation |
| `takl` | Gabriel `TAKL` | list walking, `and`/`or` |
| `ack` | r7rs-benchmarks `ack` (ack 3 9 = 4093, ack 3 10 = 8189) | deep non-tail recursion |
| `nqueens` | r7rs-benchmarks `nqueens` (8 → 92, 11 → 2680) | lists, `append`, internal defines |
| `deriv` | Gabriel `DERIV` | symbols, `map` with closures, allocation |
| `destruc` | Gabriel `DESTRUCTIVE` | `set-car!`/`set-cdr!`, `do` loops |
| `div` | Gabriel `DIV` (iterative and recursive) | `cddr`, allocation |
| `primes` | r7rs-benchmarks `primes` | `letrec`, `modulo`, allocation |
| `sum` | r7rs-benchmarks `sum` | a tight named-`let` loop |
| `array1` | Kernighan–Van Wyk `array1` | vectors, `do` loops |
| `mbrot` | r7rs-benchmarks `mbrot` | flonum arithmetic, vectors |

No reference Scheme is installed here. Expected outputs are known results
(fib, tak, ack, nqueens, primes) or were cross-checked against a direct
Python transcription of the program (destruc, mbrot, tak 32 16 8).

Run them with:

```sh
examples/scheme/bench/run.sh                     # all, default (tail-call) dispatch
examples/scheme/bench/run.sh --dispatch=goto     # extra flags go to `loimi build`
examples/scheme/bench/run.sh -- fib tak          # a subset
```
