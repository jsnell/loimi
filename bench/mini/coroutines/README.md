# Coroutine benchmarks for Mini

Three public Lua benchmark programs that use coroutines, adapted to
Mini (`examples/mini/SPEC.md` plus the `coroutine` library) with
mechanical edits only. The latest results are in
[`docs/results.md`](../../../docs/results.md) as the `mini-coro` suite.

- Each `NAME.lua` prints exactly `NAME.expected`, which is PUC-Lua
  5.4.9's output. LuaJIT 2.1, with and without `-joff`, prints the same.
- Sizes were chosen so that PUC-Lua 5.4.9 takes about 1–2 s.

## Running

```sh
bench/mini/coroutines/run.py [--mini PATH] [-n RUNS] [--lua-dir DIR] [NAME ...]
```

This times the Mini binary given with `--mini` against `lua` and
`luajit -joff`, best of 3 by default. From a git worktree, pass
`--lua-dir` pointing at the main checkout's `bench/lua/install/bin`.

## coroutine_ring.lua

- Source: `bench/coroutine-ring.lua` from
  https://github.com/LuaJIT/LuaJIT-test-cleanup, commit
  `014708bceb70550a3ab8d539cff14d9085ca9cb8`.
  Originally the Computer Language Benchmarks Game "thread-ring" program,
  contributed by Sam Roberts, reviewed by Bruno Massa.
- License: Benchmarks Game program, under the Benchmarks Game's revised
  BSD license. (LuaJIT-test-cleanup has no license file; its README says
  Mike Pall's own tests are public domain and that others "bear other
  copyrights, probably public domain, BSD or MIT licensed".)
- What it does: 503 coroutines pass a token around a ring by
  resume/yield, 10^7 hops.
- Edits:
  - `local n = tonumber(arg and arg[1]) or 2e7` → `local n = 10000000`
    (no `arg` in Mini; integer constant).

## chameneos.lua

- Source: `bench/chameneos.lua` from LuaJIT-test-cleanup, same commit.
  (A coroutine version of the Benchmarks Game "chameneos" program; no author
  line in the file. The repo's PARAM_x86.txt runs it with 1e7.)
- License: no header; by the repository README, either Mike Pall's
  (public domain) or a Benchmarks Game contribution (revised BSD).
- What it does: four coroutines meet in a shared place under a round-robin
  scheduler; 1.5·10^6 meetings, many short resume/yield pairs.
- Edits:
  - `local N = tonumber(arg and arg[1]) or 10` → `local N = 1500000`.

## sieve.lua

- Source: `bench/tests/sieve.lua` from https://github.com/luau-lang/luau,
  commit `e3275c20972d97fd5b230b89b742770064004256`. Itself the classic
  `test/sieve.lua` from the Lua distribution (Roberto Ierusalimschy).
- License: MIT (Copyright (c) 2019-2026 Roblox Corporation; Copyright (c)
  1994–2019 Lua.org, PUC-Rio).
- What it does: prime sieve as a chain of `coroutine.wrap` filters
  (primes below 1000), repeated 500 times. Deep chains of nested resumes
  (up to 168 filters).
- Edits:
  - Removed the two `bench_support` harness lines at the top
    (`prequire`/`require`, which use pcall and require) and the final
    `bench.runCode(test, "sieve")`, replaced by `test()`.
  - Removed the `os.clock()` timing (`ts0`, `ts1`, `return ts1-ts0`).
  - Added a prime counter (`local count = 0`, `count = count + 1` per
    prime, `io.write(count, "\n")` at the end) so the run has checkable
    output.
  - `for loops=1,100` → `for loops=1,500`.

## Not used

- `pidigits-nogmp.lua` (LuaJIT-test-cleanup) yields its digits from a
  coroutine but builds its code with `loadstring` and `string.gsub`.
- The Benchmarks Game archive
  (salsa.debian.org/benchmarksgame-team/archive-alioth-benchmarksgame) has
  no Lua programs for threadring or chameneos-redux.
- gligneul/Lua-Benchmarks has no coroutine programs. Luau's
  `bench/micro_tests/test_Pcall_pcall_yield.lua` needs pcall.
