# A regex engine as a Loimi C library: a usability study

This is a report on writing a regex engine in Loimi and shipping it as
a C library. It is for people improving Loimi's usability, or curious
how usable it is.

A first-time user (an AI agent) built two engines over the same
bytecode: a backtracking vm and a Pike VM. It read only `README.md`,
`docs/`, `std/`, `examples/tutorial`, `examples/embed`, `tests/run` and
`tests/fail`; it did not read the compiler's source. What the engine
accepts is in `SPEC.md`.

Both engines agree with Python's `re` on every case tried, captures
included: about 30,000 random pattern and subject pairs, plus 80
hand-written cases. That held in all three dispatch modes, under
AddressSanitizer, and under a bytecode fuzzer and a pattern fuzzer.

## Summary

- **The language part went smoothly.** About 1,000 lines of new Loimi
  built after three one-line fixes, and the first run gave correct
  matches. The whole job took about 47 minutes.
- **The friction was in the embedding path**: a panic in the library
  killed the C host, and AddressSanitizer and `--vm-stats` didn't work
  for libraries. The generated C header was minimal.
- **The engine is fast.** The backtracker beats Python's `re` and the
  PCRE2 interpreter on most benchmarks, and is within about 2× of
  PCRE2's JIT.
- **Almost every issue has been fixed** since the study.

## Issues

Most important first.

| # | Issue | State |
|---|---|---|
| 1 | A panic in library code (bounds check, `unwrap`, out of memory) aborted the C host. | Fixed. `#[catch_panic(v)]` on an `export "C"` function turns a panic into the return value `v`; `rt.panic_message()` gives the message. `mem.try_alloc` returns `?*T`. Costs 1–2 ns per call. |
| 2 | `--sanitize=address` refused `--lib` and `--shared`. | Fixed. The host links the ASan runtime: `gcc -fsanitize=address host.c librx.a`. |
| 3 | `--vm-stats` failed for a library build. | Fixed. The report prints when the host exits. |
| 4 | The generated header had no constants, no doc comments, no `const` pointers, and no way to choose a C name for opaque types. | Fixed: `export "C" const`, `///` comments copied, `#[c_const(..)]`, `#[c_name(..)]` (or `--header-prefix`). |
| 5 | No generated op enum, so the Pike VM dispatched with an if-chain that silently ignored new ops. | Fixed. `Vm.Op` and `Vm.op_of(w)`; the Pike VM now uses exhaustive `match`es. |
| 6 | `!x` was bitwise not on integers, undocumented; `~x` was a parse error. | Fixed. `~x` is bitwise not; language.md has an operator table. |
| 7 | `jump (expr)` was parsed as a call to a function named `jump`. | Fixed. Control words take a parenthesized operand. |
| 8 | `Vec.new()` allocated and `*T` can't be null, so placeholder values leaked when overwritten. | Fixed. An empty `Vec` allocates nothing; stdlib.md shows the pattern for optional resources. |
| 9 | `--vm-stats` suggested super instructions that overlap each other. | Fixed. Overlapping candidates are listed separately, commented out. |
| 10 | `a ?? b` was a type error when `b` is itself an option. | Fixed. `a ?? b` with `b: ?T` gives `?T`. |
| 11 | Small items: `var r = Ok(..)` needs a type annotation; `in` is reserved; no `memchr`/`memcmp` on raw pointers; no `HashMap.clear()`. | Mostly fixed. Better error messages for the first two; `mem.find_byte`, `mem.compare_bytes` and friends, and `HashMap.clear()` were added. Inference stays local. |
| 12 | The docs had no short example of a C API with an opaque handle, an error string and an output array, and none of returning memory the C side must free. | Open as of the study. language.md's embedding section was enough to write the API. |

## What worked well

- **Declaring the vm paid for itself.** Encoders, the disassembler, the
  verifier and `optimize` came for free. `Verified<Rx>` made "only
  verified code runs" a type-level fact. The bytecode fuzzer (about 80
  lines) ran 20,000 mutated programs under AddressSanitizer cleanly.
- **The generated code is good.** The CLASS handler is 17 instructions,
  with no spills.
- **`-> never` vm functions** (`backtrack`, `grow_stack`) were exactly
  right for a backtracking matcher: one shared copy, called from every
  op's failure path.
- **`std.bc`** labels and fixups kept the compiler short.
- **Super instructions** took two minutes from `--vm-stats` to
  `optimize`.
- **A per-match step limit** took 15 lines as an instrumentation
  variant, and costs nothing when off.
- **The embedding path**: no initialization call, a clean symbol table,
  no clashes between two Loimi libraries, and threads are safe.
- **The library needed no GC.** The vm, the verifier, `std.bc` and the
  embedding path all work without it.
- **Builds are fast**: 0.9 s for the release library.

## Benchmarks

4 MiB of generated text. Each benchmark counts the non-overlapping
matches, calling the library from C. Best of 3, on an i7-6700, with
Python 3.13.5 and PCRE2 10.46 through its C API.

| benchmark | pattern | rx backtrack | rx Pike VM | Python `re` | PCRE2 | PCRE2 JIT |
|---|---|---:|---:|---:|---:|---:|
| literal | `Sherlock Holmes` | 3.1 ms | 15.3 ms | 2.4 ms | 2.6 ms | 0.8 ms |
| literal-alt | `Sherlock\|Watson\|Holmes\|1887` | 7.9 ms | 74 ms | 36 ms | 17.7 ms | 9.4 ms |
| class | `[a-z]+ing\|\d{3,}` | 57 ms | 327 ms | 149 ms | 220 ms | 17.0 ms |
| words | `\w+` | 17.4 ms | 185 ms | 153 ms | 53 ms | 24.4 ms |
| capture | `(\w+)\s+(\w+)` | 15.8 ms | 309 ms | 106 ms | 49 ms | 17.0 ms |
| alt-heavy | `(?:fox\|dog\|cat\|bird\|lazy)\s(?:over\|jumps\|end)` | 22.8 ms | 135 ms | 31 ms | 48.6 ms | 11.7 ms |
| dot-star | `x9.*end` | 9.7 ms | 88 ms | 7.9 ms | 21.4 ms | 4.2 ms |
| pathological | `(a*)*b` on a²² | 0.0 ms¹ | 0.0 ms | 417 ms | 0.0 ms¹ | 0.0 ms¹ |
| patho-alt | `(a\|aa)+$` on a²⁸b | 42 ms | 0.0 ms | 119 ms | 152 ms | 11.3 ms |

¹ The input has no `b`, so a required-byte check rejects it before
matching. With `(a*)*[b]`, which hides the required byte, the
backtracker takes 138 ms and Python 418 ms.

- The backtracker beats Python's `re` and the PCRE2 interpreter except
  on `literal` and `dot-star`, where it is within 30% of Python.
- It is within about 2× of PCRE2's JIT, and ahead of it on `words` and
  `literal-alt`.
- The Pike VM is a plain loop with no fast paths: 5–20× slower, but
  linear, so it handles the pathological cases.
- `goto` and `switch` dispatch are 0–10% slower than `tailcall`, except
  on `patho-alt`, where `switch` is 30–60% slower.
- The step limit costs 3–15% when set.

## Size and time

1,369 lines of Loimi for the engine and its C API (2,186 lines in all,
with the C host and test scripts). The largest files are `syntax.lm`
(310, the parser), `vm.lm` (307) and `compile.lm` (285).

| phase | time |
|---|---|
| reading the docs | 6 min |
| parser, vm, compiler, Pike VM, C API, host | 8 min |
| differential tests against Python, first benchmarks | 5 min |
| AddressSanitizer and `--vm-stats` for the library, super instructions | 6 min |
| matching Python's rules for empty loop iterations | 5 min |
| verifier test, fuzzer, step limit, dispatch modes, threads | 7 min |
| `SPEC.md` and this report | 10 min |

Most debugging time went into regex semantics, not into Loimi.
