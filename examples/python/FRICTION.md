# Writing a Python interpreter in Loimi: a usability report

This report is for anyone improving Loimi's usability, or curious how
usable it is. It records what was hard when a first-time user wrote a
Python 3 subset ([SPEC.md](SPEC.md)) in Loimi, and how fast the result
is.

## The study

- **What was built:** a Python 3 subset with dicts as the core data
  structure, exceptions (`try/except/else/finally`, `with`), generators,
  arbitrary-precision ints, classes with inheritance and `super()`,
  keyword and default arguments, closures with `nonlocal`, and string
  formatting (f-strings, `%`, `.format`). These were chosen because
  earlier studies had not exercised them.
- **How:** written from the docs only (`docs/`, `std/`, `README.md`,
  `loimi --help`, and `tests/` as samples). The other examples were not
  read until the interpreter passed its tests.
- **When:** 2026-11-03. The first version took about an hour from an
  empty worktree to all tests and all 11 benchmarks passing. Three
  rounds of optimization followed on 2026-11-04.
- **Size:** about 10,350 lines of Loimi at the time of the study.

| | At the time of the study |
|---|---|
| Files | `lexer.lm` 309, `ast.lm` 100, `parser.lm` 913, `compile.lm` 1738, `vm.lm` 1341, `ops.lm` 2095 (runtime semantics), `builtins.lm` 2001 (builtins, methods, `math`/`random`/`time`/`sys`, a Python prelude), `fmt.lm` 505, `value.lm` 555 (values, objects, the dict), `big.lm` 585 (bigints), `main.lm` 134 |
| VM | 78 `op`/`family` declarations, 4 supers, 32-bit Lua-style instruction words, register windows, frames in a host array, NaN-boxed values, three inline-cache layouts |
| Tests | 10 Python programs (expected output from CPython 3.13.18), run in tail-call, goto and switch builds, each normally and under `--gc-stress`, plus `--gc-verify` and `--gc-check-roots`: 80 runs, all pass |
| Compiler or std changes needed | None. One compiler bug was worked around by renaming (issue 1). |

## Main findings

- **The hot loop was easy; everything around it was hard.** Python's
  cost is in calls, attribute lookups and allocation. In this design
  those live in host functions reached from `cold` blocks, and at the
  time there were few tools for seeing where that time went.
- **Effects made re-entry safe without effort.** Every function that
  can call back into guest code is marked `uses reenter`, and the
  compiler inserted every sync, reload and GC root. GC stress, verify
  and root checks found no bug at any point.
- **The core Python data types had to be written by hand.** std had no
  bigint and no ordered map, so these were a sixth of the interpreter.
  Both are now in std (`std.bigint`, `std.dict`).
- **Stack frames crept onto handler fast paths** through ordinary code:
  the tutorial's GC check, a `raise` in a rare branch, a panic from
  `unwrap`, a write barrier. Finding them needed `objdump`. A lint now
  reports them.
- **Superinstructions suggested by `--vm-stats` did not help.** They
  saved millions of dispatches and no measurable time.
- **Result:** 0.68x CPython 3.13's time for the first correct version,
  0.33x after the third optimization round.

## Issues and their current state

State as of the last update to this report (2026-11-05 for the
language papercuts).

| # | Issue | State | Notes |
|---|---|---|---|
| 1 | Two structs with the same name in different modules collided in generic instances (`Vec<Handler>`): clang failed on the generated IR | Fixed | Instances whose argument names collide are now named with their modules (`std.mem.Vec<vm.Handler>`). Test: `tests/run/same_name_generics.lm`. |
| 2 | No bigint and no ordered map in std | Fixed | `std.bigint` and `std.dict` now exist. `big.lm` shrank from 585 to 140 lines and ints got 1.5-10x faster. The dict is `std.dict.Dict<Value, Value>`. |
| 3 | A NaN box held only 7 pointer variants; Python has about 20 heap types, so most went into one catch-all `Obj` and ints were `Int(i32)` | Fixed | Up to 8 `Gc` variants can now share one 48-bit block, told apart by the low pointer bits. Ints are `Int(i48)`; sums past 2^31 no longer allocate. |
| 4 | The tutorial's GC check (`cold { gc.collect(..) }`) put a stack frame on handler fast paths | Documented | Partly a misreading: such a block is outlined when the rest of its block ends in a control word, which held for these ops. It is not outlined in a plain vm function; the docs now say so. |
| 5 | A `raise` or panic in a rare branch gave a handler a frame | Fixed (opt-in lint) | `loimi build --release -W handler_frame` reports each handler with a frame on its fast path and the likely calls that cause it. |
| 6 | Write barriers added a `push`/`pop` to every handler that stores a reference | Explained, won't fix | The push comes from stack alignment for any call from a handler. An inline-asm stub experiment measured as noise and grew code 8-30%; it is on branch `cold-stub-experiment`. |
| 7 | Generators: `next_pc` in an op declared `then OP` is the `then` word's address, which broke generator loops | Fixed | New `then_next_pc`; using `next_pc` in such an op warns (`next_pc_then`). No docs section on generators was written. |
| 8 | Calling guest code from host code is safe but costly, so hot paths had to be redone as in-loop frames | Open | Unchanged. |
| 9 | `mem.sort_by` takes a comparator that can neither fail nor re-enter | Fixed | `mem.try_sort_by` / `Vec.try_sort_by`: a stable sort with a `Result` comparator that may be `uses reenter`. |
| 10 | Small language papercuts (table below) | Mostly fixed | Literal-only expressions in a `for` range still need a type annotation. |
| 11 | No std help for dict-backed objects with inline caches | Changed | Instances now use inline slots behind shapes (see [Instance layout](#instance-layout-inline-slots-behind-shapes-2026-11-04)). `std.obj` did not fit. |
| 12 | `--vm-stats` super suggestions did not pay off | Won't fix | By design: the docs already say supers rarely help on their own. |
| 13 | No way to see where time goes outside handlers (no `perf`, no sampler) | Fixed | `--vm-stats` now counts entries into each `cold` block, with its source location, at no cost without the flag. |

### Language papercuts (issue 10)

Each of these cost one edit-compile cycle.

| Tried | State |
|---|---|
| `var G: *A = 0 as *A;` rejected as not constant | Fixed: `0 as *T` is allowed in global initializers |
| Enum-to-int cast in a `const` | Fixed |
| `A(a, b) \| B(a, b) =>` with bindings | Fixed |
| `let f = \|x\| ..;` as a local helper closure | Fixed: a local `block fn` that can be called or passed on, never stored |
| `(if x { 1 } else { 0 }) == y` with `y: i32` | Fixed (2026-11-05) for `if`, `match` and block operands |
| `for j in 0..3 { .. j + base_u32 .. }` | Open: needs `for j: u32 in 0..3`; the error now notes that `j` is `i64` |
| `std.lex.push_utf8(..)` without `import std.lex` gave `unknown name std` | Fixed: the error suggests the import |
| `op DELGLOBAL(name: kst) : ABx` put the operand in the 8-bit field | Fixed: the error names the format field and suggests a placeholder operand |
| One error per build (about fifteen fix-and-rebuild cycles) | Fixed (2026-11-05): a build reports every independent error, up to `--max-errors N` |

Two messages were already clear and needed no change: `var` as a
reserved word, and control words in plain vm functions.

## What worked well

- **The vm declaration.** Ops, operand kinds, cache layouts, families,
  `then JMP` and `inline for` over op tables. Encoders, the disassembler
  (`--disasm` was the main debugging tool) and the verifier came for
  free.
- **Effects.** See the main findings.
- **`cold` blocks with control words, and `-> never` vm functions.**
  The call path, generator resume and in-loop operator methods are each
  one vm function, inlined on hot paths and shared on cold ones.
- **`on raise`.** Python's exception unwinding is one 25-line handler.
- **`--explain-values`**, the `value_layout` warning and `block N` pins
  made the NaN-box layout quick to decide.
- **Diagnostics** are precise and usually say what to write instead.
  Panics name the source line.

## Would the other examples have helped?

Yes, in three places, though none has dict-backed objects or bigints,
which were the bulk of the work:

- `examples/minijs/prelude.lm` uses the same idea as this interpreter's
  Python prelude: builtins that call back are written in the guest
  language.
- MiniJS's `verify handlers` block is what the exception-handler table
  lacked until the end.
- Mini's `COROUTINES.md` would have shortened the generator design.

## Benchmarks

All 11 pyperformance programs in `bench/python/programs` print exactly
CPython's output. Times are from `bench/python/compare.py`: wall clock,
best of 3 or 5 interleaved runs, tail-call builds, on a shared 8-core
x86-64 machine. CPython 3.13.18 and MicroPython 1.29.0 are built from
pinned sources by `bench/python/install.sh`. MicroPython's `random` and
float printing differ from CPython's, so three of its outputs mismatch
and are left out of its geomean.

Geomean of Loimi's time over CPython's:

| Version | Date | vs CPython | vs MicroPython |
|---|---|---:|---:|
| First correct version | 2026-11-03 | 0.68 | |
| After the first optimizations | 2026-11-03 | 0.53 | 0.20 |
| [Second round](#optimizations-second-round-2026-11-04) | 2026-11-04 | 0.45 | |
| [Instance layout](#instance-layout-inline-slots-behind-shapes-2026-11-04) | 2026-11-04 | 0.39 | |
| [Third round](#optimizations-third-round-calls-generator-expressions-slices-2026-11-04) | 2026-11-04 | 0.33 | 0.11 |

The ratios come from separate runs under different machine load, so
compare them within a round rather than across rounds.

After the third round:

| Benchmark | Loimi (s) | CPython (s) | Loimi/CPython |
|---|---:|---:|---:|
| chaos | 0.242 | 0.682 | 0.35 |
| deltablue | 0.340 | 0.934 | 0.36 |
| fannkuch | 0.264 | 0.878 | 0.30 |
| float | 0.202 | 0.826 | 0.24 |
| go | 0.276 | 0.766 | 0.36 |
| hexiom | 0.290 | 0.659 | 0.44 |
| nbody | 0.352 | 0.926 | 0.38 |
| nqueens | 0.318 | 0.764 | 0.42 |
| raytrace | 0.294 | 0.855 | 0.34 |
| richards | 0.175 | 0.797 | 0.22 |
| spectral_norm | 0.195 | 0.676 | 0.29 |
| **geomean** | | | **0.33** |

Dispatch modes, on the version at 0.53x CPython (geomean relative to
tail calls): goto 1.045, switch 1.193. Tail calls were fastest on every
benchmark.

### First optimizations (2026-11-03)

In order of payoff:

| Change | Effect |
|---|---|
| Medium ints: an int outside i32 but inside i64 is one small box holding an `i64` | A generator summing 1M ints: 107 to 49 ms |
| `__init__` cached in the class; SETATTR appends a new attribute when the dict has the size it had last time | 1M `P(i, 2)`: 190 to 106 ms (CPython 112); raytrace 0.82 to 0.63 s |
| Slice assignment in place | fannkuch 0.78-0.97 to 0.46 s |
| Operator methods (`__add__`, `__eq__`, ...) called in the loop instead of through a nested run | raytrace 0.61 to 0.52 s, chaos 0.40 to 0.34 s |
| Inline caches for class and module attributes, static and class methods, and `super()` methods | deltablue 0.77 to 0.60 s, hexiom 0.68 to 0.59 s |
| Builtins that consume a generator (`tuple(g)`, `sum(g)`, ...) written in Python, so the generator runs in the loop | `tuple(genexp)` as fast as CPython |
| GC growth 300%, 8 MiB minimum | A few % |
| Superinstructions from `--vm-stats` | No effect (geomean 0.996) |
| Removing stack frames from hot handlers (issues 4-6) | About 2%, within noise |

The tool that found almost every win here was a set of counters in the
runtime slow paths (`PYLOIMI_COUNTS=1`), written for the purpose
because no profiler was installed (issue 13).

## Optimizations, second round (2026-11-04)

Found with a timer-based sampler (`LD_PRELOAD`, SIGPROF), `--vm-stats`
cold-block counts and `PYLOIMI_COUNTS=1`. Decisions used CPU time of
interleaved runs, because wall time varied by ±7% on the loaded machine.
Geomean against CPython: **0.53 to 0.45** (0.85 of the time before).

| Change | Effect (time after / before) |
|---|---|
| `for a, b in ..` is one op (FORITER2): `enumerate()` and `zip()` of lists write the pair without building a tuple | spectral_norm 0.62-0.68 |
| `x in list` skips the general equality for two different ints and for strings of different lengths | hexiom 0.76 (with the next line) |
| Operator-method lookup cached per class | deltablue 0.93 |
| A global method cache, `(class id, name)` to method, checked before the full lookup | richards 0.87, then 0.92 |
| `isinstance`, `list.pop`/`insert`/`append` and one-argument `math` functions run without the general call machinery | fannkuch 0.84, float 0.76 |
| Slice bounds without a division for steps 1 and -1 | fannkuch 0.95 |
| `reversed(range(..))` is a range iterator | nqueens 0.84 |
| `x ** 2` and float powers in the handler | chaos 0.93 |

Tried and dropped: caching the instance's entries array in the instance
(geomean 1.000); generational GC (0.982, but the collector was being
reworked; worth re-measuring); a larger heap minimum or growth (noise).

Remaining cost: attribute access and instance creation (GETATTR 14-23%
of go, deltablue and richards), because an instance was four objects
and a lookup was four dependent loads. That led to the next change.

## Instance layout: inline slots behind shapes (2026-11-04)

An instance is now one object: its class, its *shape* and its attribute
values in a trailing array, in the order they were added. A shape lists
the attribute names in order; instances that add the same names in the
same order share it. Shapes form a tree of transitions, with one root
per class. A 3-attribute instance went from four allocations (about 256
bytes) to one of 64 bytes. Interpreter changes only.

- **Cache check.** GETATTR and SETATTR caches hold a slot number and
  check that the shape has this name at that slot. One entry then hits
  for every shape that shares the prefix, such as subclasses that call
  the base's `__init__` first.
- **Adding an attribute** follows the shape's last transition when it is
  for the same name: a store, a shape store and a barrier, no lookup.
- **Slot counts** come from the compiler, which counts the distinct
  `self.NAME = ..` targets in each class. Extra attributes go into a
  tuple in the last slot.
- **Dict mode** handles the dynamic cases: `__dict__`, `vars()`,
  `del obj.x`, and more than 64 attributes or transitions. The instance
  then holds a real Python dict, as CPython does.
- **LOADMETHOD** caches the function itself, valid while the class id
  and class epoch match, with four entries instead of two.

**Why not `std.obj`.** It has the same model and shaped this design,
but three things did not fit:

- Its keys are untraced `u64` symbols, while Python's attribute names
  are GC strings.
- Its caches match one shape per entry; the Python benchmarks have
  sites that see many shapes with a common prefix.
- Python's dict mode must be a real `dict` that user code holds and
  changes.

**Results.** Geomean against CPython: **0.45 to 0.39** (0.87 of the
time before). deltablue 0.71, float 0.66, richards 0.68, raytrace 0.75
of the time before. Peak memory of float: 105 to 36 MB (CPython 27).

## Optimizations, third round: calls, generator expressions, slices (2026-11-04)

Same method as the second round, on a less loaded machine. Interpreter
changes only. Geomean against CPython: **0.38 to 0.33** (0.88 of the
time before); against MicroPython 0.13 to 0.11.

| Change | Effect (time after / before) |
|---|---|
| Calling a user class pushes `__init__`'s frame directly; calls with defaulted parameters and keywords bind arguments without copying | float 0.97, raytrace 0.96, go 0.96, chaos 0.97 |
| `tuple`/`list`/`set`/`sum`/`min`/`max`/`sorted` of a generator expression run the loop inline (debug builds keep the generator) | nqueens 0.66, hexiom 0.79 |
| `super().m(..)` compiles to one op, cached per class | deltablue 0.94 |
| Builtin types and their subclasses are constructed without the general call path | float 0.95, chaos 0.96 |
| List slices with steps 1 and -1 copy inline with one barrier | fannkuch 0.76, nqueens 0.94 |
| Per-class flags for `__hash__` and `__bool__`/`__len__`; interned names for other dunder lookups | go 0.96 |
| `&`, `\|`, `^` of 64-bit ints in the handlers | go 0.98 |
| CALL became a family with a class-instantiation op and a `math` function op | raytrace 0.95, float 0.96 |

Also fixed: `True & False` gave an int instead of a bool.

Remaining cost: GETATTR hits (deltablue 26%, go 13%), method calls,
libm (`pow` 14% of nbody, `sin`/`cos` 15% of float), set building in
nqueens, and `x in list` and `__getitem__` calls in hexiom.
