# Writing a Python interpreter in Loimi: a usability report

A Python 3 subset ([SPEC.md](SPEC.md)) written from the docs (`docs/`,
`std/`, `README.md`, `loimi --help`, `tests/` as samples) as a usability
study. The study was meant to exercise what earlier studies (Lua-like,
JS-like, Scheme, Lox, regex, stack, wasm) didn't: dicts as the core data
structure (instance attributes, class namespaces, globals, kwargs),
exceptions with `try/except/else/finally` and `with`, generators as
resumable frames, arbitrary-precision ints, classes with inheritance and
`super()`, keyword and default arguments, closures with `nonlocal`, and
formatting (f-strings, `%`, `.format`). I did not read `examples/*`
until the interpreter passed its tests (see the end of this file for
whether they would have helped).

## Follow-up (what has been done since)

This report is kept as written. Since then:

| # | Status |
|---|---|
| 1 | Fixed: generic instances over two types of the same name from different modules get different names. An instance keeps the short names of its arguments (`std.mem.Vec<Handler>`) unless another instance already has that name; then its arguments are named with their modules (`std.mem.Vec<vm.Handler>`), so programs without a collision keep their symbols (the IR of every example is byte-identical). Types, function and method instances (methods are named after the receiver instance), nested instances and same-named generic types as arguments: `tests/run/same_name_generics.lm`. |
| 2 | Built: `std.bigint` (stdlib.md): 64-bit limbs on new `u64` intrinsics `mul_wide`/`div_wide`, Karatsuba, Knuth division, divide-and-conquer formatting, Python's floor division, two's-complement bit operations, correctly rounded `to_f64` and CPython's int `hash`; a caller-buffer core over `Num` views, an owned `BigInt` and a GC `Big` with `gc_add(h, Arg, Arg)`-style functions. `big.lm` is now a 140-line adapter (was 585): ints are 1.5-10x faster (`fact(3000)` 11 → 1 ms, `fib(20000)` by additions 37 → 8 ms), `hex`/`oct`/`bin`/`format` of big ints are no longer quadratic, `float(n)` is correctly rounded, and `hash(n)` is CPython's. The dict is `std.dict.Dict<Value, Value>` (CPython's compact layout with stored hashes, entry indices stable until a compaction, a `version` counter for inline caches): its three lookups became `find_by` closures and `find_with` (fallible, re-entrant, restarts when `__eq__` changed the dict; the old code could index out of bounds there), `dict_append_new` became `try_add_by`, resize uses stored hashes (the old one recomputed them without user `__hash__`). Benchmarks 0.97x of the old dict's time by geomean; instance creation is about 20% slower because entries are 24 bytes (stored hash) instead of 16. Since then: instance attributes live in `std.dict.KeyDict<Value, Value>` (new: CPython's str-keys layout, entries of 16 bytes, the key gives its cached hash for resize and removal; `__dict__`/`vars()` of an instance move the attributes to a `Dict` on first use, leaving a marker AttrDict that no cache can hit, so they are live as in CPython; the class then skips its method caches, as for an instance attribute that hides a method; `tests/inst_dict.py`). Against `Dict`, best of 3-9 interleaved: the benchmarks unchanged within noise (geomean 0.99), 7 attributes per instance in a loop 0.84-0.90, `P(i, 2, 3)` in a loop unchanged: smaller allocations, but with no stored hash `eq` loads the colliding keys on the probe path. Found on the way: `d[k] = v; d.popitem()` in a loop hung (popped entries gave their room back while their index slots stayed deleted, so the index filled with deleted slots); fixed in std for Dict, KeyDict and OrderedMap (`tests/dict_popitem.py`). |
| 3 | Fixed in the compiler: a nanbox with more pointer variants than 48-bit blocks gets the *shared* layout, where up to 8 `Gc` variants share one 48-bit block, told apart by the low 3 bits of the (8-byte aligned) pointer (`block 45`; language.md section 2). The test is `(w & mask) == tag` (one more instruction than a whole block's `shr; cmp`), the payload is still one `and` (the tag folds into the load's displacement), and a `match` over the variants of one shared block compiles to a jump table. Here: `Int(i48)` (was `Int(i32)`, with medium-int boxes from 2^31), `Str`, `List`, `Tuple`, `Inst`, `Func` in whole blocks; `Dict`, `Obj` and two new variants share one: `Cell(Gc<Cell>)` (closure cells: a one-field object instead of a 96-byte Box) and `Long(Gc<Long>)` (medium and big ints, so int code matches `Long(b)` instead of `Obj(b)` plus `b.kind == K_BIG`). `bench/python/compare.py -n 3`, old and new interleaved, twice: geomean 1.01 and 0.99 of the old time (noise); spectral_norm 8-9% slower in both runs (its `(i + j) * (i + j + 1) // 2` is i48 arithmetic: an overflow check at 48 bits is `shl; shl; add; jo; shr` instead of `add; jo`), the others within 3%. Sums past 2^31 no longer allocate: `s = s + i` over `range(3000000)` 84 → 18 ms, a tuple-unpacking loop summing to 5*10^11 51 → 27 ms. Classes, bound methods, generators and iterators stay `Obj` kinds: their dispatch reads fields of the same object right after the kind check, and splitting them touches every generic `Obj(b)` site. `tests/int_widths.py` covers ints around 2^47 and 2^63. Follow-up on the i48 cost (lowering.md, "Overflow-checked arithmetic on odd widths"): the checked add (`shl; shl; add; jo; shr; or`) is already the short form; doing it in i64 on sign-extended operands with a range check is 12 instructions and 5-10% slower in int loops, and a separate plain add for the value (shorter dependency chain) 2-5% slower. What did change: `/` and `%` on odd widths test `y == 0` and `MIN / -1` on the shifted operands, so FLOORDIV's `y > 0` guard folds both checks away (the handler had both panic calls and a stack frame on its fast path; 77 -> 59 lines of assembly). spectral_norm, best of 9 interleaved, relative to the old `Int(i32)` build: 1.100 before, 1.076 after; the same sources with `Int(i32)` on today's layout: 1.029. The remaining ~5% is the two `shl` per add/compare and the `shr` before re-tagging, which need the tag in the low bits of a rotated word to avoid. |
| 4 | Partly a misreading, now documented: a `cold` block that falls through is outlined when the rest of its enclosing block ends in a control word (`if unlikely(..) { cold { gc.collect(..); } } ..; next`): the compiler copies that rest into it. That already held for MAKECELL and the other ops here (with or without `redo`); their frames come from the allocation's `gc.refill` and the barrier's `remember` on the fast path. It does not hold in a plain vm function (`gc_point`), which has no rest to copy (language.md section 4 and tutorial chapter 7 now say so). Outlining fall-through blocks as returning `preserve_most` calls (captures and changed state through thread-locals) was judged not worth its cost for that one pattern. |
| 5 | Fixed (opt-in): lint `handler_frame`, `loimi build --release -W handler_frame`, compiles the IR to assembly a second time and reports each tail-call handler whose fall-through path sets up a frame, with the calls in it as likely causes, panics with their messages: ``op `LOADCELL` sets up a stack frame on its fast path (`pushq %rbp`) ..: `std.gc.refill`, `ops.exc_of`, a panic (`examples/python/ops.lm:111:15: unwrap of None`)`` (50 handlers here; 40 in Mini, 67 MiniJS, 12 Scheme). Making panic paths in handlers tail calls to a handler-shaped stub was tried and measured: it removed the frame of 1 of the 218 handlers with one across the seven interpreters (the others make other calls too), so it was not kept. |
| 6 | Explained, not changed: the push comes from stack alignment for any call from a handler entered by `jmp`, not from the barrier itself (`docs/lowering.md` §3, "Why slow-path calls give handlers a frame"). An inline-asm stub experiment removed some frames but measured as noise overall and grew code 8–30%, so it stays on branch `cold-stub-experiment`. |
| 7 | Fixed: `then_next_pc`, the address after an op's `then` instruction, in ops declared `then OP`; `next_pc` in such an op (or a vm function inlined into it) warns (`next_pc_then`), since it is the `then` word's address (language.md section 3.5). FORITER uses `then_next_pc` (same IR). A generators section is not written. |
| 8 | Unchanged. |
| 9 | Fixed: `mem.try_sort_by(p, n, less)` / `Vec.try_sort_by(less)`, a stable sort whose comparator returns `Result<bool, E>` and is `uses reenter`; it sorts indices and permutes once, so elements stay in place (reachable) while guest code runs, and the data is unchanged on error (stdlib.md). `sorted`/`list.sort` use it (the 40-line merge sort is gone). Mini's `table.sort` cannot: it reproduces Lua's quicksort (unstable, and its "invalid order function" detection depends on that algorithm). |
| 10 | Mostly fixed: `0 as *T` in global initializers; enum-to-int casts in constants (they failed whenever the constant was used); `A(a, b) \| B(a, b) =>` with bindings (one arm per alternative); `let f = \|x: T\| ..;` is a local `block fn` (called or passed on, never stored, so it cannot outlive its captures); a mismatch in the body of `for j in 0..3` notes that `j` is `i64`; `unknown name std` (and an unimported std module that is loaded) suggests the import; a mismatched encoder argument notes which format field the operand is ("operand `name` of `DELGLOBAL` is field `a` of format `ABx` (8 bits) .. a placeholder operand such as `z: imm` moves the next ones along"). Literal-only expressions are still typed `i64` without context. |
| 11 | Changed: instances keep their attributes in inline slots behind shapes, with dicts only for the dynamic cases ("Instance layout" above); the caches check the name at the cached slot, which is the "entry index for key K" rule asked for here, with the shape as the layout. Nothing generic was added to std: `std.obj` did not fit (same section). |
| 12 | Unchanged (by design). |
| 13 | Fixed: `--vm-stats` counts entries into each `cold` block, per op, with its source location (`CALL (cold block at examples/python/vm.lm:681:9)`; blocks in plain vm functions under `fn name`), at no cost without `--vm-stats` (language.md section 7.3). richards: LOADMETHOD_CALLMETHOD's slow path 1.6M times, CALL's 0.99M. |

## Summary

| | |
|---|---|
| Files | `lexer.lm` 309, `ast.lm` 100, `parser.lm` 913, `compile.lm` 1738, `vm.lm` 1341, `ops.lm` 2095 (runtime semantics), `builtins.lm` 2001 (builtins, methods, `math`/`random`/`time`/`sys`, a Python prelude), `fmt.lm` 505 (repr/str, format specs, `%`), `value.lm` 555 (values, objects, the dict), `big.lm` 585 (bigints), `main.lm` 134; `check.sh` 71. About 10,350 lines of Loimi. |
| VM | 78 `op`/`family` declarations (several are `inline for` templates that expand to 6 or 12 ops), 4 supers, 32-bit Lua-style words, register windows, frames in a host array, NaN-boxed values (7 pointer variants), three inline-cache layouts. |
| Tests | 10 programs in `tests/` (1042 lines of Python; expected output from CPython 3.13.18), run in tail-call, goto and switch builds, each normally and under `--gc-stress`, plus generational `--gc-verify` and a `--gc-check-roots` build: 80 runs, all pass. `tests/golden.rs` `python_programs` runs `check.sh -q` (one build, normal + stress, 11 s). |
| Benchmarks | All 11 pyperformance programs (`bench/python/programs`) print exactly CPython's output. Geomean **0.53x CPython 3.13**'s time, **0.20x MicroPython**'s (table below). |
| Time | By commit timestamps: worktree created 09:37; first commit (lexer, parser, values, bigints, vm, runtime) 10:10; first programs run 10:24; all tests passing in every mode 10:30; all 11 benchmarks correct 10:31; optimizations until 11:00. (The clock in this environment advanced oddly slowly relative to the work done; the order and the relative gaps are reliable, the absolute minutes less so.) |
| Compiler/std changes | None. One compiler bug found (item 1), worked around by renaming. |

## Benchmarks

`bench/python/compare.py -n 3` (best of 3, interleaved, wall clock,
shared 8-core x86-64 machine; run-to-run noise about 5%). CPython 3.13.18
and MicroPython 1.29.0 are built from pinned sources by
`bench/python/install.sh` (CPython: plain `./configure`, which is `-O3`
without PGO or LTO; MicroPython: the unix port's standard variant).
MicroPython's `random` differs from CPython's and its float printing
differs in the last digit, so three of its outputs mismatch; they are
left out of its geomean.

| benchmark | loimi (s) | CPython (s) | MicroPython (s) | loimi/CPython | loimi/MicroPython |
|---|---|---|---|---|---|
| chaos | 0.338 | 0.706 | 1.838 (mismatch) | 0.48 | - |
| deltablue | 0.596 | 0.962 | 4.905 | 0.62 | 0.12 |
| fannkuch | 0.459 | 0.906 | 1.406 | 0.51 | 0.33 |
| float | 0.476 | 0.883 | 2.168 | 0.54 | 0.22 |
| go | 0.387 | 0.805 | 1.848 (mismatch) | 0.48 | - |
| hexiom | 0.525 | 0.694 | 2.984 | 0.76 | 0.18 |
| nbody | 0.344 | 0.957 | 3.413 (mismatch) | 0.36 | - |
| nqueens | 0.773 | 0.814 | 1.659 | 0.95 | 0.47 |
| raytrace | 0.522 | 0.869 | 3.396 | 0.60 | 0.15 |
| richards | 0.324 | 0.825 | 3.148 | 0.39 | 0.10 |
| spectral_norm | 0.300 | 0.704 | 1.371 | 0.43 | 0.22 |
| **geomean** | | | | **0.53** | **0.20** |

The first correct version (all tests and benchmarks right, no
optimization work) was at 0.68x CPython's time by geomean.

### What helped (in order of payoff)

| change | effect |
|---|---|
| Medium ints: an int outside i32 but inside i64 is one small box holding an `i64`, not limbs built through `mem.Vec` temporaries | a generator summing 1M ints 107 → 49 ms; anything whose sums pass 2^31 |
| Instance creation: `__init__` cached in the class box (validated by a class epoch), and a SETATTR cache that *appends* a new attribute when the dict has the size it had last time for this class (one probe to check the key is absent) | 1M `P(i, 2)`: 190 → 106 ms (CPython 112); raytrace 0.82 → 0.63 s, float 0.67 → 0.48 s |
| Slice assignment in place instead of through a host `Vec` | fannkuch 0.78–0.97 → 0.46 s |
| Operator methods (`__add__`, `__getitem__`, `__eq__`, ... that are plain functions of two parameters) called *in the loop*: the cold path pushes a frame whose return slot is the op's destination, instead of a nested run | raytrace 0.61 → 0.52 s, chaos 0.40 → 0.34 s |
| Mixed int/float arithmetic and comparisons handled at the top of the cold block, before the general (re-entrant, synced) path | part of the line above |
| Inline caches for attributes of classes and modules (`Strength.REQUIRED`, `math.sqrt`), for static and class methods called through the class, and `super()` methods returned unbound; two-entry attribute and method caches | deltablue 0.77 → 0.60 s, hexiom 0.68 → 0.59 s |
| `in` / `not in` against dicts, sets, lists and tuples of builtin values without a call | hexiom 0.55 → 0.53 s |
| `==`/`!=` with an instance whose class defines neither `__eq__` nor `__ne__` decided by identity at the top of the cold block | deltablue's 538K slow comparisons gone; time within noise |
| `x is None` compiled to one compare-and-branch op; `return local` without a MOVE | within noise |
| Generator arguments of consuming builtins (`tuple(g)`, `sum(g)`, `"".join(g)`, ...) collected by a function written in Python (a prelude), so the generator runs in the loop rather than one nested run per item | `tuple(genexp)` micro now equal to CPython; nqueens within noise |
| GC growth 300% / 8 MiB minimum (`set_growth`) | raytrace did 448 collections (55 ms); a few % |
| Superinstructions from `--vm-stats` (`LOADMETHOD_CALLMETHOD`, `GETATTR_GETATTR`, `GETATTR_GETATTR_MUL`, `MOVE_MOVE_MOVE`) | **no effect**: geomean 0.996 of the version without them (`--no-supers` A/B) |
| Removing stack frames from hot handlers (items 4–6 below) | within noise (about 2%) |

Dispatch modes on the final version (best of 3, geomean relative to
tail calls): **goto 1.045, switch 1.193**. Tail calls are fastest on
every benchmark.

What Loimi made hard here was not the hot loop (see "Generated code"):
it was everything around it. Python's cost is in calls, attribute
lookups and allocation, which in this design live in host functions
reached from `cold` blocks, and the tools for seeing where *that* time
goes are thin (item 13).

## Optimizations, second round (2026-11-04)

Where the time went, from a timer sampler (`LD_PRELOAD`, SIGPROF every
37 µs, the interrupted `rip` plus return addresses found by scanning the
stack, so callers of the runtime functions show), `--vm-stats` cold-block
counts and `PYLOIMI_COUNTS=1`, per benchmark, and what was done about it.
A/B numbers are CPU time (user + sys), best of 3-5 interleaved runs, on a
machine with load 4-6 from other agents: wall times moved by ±7% between
identical runs, CPU times by about ±3%.

`bench/python/compare.py -n 5`, tail-call builds with
`--align-functions=64` before (6187ede) and after, interleaved with
CPython 3.13.18, wall clock, best of 5 (load average 4.5-5.4):

| benchmark | before (s) | after (s) | CPython (s) | before/CPython | after/CPython | after/before |
|---|---:|---:|---:|---:|---:|---:|
| chaos | 0.423 | 0.416 | 0.862 | 0.49 | 0.48 | 0.98 |
| deltablue | 0.713 | 0.645 | 1.195 | 0.60 | 0.54 | 0.90 |
| fannkuch | 0.635 | 0.526 | 1.183 | 0.54 | 0.45 | 0.83 |
| float | 0.595 | 0.462 | 1.151 | 0.52 | 0.40 | 0.78 |
| go | 0.479 | 0.480 | 1.088 | 0.44 | 0.44 | 1.00 |
| hexiom | 0.682 | 0.554 | 0.916 | 0.74 | 0.60 | 0.81 |
| nbody | 0.471 | 0.461 | 1.231 | 0.38 | 0.37 | 0.98 |
| nqueens | 0.993 | 0.813 | 1.063 | 0.93 | 0.76 | 0.82 |
| raytrace | 0.666 | 0.635 | 1.170 | 0.57 | 0.54 | 0.95 |
| richards | 0.407 | 0.332 | 1.070 | 0.38 | 0.31 | 0.82 |
| spectral_norm | 0.413 | 0.265 | 0.939 | 0.44 | 0.28 | 0.64 |
| **geomean** | | | | **0.53** | **0.45** | **0.85** |

Per change (`after/before` of that step):

| change | why (profile) | effect |
|---|---|---|
| `for a, b in ..` into two locals is one op, FORITER2: enumerate() and zip() of lists write the pair without the tuple, a list of pairs unpacks in the op; comprehensions too | spectral_norm: `ops.next` on enumerate 15%, a tuple and an UNPACK per item | spectral_norm 0.62-0.68 |
| `x in list`/`tuple` skips `eq_builtin` for two different ints and for strings of different lengths | hexiom: `eq_builtin` 11%, called per element from `contains_fast` | hexiom 0.76 (with the next line) |
| Operator-method lookup (`__getitem__`, `__add__`, `__eq__`, ..) cached per class in the class box, reset with the `__init__` cache at each class epoch; `==` on instances caches "no `__eq__`/`__ne__`" there too | hexiom's `Done.__getitem__` walked the class chain per index; deltablue's `identity_eq` did 4 dict probes per `!=` (8%) | deltablue 0.93 (with the next lines) |
| A global method cache, `(class id, name) -> (entry, depth)`, 1024 entries, checked by the class epoch, probed from LOADMETHOD's cold block before `ops.loadmethod`; a hit fills the site's cache as the slow path would | richards: 2.3M LOADMETHOD misses (four Task classes at two-entry sites), `ops.loadmethod` 15% + its cold block 6% | richards 0.87, then 0.92 |
| `isinstance(inst, class)` in CALL's cold block | richards: 1M `isinstance` calls through `call_slow`/`builtins.call` | part of richards above |
| `list.pop()`, `pop(i)`, `insert(i, x)`, `append(x)` (also as bound methods) and one-argument `math` functions (`sqrt`, `sin`, `cos`, ..) run in CALL/CALLMETHOD's cold blocks without the call machinery | fannkuch: `pop(0)`/`insert` through bound methods, `call_slow` + `builtins.call` + `call_method_builtin` 17%; float: `math.sqrt/cos/sin` | fannkuch 0.84, float 0.76 |
| `slice_indices` without a division for steps 1 and -1 | fannkuch: `slice_indices` 14% (an `idiv` per slice) | fannkuch 0.95 |
| `reversed(range(..))` is a range iterator (was a list copy and a reverse iterator FORITER did not know); the prelude collecting a generator for `tuple()`/`set()`/`sum()` is a list comprehension (LISTAPPEND instead of LOADMETHOD/CALLMETHOD per item) | nqueens: 0.9M FORITER cold entries, 0.3M GETITER cold entries | nqueens 0.84, hexiom 0.95 |
| POW: a float base to a float or int power with a finite result calls `pow()` in the handler; an int squared multiplies | chaos `x ** 2` went through `ops.arith` | chaos 0.93; nbody unchanged (libm's `pow` is the cost) |
| TRUEDIV float/int on the fast path | spectral_norm's `1.0 / int` | within noise |

Dropped, measured:

- *The instance's entries array cached in `Inst`* (`o.ents`, checked
  against `o.dict.ents` in parallel, so GETATTR loads inst → entry
  instead of inst → dict → entries → entry): geomean 1.000, float 1.06
  (24-byte instances instead of 16). GETATTR is still 14-23% of go,
  deltablue and richards, nearly all of it waiting on those dependent
  loads; see "Next" below.
- *Generational GC* (`heap.set_generational(true)`): geomean 0.982
  (hexiom 0.92, raytrace 0.94, float 1.05). Not kept: the collector is
  being reworked in parallel; worth re-measuring after that.
- *Min heap 32 MiB, or growth 600%*: 0.995-0.998, noise.
- *The global method cache probed in LOADMETHOD's cold block without
  filling the site's cache*: richards 0.92 but deltablue 1.06: a
  two-class site stopped settling into its two entries.

Where the time goes now (tail-call build, share of samples):

- **Attribute access and instance creation** (deltablue, go, richards,
  raytrace, chaos, float): GETATTR 14-23% (hits: four dependent loads,
  register → instance → AttrDict → entries → entry), LOADMETHOD +
  CALLMETHOD 10-25%, SETATTR's append path (`attr_append_new`, a probe
  per new attribute) 6-9% in raytrace, chaos and float, plus `new_attrs`
  3-4%: an instance is four objects (Inst, KeyDict, index, entries).
  CPython 3.13 keeps the values inline in the object behind a shared
  keys object; doing the same (a class-level layout plus inline slots,
  the dict only when `__dict__` is used) is the largest remaining item
  and touches about 75 sites in ops/builtins.
- **GC**: `std.gc.collect` is 2-13% (nqueens 9-13%, raytrace and chaos
  9-10%, fannkuch 8%, float 7%).
- **Generators** (nqueens, hexiom): resume and YIELD copy every register
  of the frame; a generator per `tuple(genexp)`/`set(genexp)` plus the
  prelude call.
- Float math in libm (nbody `pow` 13%, float `sin`/`cos` 12%), the same
  calls CPython makes.

## Instance layout: inline slots behind shapes (2026-11-04)

The second round's profile put attribute access and instance creation
first: GETATTR waited on four dependent loads (register → instance →
AttrDict → entries → entry), an instance was four objects (Inst,
KeyDict, its index, its entries) and adding an attribute probed the
index (`attr_append_new`, 6-9% of raytrace, chaos and float). This round
changes the layout of instances; interpreter changes only, no compiler
or std change.

**The layout.** An instance is one object, `Inst { cls, keys, slots:
[Value] }`: its class, its *shape* and its attribute values in a
trailing array, in the order they were added. A shape is a `K_SHAPE`
box whose `extra` lists the attribute names (interned strings) in
order; instances that add the same names in the same order share it.
Shapes form a transition tree (child by name in a small Dict, plus the
last child taken), with one empty root per class and slot count. A
3-attribute instance (float's `Point`) is one 64-byte allocation (with
the header); it was four, about 256 bytes.

- *GETATTR/SETATTR caches* stay what they were, a slot number (two for
  GETATTR), but are checked against the shape: slot `i` is `name` if
  `keys.extra[i] == name` (one compare of interned pointers) and `i` is
  below the shape's inline count. So the check is not "same shape" but
  "same name at that slot", and one entry hits for every shape that
  shares the prefix: subclasses that call the base's `__init__` first,
  sibling classes, objects of different classes built the same way
  (deltablue's constraint hierarchy, richards' tasks). A GETATTR hit is
  about 28 instructions with the dispatch: register → instance, then
  instance → shape (count and name) and instance → slot, independent.
- *Adding an attribute* (SETATTR's add path) follows the shape's last
  transition if it is for this name, with the old class-id and
  class-epoch guard (no property or shadowed class attribute can be
  involved): a store, a shape store and a barrier, no probe.
- *Slot counts* are fixed at allocation. The compiler counts the
  distinct `self.NAME = ..` targets in the methods of each class body
  (first parameter as `self`; BUILDCLASS's new `hint` operand); a class
  starts with its own count plus its base's, and the slow path raises
  it whenever an instance of the class needs more. New instances get
  that many slots (a new root shape for the new count; old instances
  keep theirs). An instance that still runs out (go's `Square`s get two
  attributes from outside the class after all 81 exist) keeps the last
  slot's value and the rest in a tuple stored in the last slot; the
  shape's inline count then excludes those, and GETATTR/SETATTR find
  them in their cold blocks (`ops.inst_cached`) without the full
  lookup.
- *Dict mode* for the dynamic cases: `__dict__`/`vars()` (the live dict
  CPython gives), `obj.__dict__ = d`, `del obj.x` (keeps the order of
  the rest) and more than 64 attributes or 64 transitions from one
  shape (setattr with computed names). The instance gets the dict-mode
  shape (no names, so every cache misses) and a Python `Dict` in
  `slots[0]`; nothing else changes (method caches go off for the class
  as before).
- *Names from `setattr`* are interned when they enter a shape, so the
  caches' pointer compares hit for them too; odd names (`setattr(o, "a
  b", 1)`) are ordinary entries.

**LOADMETHOD** (the method-call side of attribute access, 16-25% of
deltablue and richards after the layout change): its instance entries
cached (class id, epoch, owner depth, entry number), and a hit walked
the class chain and loaded the function through the owner's dict. They
now hold the function itself (its value bits in a 64-bit cache word,
also in the global method cache), read only while the class id and the
class epoch match; then the class is alive (an instance of it is the
receiver, ids are not reused) and its chain's dicts, which hold the
function, are unchanged. That is one `unsafe { bits as Value }`
(`ops.method_value`). Four entries instead of two: richards' sites see
the four `Task` classes.

**Why not `std.obj`.** It has the same model (shapes, transitions,
dictionary mode) and its design shaped this one, but three things did
not fit. Its keys are untraced `u64` symbols, so attribute names (GC
strings, some from `setattr`) would need pinning and a string → symbol
step in every slow path. Its caches are (shape id, slot): a site is
monomorphic per shape, while the Python benchmarks' polymorphic sites
see many shapes with a common prefix, which the name-at-slot check
covers with one entry. And Python's dict mode must be a real `dict`
object that user code holds and mutates (`__dict__`), not a key → slot
index beside the slots. What could be shared is the transition table,
which is 30 lines here.

**Results.** `bench/python/compare.py -n 5`, tail-call builds with
`--align-functions=64`, before (d1cae11) and after interleaved with
CPython 3.13.18, wall clock, best of 5 (load average 4.9-5.4):

| benchmark | before (s) | after (s) | CPython (s) | before/CPython | after/CPython | after/before |
|---|---:|---:|---:|---:|---:|---:|
| chaos | 0.460 | 0.395 | 0.976 | 0.47 | 0.40 | 0.86 |
| deltablue | 0.729 | 0.518 | 1.328 | 0.55 | 0.39 | 0.71 |
| fannkuch | 0.516 | 0.513 | 1.194 | 0.43 | 0.43 | 0.99 |
| float | 0.482 | 0.319 | 1.190 | 0.41 | 0.27 | 0.66 |
| go | 0.490 | 0.515 | 1.193 | 0.41 | 0.43 | 1.05 |
| hexiom | 0.563 | 0.591 | 1.004 | 0.56 | 0.59 | 1.05 |
| nbody | 0.445 | 0.448 | 1.285 | 0.35 | 0.35 | 1.01 |
| nqueens | 0.802 | 0.802 | 1.023 | 0.78 | 0.78 | 1.00 |
| raytrace | 0.629 | 0.473 | 1.174 | 0.54 | 0.40 | 0.75 |
| richards | 0.335 | 0.228 | 1.026 | 0.33 | 0.22 | 0.68 |
| spectral_norm | 0.236 | 0.236 | 0.852 | 0.28 | 0.28 | 1.00 |
| **geomean** | | | | **0.45** | **0.39** | **0.87** |

go and hexiom came out 5% slower in this wall-clock run; in CPU time,
best of 9 interleaved right after, go took 0.88 and hexiom 0.94 of the
time before (fannkuch 0.94, nqueens 0.96: the noise floor).

Per step, CPU time (user + sys) of interleaved runs, best of 3-9 (load
average 4-7: wall and CPU times both moved by up to ±10% between
identical runs, so only the steps that held up in reruns are listed):

| change | effect |
|---|---|
| inline slots behind shapes, one global root | float 0.74, raytrace 0.83-0.85, chaos 0.85-0.86, deltablue 0.87-0.89, go 0.99-1.12 (the first version moved instances that outgrew their slots to dict mode: all of go's `Square`s), geomean 0.93-0.94 |
| a root shape per class | raytrace 0.91 of the step before: with one root, the root's last-transition cache alternated between `Vector`, `Ray` and `Sphere` (SETATTR add misses 589k → 74) |
| LOADMETHOD caches the function | deltablue 0.83-0.85, richards 0.86 |
| four LOADMETHOD entries | richards 0.79-0.87 |
| shapes with an inline count (no overflow test on hits) | eight instructions fewer per GETATTR hit; within noise |

Peak RSS (`ru_maxrss`): float 105 → 36 MB (CPython 27), the others
within ±2 MB. Instances move to dict mode only for the reasons above:
`PYLOIMI_COUNTS` counter 500 is 0 on all 11 benchmarks.

Where the time goes now (tail-call, share of samples): GETATTR is still
13-24% of go, deltablue and richards, now as dispatch plus two
dependent loads per hit rather than misses (GETATTR's slow path runs
120 times in go, 6k in deltablue); LOADMETHOD_CALLMETHOD 10-16% of
deltablue and richards; SETATTR 5-9% (its barrier's `remember` call
gives it a frame, item 6); instance creation in float, raytrace and
chaos is `call_slow`'s class path plus the `__init__` frame (5-8%).

Next candidates: a CALL fast path for classes whose `__init__` is a
simple function (allocate, push the frame, no `call_slow` loop);
caching class attributes read through instances (`self.CONST`) with
the shape (an absent-name proof per shape) and the class epoch; and
attribute reads of `self` in methods could skip the tag test when the
compiler knows the register holds the receiver.

## Optimizations, third round: calls, generator expressions, slices (2026-11-04)

Same method as the second round: the timer sampler (now also with a
variant that scans the stack for return addresses to name callers),
`PYLOIMI_COUNTS=1` slow-path counters, and A/B decisions on CPU time
(user + sys) of interleaved runs, best of 5-9 (host load 2-3; identical
builds moved by about ±1.5%). Interpreter changes only (compile.lm,
parser.lm and the runtime); no compiler or std change.

`bench/python/compare.py -n 5`, tail-call builds with
`--align-functions=64`, before (b9e179a) and after (1b6c661)
interleaved with CPython 3.13.18, wall clock, best of 5 (load average
1.8-3.3):

| benchmark | before (s) | after (s) | CPython (s) | before/CPython | after/CPython | after/before |
|---|---:|---:|---:|---:|---:|---:|
| chaos | 0.256 | 0.242 | 0.682 | 0.38 | 0.35 | 0.95 |
| deltablue | 0.362 | 0.340 | 0.934 | 0.39 | 0.36 | 0.94 |
| fannkuch | 0.327 | 0.264 | 0.878 | 0.37 | 0.30 | 0.81 |
| float | 0.224 | 0.202 | 0.826 | 0.27 | 0.24 | 0.90 |
| go | 0.311 | 0.276 | 0.766 | 0.41 | 0.36 | 0.89 |
| hexiom | 0.372 | 0.290 | 0.659 | 0.56 | 0.44 | 0.78 |
| nbody | 0.351 | 0.352 | 0.926 | 0.38 | 0.38 | 1.00 |
| nqueens | 0.517 | 0.318 | 0.764 | 0.68 | 0.42 | 0.62 |
| raytrace | 0.326 | 0.294 | 0.855 | 0.38 | 0.34 | 0.90 |
| richards | 0.173 | 0.175 | 0.797 | 0.22 | 0.22 | 1.01 |
| spectral_norm | 0.197 | 0.195 | 0.676 | 0.29 | 0.29 | 0.99 |
| **geomean** | | | | **0.38** | **0.33** | **0.88** |

MicroPython 1.29.0 in the same run: 0.13 → 0.11 (the eight programs
whose output matches).

Per change (CPU time, that step's build over the one before it):

| change | why (profile, counters) | effect |
|---|---|---|
| A call of a user class whose `__init__` takes the arguments as given allocates and pushes `__init__`'s frame from CALL's cold block, the result slot being the self slot (so RET stays on its fast path, no `F_DISCARD`); calls that leave out defaulted trailing parameters fill them in and push the frame (CALL, CALLMETHOD, `call_slow`); `bind_args` binds up to 8 arguments with keywords to plain positional parameters without copying them aside (a `mem.Vec` per call) | float, raytrace, chaos: 1M instance creations through `call_slow`'s loop; go: 755k calls of `find(self, update=False)` through `bind_args` | float 0.97, raytrace 0.96, go 0.96, chaos 0.97 |
| `tuple`/`list`/`set`/`sum`/`min`/`max`/`sorted` of a generator expression (the only argument, no lambda or generator expression inside): the comprehension runs inline into a list (a set for `set`) while GETGLOBAL finds the builtin (an `IS` against it), else the generator is made as written. `--cfg debug` builds keep the generator (the debugger steps into its frame, as pdb does) | nqueens: per permutation `tuple(..)` and two `set(..)` of generator expressions, each a generator, `bind_args` (generator functions are not "simple"), the prelude call and a resume/YIELD per item (1.6M constructs, 662k `bind_args`) | nqueens 0.66, hexiom 0.79 |
| `super(C, self).m(..)` and `super().m(..)` compile to SUPERMETHOD, which leaves the method and self as LOADMETHOD does, without the super object; cached per class id and class epoch (the lookup does not depend on self's class). The two-argument form runs it while `super` is the builtin (`IS` against it) | deltablue: 332k LOADMETHOD slow paths on super objects, 154k super constructions | deltablue 0.94 |
| Builtin types (and subclasses of them: deltablue's `OrderedCollection(list)`) are constructed from CALL's cold block, and `object.__init__(self)` (`super(..).__init__()` in a class without other bases) is a no-op in CALLMETHOD's | deltablue: 543k `call_slow` class paths; chaos: `int(..)`, `range(..)` | float 0.95, chaos 0.96, deltablue 0.98 |
| List slices with int or None bounds and steps of 1 and -1 compute their bounds inline and copy with raw stores and one barrier; `list_grow` copies the same way | fannkuch: `getslice` + `setslice` + `slice_indices` + `new_list` 46% (a barrier test and a bounds check per item; `slice_indices` returned a 4-tuple through memory) | fannkuch 0.76, nqueens 0.94 |
| Per-class flags (refreshed with the cached `__init__` at each class epoch) for `__hash__` and `__bool__`/`__len__`; interned names for `__call__`, `__iter__`, `__getitem__` in the runtime (they interned the name, a string hash and map probe, per use); SETGLOBAL keeps the entry number as GETGLOBAL_C does | go: `vm.intern` from `truthy` on instances (71k), `put_by` from SETGLOBAL (`TIMESTAMP`, `MOVES`) | go 0.96 |
| `&`, `|`, `^` of ints that fit an i64 (medium `Long`s) in the handlers' cold blocks | go: 805k `ops.arith` calls for its 63-bit Zobrist hashes | go 0.98 |
| CALL is a family: CALL_NEW (class instantiation, the first change's path) and CALL_M1 (one-argument `math` functions imported as names) run without entering the generic op's cold block (a 0x90-byte frame and the checks before them) | float, raytrace: `CALL.cold` 5-8% after the first change | raytrace 0.95, float 0.97 then 0.96 |
| SETATTR's add path checks the shape instead of the site: the shape reached by adding a name records the class epoch at which the class had no attribute of that name (shape trees are per class), so the cache holds only the slot of existing attributes (one word instead of three) | deltablue: `Constraint.__init__` adds `strength` at slot 0 or 3 (`ScaleConstraint` sets three attributes first): 57k SETATTR slow paths | slow paths 57k → 2.6k; time within noise (kept: smaller cache, simpler check) |

Also fixed on the way: `True & False` (and `|`, `^` of two bools) gave
an int; CPython gives a bool.

Checked and not changed:

- *Register clearing on calls* (what MiniJS and Scheme gained from
  today): Python's calls never cleared the callee's registers; the stack
  is scanned up to a high-water mark (`Ctx.hw`) since the first version.
- *SETATTR's stack frame* (item 6): `std.gc`'s barrier no longer calls
  `remember` (the slow path is inline), and SETATTR's remaining
  `push %rbp` is register pressure in the add path, not a call.
- *Class attributes read through instances* (`self.CONST`): GETATTR's
  slow path runs 30k times in raytrace and fewer elsewhere; not worth a
  cache.

Dropped, measured:

- *`x in list` with an int: a loop over ints that compares bits only*
  (no `surely_unequal`/`eq_builtin` per item): hexiom 0.998, go 1.018.
  `contains_fast` is 8% of hexiom, spread over many short lists.
- *CALLMETHODKW binding keywords in the handler* (`bind_kw_fast`, then
  `push_frame`, skipping `call_slow`): go 0.999 after `bind_args` had
  its own keyword fast path.

Where the time goes now: GETATTR hits (deltablue 26%, go 13%) and
method calls (LOADMETHOD_CALLMETHOD 18% of deltablue: about 70
instructions on a hit, with the new frame's eight stores); libm (`pow` 14%
of nbody, `sin`/`cos` 15% of float); nqueens: set building (`put_by`,
`hash_key` 12%), slicing, the `pool` cell the generator expression's
fallback still captures (LOADCELL 3%); hexiom: `x in list` and
`__getitem__` through `call2`.

Next candidates: frames with fewer stores (`k` and `proto` follow from
the function, `gen` matters only with `F_GEN`); operator dunders on
instances (`Vector - Vector`: SUB's cold block, 3% of raytrace) as
family members like CALL_NEW; generators that resume without copying
their registers when the window is where they left it; set/dict
building from comprehensions sized up front.

## Friction, by impact

### 1. Compiler bug: two structs with the same name in different modules collide in generic instances

*Tried:* `mem.Vec<Handler>` in the compiler, where `Handler` is
`ast.Handler` (an except clause) and the vm had its own `vm.Handler`
(an exception table entry) also used in a `Vec`.
*Happened:* the program type-checked; clang then failed:

```
/tmp/loimi-2962851/main.ll:132:1: error: redefinition of type
  132 | %"std.mem.Vec<Handler>" = type { ptr, i64, i64 }
error: clang-19 failed on generated IR (kept at /tmp/loimi-2962851/main.ll)
```

A three-file repro: `m1.lm` and `m2.lm` each declare `pub struct
Handler` with different fields, and `main.lm` creates a
`mem.Vec<m1.Handler>` and a `mem.Vec<m2.Handler>`.
*Did instead:* renamed the vm's struct to `ExcEntry`.
*Suggested fix:* mangle generic instance type names with the module path
of their arguments (`std.mem.Vec<vm.Handler>`), as function names
presumably already are. This is the only item that is a plain bug; it
also fails late (IR, not a diagnostic), so a new user cannot tell what
they did wrong.

### 2. No bigint, no ordered map in std: the core Python data types are all user code

*Tried:* looked for `BigInt` and `IndexMap` (both listed in stdlib.md
under "Design (not built)").
*Did instead:* wrote `big.lm` (585 lines: sign-magnitude, 32-bit limbs,
Knuth division, two's-complement bit operations, decimal I/O) and a
CPython-style compact ordered dict in `value.lm` (index table of `i32`
plus an entries array in insertion order, perturbed probing, deleted
markers, resizing). Both were straightforward with `object` trailing
arrays and `mem.Vec`, but together they are a sixth of the interpreter,
and the dict is the hottest data structure of the whole program
(instance attributes, globals, class namespaces, kwargs). The dict also
needed three variants of lookup (generic key with builtin equality,
interned-string key with pointer compare first, user `__eq__` through
the VM) because hashing and equality can call guest code.
*Suggested fix:* `std.collections.IndexMap<V>` over a user `value` type
with hash/eq hooks that may fail and may re-enter (`Result`, `uses
reenter`), and a `std.num.BigInt` with an inline small case. Every
dynamic-language interpreter here has written its own hash table; Python
and Ruby need both of these.

### 3. A NaN box holds 7 pointer variants; Python has ~20 heap types

*Tried:* one variant per heap type, read `--explain-values` (very
helpful) and the layout rules: 7 48-bit blocks for pointers.
*Did instead:* `Str`, `List`, `Tuple`, `Dict`, `Inst`, `Func`, plus
`Obj(Gc<Box>)`, a catch-all object with a `kind` field and a union of
fields (`n m s: i64`, `a b c: Value`, `dict`, `base`, `big`, trailing
`extra: [Value]`) for classes, bound methods, iterators, generators,
cells, big ints, sets, modules, properties, `super` objects, slices.
Every Box costs 96+ bytes and every access to one is a kind check. It
also forced `Int(i32)`: an `Int(i48)` needs a 48-bit block, and there is
none left, which is why Python's ints needed a third representation
(item in the optimization table: medium ints).
*Suggested fix:* a documented pattern (or library support) for "one
pointer variant whose object carries a type tag", i.e. polymorphic
`Gc`: `object` types that share a header prefix and a `match` on an
object's type id, so `Obj(Gc<Any>)` could be matched into `Gc<Class>`,
`Gc<Gen>`, ... without a union struct. The `pointer` repr listed as not
built would be the other way.

### 4. The documented GC safepoint idiom gives handlers a stack frame

*Tried:* `if unlikely(ctx.heap.allocated > ctx.heap.threshold) { cold {
gc.collect(&ctx.heap); } }` at the top of allocating ops, as the
tutorial (chapter 7) shows; MiniJS does the same.
*Happened:* language.md §4 does say, under "Not built", that only cold
blocks ending in a control word are outlined; others stay inline. So the
`gc.collect` call sits in the handler, and `objdump` showed `push %rbp;
sub $0x10,%rsp` (and spills of the instruction word) at the *entry* of
GETITER, MAKECELL, NEWLIST, LISTAPPEND and the CALLMETHOD list-append
path, executed on every run of the op.
*Did instead:* `if unlikely(..) { cold { gc.collect(&ctx.heap); redo } }`
(24 ops). `redo` makes the block end in a control word, so it is
outlined, and re-running the op from the start is safe because the check
is its first statement; after a collection `allocated` is 0, which also
holds in stress mode, so it does not loop.
*Suggested fix:* outline non-terminating cold blocks too (the design
already describes it), or make the tutorial's idiom use `redo` and
explain why. A lint like `hot_gc` for "call on the fast path forces a
frame" would have found this and items 5–6 immediately.

### 5. `raise` (or a panic) in a `-> never` vm function's rare branch puts a frame on the fast path

*Tried:* `push_frame` checked the stack limit with
`if !ensure(top) { raise ops.exc(ctx, E_RECURSION, "...") }`.
*Happened:* building the exception (an allocation plus a call) inline in
the shared function gave the CALL handler `push %rbp; sub $0x10,%rsp`
on every call; similarly, `owner.base.unwrap()` in LOADMETHOD's cache
hit path pulled in `loimi.panic` calls, and slice indexing (`g.extra[i]`)
bounds-check panics gave FORITER (generator resume) and YIELD frames.
*Did instead:* `cold { raise .. }`, `owner.base ?? owner`, raw pointers
(`g.extra.ptr`) with an explicit `gc.write_barrier`. After that, CALL,
FORITER, GETATTR, LOADMETHOD, RET, JF and the arithmetic ops have no
frame (`scratch`-style survey in "Generated code").
*Suggested fix:* the same lint as item 4: report "this handler gets a
stack frame because of <call> at <line>" (`-W hot_frame`). The
information is in LLVM's output; a new user would never look.

### 6. Write barriers add a `push/pop` to every handler that stores a reference

*Happened:* `gc.write_barrier` (and the compiler's own barrier on
`obj.field = v`) calls `std.gc.remember` in its unlikely branch; that
call makes LLVM keep the stack 16-byte aligned with a `push %rax` at
entry and `pop` before each dispatch (SETATTR, SETINDEX, LISTAPPEND,
CALLMETHOD, YIELD). Two instructions per store-op execution, in a
non-generational heap where `remember` can never run.
*Suggested fix:* emit the `remember` call as an outlined tail (a cold
continuation that returns to the handler) or a `preserve_all` call, the
way cold blocks are outlined.

### 7. Generators: no `suspend`, and `next_pc` is subtle with `then`

*Tried:* generators as frames whose registers are copied into the
generator object on `yield` and back on resume, resumed in the loop by
FORITER (a `-> never` vm function that pushes the generator's frame) and
from host code by a nested entry (`list(g)`, `next(g)`).
*Happened:* this works and is fast, but it is a design the user must
invent (language.md §10 points to Mini's coroutines, which use nested
entries). One bug cost time: the resume function saved `next_pc` as the
return address, which for `FORITER .. then JMP` is the JMP word, so
every loop over a generator ended after the first item. Loimi's
definition is consistent (`next` skips only the op, not its `then`
word), but there is no name for "after this op and its `then` word",
and the `pc + 2` I ended up passing is exactly the hand-computed
address `next_pc` exists to avoid.
*Suggested fix:* `then_next_pc` (or make `next_pc` in a `then` op
mean after the pair, with a lint); and a short "generators in a register
vm" section in the docs: frame copied in/out, two resume paths,
`gen_done` flag for the host path.

### 8. Calling guest code from host code: correct, but expensive enough to design around

*Tried:* builtins and runtime helpers (`sorted(key=f)`, `__eq__` from
dict lookups, `__str__` from `print`, `__add__` from the arithmetic slow
path) call guest functions through a nested entry (`uses reenter`).
*Happened:* the effect system made this safe without thinking: every
function that might call back is `uses reenter`, handlers sync and
reload automatically, and traced locals in host code are rooted at
every such call. No GC bug was found by stress, verify or
`--gc-check-roots` at any point. But each nested run costs a sync, a
reload, a shadow-stack frame and an entry, so the hot dunder paths
(item in the optimization table) had to be redone as in-loop frames, and
generator consumption by builtins as Python code. MiniJS reached the
same "prelude in the guest language" solution independently.
*Suggested fix:* a library-level "call into the vm with a continuation"
helper for the in-loop case (push a frame whose result goes to a given
slot and continue at `next_pc`) would make the fast pattern the easy one.

### 9. `mem.sort_by` takes a comparator that can neither fail nor re-enter

*Tried:* `sorted(xs, key=...)` and `list.sort()` with Python `<`, which
can raise and can call `__lt__`.
*Happened:* `sort_by(cmp: block fn(*T, *T) -> i32)` has no effects and
no error path.
*Did instead:* a 40-line merge sort over `Vec<Value>` with a `Result`
comparator `uses reenter`.
*Suggested fix:* `mem.try_sort_by<T, E>(.., cmp: block fn(*T, *T) ->
Result<bool, E> uses reenter) -> Result<(), E>`.

### 10. Small language papercuts (each cost one edit-compile cycle)

| tried | error | did instead | suggestion |
|---|---|---|---|
| `var G: *A = 0 as *A;` | `the initializer of a module-level var must be a constant expression (literals, ...)` | `?*A = None` (or a struct with `..zero`, where null pointers *are* allowed) | accept `0 as *T`, which `..zero` already produces |
| `pub const B_LIST_APPEND: u32 = B.LAppend as u32;` | `constant initializer must be a constant expression` | `#[inline(always)] fn` | enum-to-int casts of payload-free variants are constants |
| `And(a, b) \| Or(a, b) => ..` | `bindings in \`\|\` patterns are not supported` | 9 arms split or rewritten with helper functions | support or-patterns with the same bindings |
| `let f = \|x\| ..;` local helper closure | `a closure can only be an argument for a block fn parameter ...` | top-level functions | allow non-escaping closures in `let` when only called |
| `struct WithItem { ctx: *Expr, var: ?*Expr }` | `expected identifier, found var, which is a reserved word` | `target` | (fine, the message says what to do) |
| `(if x { 1 } else { 0 }) == y` with `y: i32`; `for j in 0..3 { .. j + base_u32 .. }` | `mismatched types: expected i64, found i32` | annotations (`for j: u32 in 0..3`) | infer literal-only expressions from the other operand (2026-11-05: done for `if`/`match`/block operands; the `for` range still needs the annotation) |
| `std.lex.push_utf8(..)` without `import std.lex` | `unknown name std` | import it | allow fully qualified std paths, or suggest the import |
| `op DELGLOBAL(name: kst) : ABx` | `mismatched types: expected u8, found u16` at the encoder call | a dummy first operand `z: imm` | operands map to fields by position, which the docs say; an op with fewer operands than fields could warn when the operand lands in a narrower field than the format's widest |
| raising from a plain vm function | `control words are not allowed in vm functions; return a value instead` | return `bool`, raise in the caller | (clear) |

The compiler reports one error at a time; getting 10,000 freshly
written lines to build took about fifteen fix-and-rebuild cycles of ~10
seconds each, which was fine. (Fixed 2026-11-05: a build reports
every independent error, up to `--max-errors N`.)

### 11. Instance attributes as dicts: no help from std for the dict+IC combination

*Tried:* the brief asked for dicts as attribute storage, so `std.obj`
(shapes) was out. Inline caches on a dict needed my own validation
rules: the GETATTR cache is an entry number checked by comparing the
entry's key with the name constant (interned, so one word compare),
LOADMETHOD caches a class id plus a global class epoch plus the
owner's depth and entry, and a per-class flag disables method caching
once any instance shadows a class attribute.
*Happened:* Loimi's cache words made each of these a few lines, and the
families/`become` machinery was not needed (one generic op with an inline
check was enough). The friction is that nothing generic exists for
"dict-backed objects with caches", so each interpreter reinvents
validity rules, which are easy to get subtly wrong (the shadowing case
above is one).
*Suggested fix:* a `std.obj`-like helper over a user dict: "cache entry
index for key K in a dict whose key layout version is V".

### 12. `--vm-stats` super suggestions did not pay off here

*Tried:* the suggested supers for richards, raytrace, float, nbody, go.
*Happened:* `MOVE_MOVE_MOVE`, `LOADMETHOD_CALLMETHOD`,
`GETATTR_GETATTR(_MUL)` saved millions of dispatches and nothing
measurable (0.996 geomean). Python benchmarks are dominated by the
work in calls, attribute lookups and allocation, not dispatch. More
useful from the same output: the pair counts pointed at compiler
improvements (`LOADNIL; IS; JF` is `x is None` and became one op;
`MOVE; RET` was a missed operand).
*Suggested fix:* none for the tool; the docs already say supers rarely
help on their own.

### 13. Finding where time goes outside handlers

*Tried:* `perf`, `gdb`, `valgrind`, `/usr/bin/time` — none installed;
`std.prof` has no sampler.
*Did instead:* a counters array (`builtins.COUNTS`, printed with
`PYLOIMI_COUNTS=1`) in every runtime slow path, micro-benchmarks in
Python, and `objdump` of handlers. The counters found almost every win
in the table above (e.g. 1M LOADMETHOD misses in deltablue were static
methods called through the class).
*Suggested fix:* `--vm-stats` could count entries into each `cold`
block per op ("GETATTR.cold2: 1,017,633"), which is exactly the
question an interpreter writer asks next; a per-cold-block counter is
the same mechanism as the op counters.

## What worked well

- **The vm declaration.** Ops, operand kinds, cache layouts, families,
  `then JMP`, `inline for` over op tables (arithmetic, comparisons,
  compare-and-branch with register and constant forms are three
  templates). Encoders, the disassembler (`--disasm` was the main
  debugging tool) and the verifier came for free; adding the
  exception-handler `verify` block took ten lines.
- **Effects.** `uses reenter` propagated through the runtime and the
  compiler inserted every sync, reload and root. GC stress, generational
  verify and `--gc-check-roots` found nothing to fix in three different
  dispatch modes, in an interpreter where almost every slow path can run
  guest code and allocate.
- **`cold` with control words, and `-> never` vm functions.** The call
  path (`push_frame`, `call_slow` looping over bound methods, classes,
  callable instances and builtins), generator resume and in-loop operator
  methods are each one vm function, inlined on hot paths and shared on
  cold ones.
- **`on raise`.** Python's exception unwinding (handler table per
  prototype, frame popping, generator frames marked done, nested-run
  boundaries) is one 25-line handler.
- **`--explain-values`**, the `value_layout` warning and `block N` pins
  made the NaN-box decision quick and explicit.
- **Reference-checking against CPython.** Seeded `random` matches
  CPython bit for bit (Mersenne Twister and `randbelow` in ~100 lines),
  so chaos and go are checked against CPython's exact output.
- **Diagnostics** are precise and usually say what to write instead.
  Panics name the source line (`builtins.lm:158:46: unwrap of None`
  found an ordering bug in exception class setup in one run).

## Generated code

Release build, tail-call mode, x86-64. State registers: `r13` = pc,
`r14` = regs, `r15` = constants, `r12` = context, `rdi` = frame pointer;
the instruction word arrives in `esi`. Every handler ends in the same
4-instruction dispatch (`mov 0x4(%r13),%esi; add $0x4,%r13; movzbl
%sil,%eax; jmp *table(,%rax,8)`); cache words are skipped by the
`add` (GETATTR: `add $0xc,%r13`).

ADD (40 instructions in all, 23 on the int path), no stack frame, no
spills, the (Int, Int) test merged into one xor/or/shift:

```
mov    %esi,%eax
movabs $0xfff8800000000000,%rdx      ; Int tag
movzbl %ah,%ecx                      ; dst
shr    $0xd,%esi ; and $0x7f8,%esi
mov    (%r14,%rsi,1),%r8             ; a
...
mov    (%r14,%rsi,8),%rsi            ; b
xor %rdx,%r9 ; xor %rdx,%r10 ; or %r9,%r10 ; shr $0x20,%r10
jne    float_path
add    %r8d,%esi ; jo cold
or     %rdx,%rax ; mov %rax,(%r14,%rcx,8)
mov 0x4(%r13),%esi ; add $0x4,%r13 ; movzbl %sil,%eax ; jmp *table(,%rax,8)
```

GETATTR on an instance with a cache hit: 22 instructions, no frame
(tag test, `inst.dict`, bounds check of the cached entry number, one
compare of the entry's key with the name constant, load, store,
dispatch). CALL of a Python function with the right argument count:
about 40 instructions, no frame: tag test, compare of `proto.simple`
with the count, high-water and frame-limit checks, seven stores of the
new frame, then `jmp *table` of the callee's first word.

Handler survey (instructions in the whole function; frame setup counted
as `push`/`sub rsp` anywhere): MOVE 10, RET 17, GETGLOBAL_C 27, JF 36,
ADD/MUL/JNLT 40, GETINDEX 56, GETATTR 63, CALL 90, FORITER 163,
LOADMETHOD 169: no frame. SETATTR, SETINDEX, LISTAPPEND, CALLMETHOD:
one `push %rax` for the barrier's `remember` call (item 6). GETITER,
YIELD, NEWLIST, MAKECELL: frames from allocation (`gc.refill`), which
they need.

Build: 9.9 s for a tail-call release build, 12.6 s goto, 13.5 s switch,
peak 188 MB.

## Would the other examples have helped?

Skimmed after the interpreter worked. Yes, three things:
`examples/minijs/prelude.lm` is the same idea as my Python prelude
(builtins that call back written in the guest language, so natives never
re-enter); MiniJS's `verify handlers` block is what my handler table
was missing until the end; and Mini's COROUTINES.md would have shortened
the generator design. MiniJS also uses the inline-cold GC idiom of item
4, so copying it would not have avoided that frame. None of them has a
dict-as-object model or bigints, which were the bulk of the work.
