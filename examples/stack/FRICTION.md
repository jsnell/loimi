# Stak in Loimi: a usability study

This is a report on writing Stak, a Forth whose stack use is checked
before it runs (see `SPEC.md`), in Loimi. It is for people improving
Loimi's usability, or curious how usable it is.

A first-time user (an AI agent) wrote it from the README, `docs/`,
`std/`, `examples/tutorial/` and a few files in `tests/run`. It did not
read the compiler's source or the other example interpreters.

## Summary

- **It worked the first time.** The first complete build (about 1,100
  lines) compiled without errors and ran the first test correctly. A
  working, tested interpreter took about 25 minutes; everything,
  including optimization, a fuzzer and this report, took about 50.
- **The main gap was stack checking.** The docs described stack
  effects for ops, but they weren't built, so the author wrote the
  stack checking by hand in three places. Loimi now has declared stack
  effects.
- **Super instructions and a cached top of stack paid off**: together
  roughly 20–35% faster than the first version.
- **Almost every issue has been fixed** since the study.

## Issues

Most important first.

| # | Issue | State |
|---|---|---|
| 1 | Ops couldn't declare stack effects, so the verifier didn't check the stack. Each op's effect was written in three places, plus a 50-line checker. | Fixed. `op X() : F stack(a b -- c)`, checked by the generated verifier, with `max_stack()`. Stak declares each effect once. |
| 2 | A `verify` block saw super instructions in optimized code, which wasn't documented. | Fixed. `verify` blocks see the code with supers split and family members generic. |
| 3 | A `verify` block could only fail, without saying where. | Fixed. `reject(at, msg)`. |
| 4 | `optimize` fused supers greedily from left to right, and `--vm-stats` suggested only pairs. Stack code has many overlapping pairs. | Fixed. `--vm-stats` suggests 3- and 4-op sequences; `optimize` chooses fusions by dynamic programming. |
| 5 | `stdlib.md` mixed design-only features with built ones. | Fixed. The stdlib docs describe only what exists. |
| 6 | No string formatting; building messages was verbose. | Fixed. `text.Fmt`, `b.fmt("{} {}", a, b)`. |
| 7 | No string patterns in `match`. | Fixed. |
| 8 | Struct literals needed every field, and there was no empty-slice literal. | Fixed. `..zero` fills fields where zero is valid, and `[]` is the empty slice. |
| 9 | `Vm.opcode(w)` and encoder parameter types were missing from the list of generated functions. | Fixed. language.md lists the generated API. |
| 10 | A traced value cached in a state register had to be stored in `sync` and marked in the root scanner by hand. | Fixed. The compiler roots traced state variables at every call that may collect. |
| 11 | A fuzzer run grew to 30 GB: a mutant doubled a string in a loop, which a step budget doesn't stop. | Fixed in the tutorial's fuzzing chapter (`ASAN_OPTIONS=hard_rss_limit_mb=..`). |
| — | `std.lex` has no `Cursor.word()` for a run of non-space bytes. | Open. |
| — | `bc.Pool` has no way to take its items and free the index. | Open. |
| — | It wasn't clear that two entries may share code through a `-> never` vm function. It works. | Open as of the study. |

## What worked well

- **The tutorial is a good template.** Step 8 was close enough that a
  different VM shape was mostly a matter of changing the ops.
- **A `Value` as a state register.** Caching the top of stack was a
  mechanical edit (`*sp` becomes `tos`). The generated `ADD` keeps it in
  a register, with no spills, in about 30 instructions.
- **`--vm-stats`** prints ready-to-paste `super` lines, and the
  `super_overlap` warning says exactly which sequence loses.
- **The verifier, `verify` blocks and `variant`** made a bytecode fuzzer
  a 200-line file. It found two real bugs in its first minute.
- **Diagnostics.** `std.diag` gave Stak compile errors with snippets
  and carets for free. Debug builds trap integer overflow with a source
  location.
- **One flag each** for the three dispatch modes and GC stress;
  `check.sh` runs the whole matrix in about 30 seconds.

## Benchmarks

Release builds, best of 5, seconds, on a shared machine. `v1` is the
first working version; `final` adds super instructions and a cached top
of stack. CPython 3.13 runs the same algorithms, for scale only.

| benchmark | what | v1 | final (tailcall) | goto | switch | CPython 3.13 |
|---|---|---:|---:|---:|---:|---:|
| fib | `32 fib`, 7M calls | 0.085 | 0.052 | 0.058 | 0.062 | 0.35 |
| loop | 3 × 30M iterations of int and float `do` loops | 0.937 | 0.708 | 0.793 | 0.729 | 1.95 (first third only) |
| sieve | 30 × sieve(100000) | 0.117 | 0.087 | 0.098 | 0.095 | 0.49 |
| strings | string building, 3M short arrays | 0.145 | 0.135 | 0.135 | 0.132 | |
| nbody | 5M steps of float locals through an array | 0.185 | 0.153 | 0.181 | 0.171 | |

What paid off:

- super instructions suggested by `--vm-stats`: 8–20%
- caching the top of stack in a state register: another 10–15%
- 3- and 4-op supers for stack idioms such as `dup 2 < if`: 5–12% on
  `fib` and `sieve`

## Size and tests

1,407 lines of Loimi: `vm.lm` 588, `compile.lm` 586, `main.lm` 127,
`value.lm` 106, plus a 202-line fuzzer.

`check.sh` runs 15 programs in all three dispatch modes, normally and
with `--gc-stress`. The fuzzer ran 25,000 mutated programs under
AddressSanitizer, plus 3,000 with GC stress, with no crashes after its
two findings were fixed.
