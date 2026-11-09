# Design note: compile-time memory safety for the standard library

How the compiler checks memory safety in the standard library, module by
module, and what it does not check. For contributors to the compiler and
to std.

**Status: built.** `#![safe]` modules, `#[raw]` functions and the GC
object rules of section 8.

Raw pointers (`*T`) need no `unsafe` anywhere in Loimi, and std is built
on them. A module that starts with `#![safe]` may still use `*T`, but
only as a reference to one live value: following it is allowed, making
it point anywhere else is not. Everything that can reach memory outside
that one value needs `unsafe { }`.

In summary:

- **Checked:** indexing and arithmetic on pointers, casts to pointers,
  other modules' pointer fields, C calls and `#[raw]` functions
  (section 1).
- **Not checked:** lifetimes of pointers stored in memory, and a few
  other holes (section 2). The separate pointer-stability check
  ([`lifetimes.md`](lifetimes.md)) covers pointers used within a
  function and its calls. ASan builds report most of the rest at run
  time (section 7).
- **GC objects** have three rules of their own, in every module
  (section 8).
- **Safe std modules:** `lex`, `parse`, `diag`, `text`, `hash`, `io`,
  `bc`, `map`, `dict`. The others (`mem`, `gc`, `obj`, `bigint`, `gil`,
  `num`, `math`, `os`, `rt`, `test`, `prof`, `debug`) are not.
- **Cost:** none for the check itself. The migrations' own changes
  (slices instead of pointer arithmetic, so bounds checks) measured
  within noise (section 6).

## 1. The rule

In a module marked `#![safe]` (an attribute at the top of the file),
outside `unsafe { }`:

| allowed | needs `unsafe { }` |
|---|---|
| `*p`, `p.f`, `p.read_unaligned()`, `p.atomic_*()` on a `*T` | `p[i]`, `p[a..b]`, `p + n`, `p - n`, `p += n`, `p.offset(n)` |
| `&place` (a local, a field, an element: `&a[i]` is bounds-checked) | `p.slice(n)` (a slice of unchecked length) |
| `new(&arena, v)`, `Vec`, slices, `?*T` | casts to a pointer: from an integer (`0 as *T`), another pointer type, a function or a `Gc` |
| passing, storing, comparing pointers; `p as u64` | casts that forge a tag or a reference: integer to `enum` or `value` |
| methods of other modules' structs; reading their non-pointer fields | reading a raw-pointer field of *another module's* struct (`v.ptr`), or a slice's `.ptr` |
| | writing any field of another module's struct that has raw-pointer fields (`v.len = 0`), `&v.len`, mutating methods on such a field (`a.chunks.push(..)`), or building one with a literal |
| this module's enums with pointer payloads | binding a pointer payload of *another module's* enum or `value` type in a pattern (`lib.Buf.Bytes(p, _)`; `_` is fine), or building such a variant |
| | `..zero` or a `var` without initializer that makes a null `*T` (use `?*T`), also inside arrays, tuples and structs, and null `fn`s and `Gc`s |
| `new(&a, v)` with an allocator whose `alloc_bytes` is `#[raw]` | `new(&a, v)` with any other allocator (nothing promises it returns `size` bytes) |
| | declaring a `vm` (its handlers are exempt: put it in another module) |
| | `assume(c)` and `unreachable` (undefined behavior if wrong; `trap()` is fine) |
| | calling (or naming) an `extern "C"` function or a function marked `#[raw]` |
| | calling an `#[unsafe]` function (as everywhere) |

Each error says what to write instead:

```
error: indexing a raw pointer (`p[i]`) is a raw-pointer operation: not allowed in a `#![safe]` module outside `unsafe { }`
  --> lexer.lm:12:5
note: use a slice (`[]T`, bounds-checked), a `Vec` method, or wrap it in `unsafe { }` with a comment saying why the index is in bounds
```

Each `unsafe { }` block gets a one-line comment saying why it is in
bounds or valid:

```loimi
// unsafe: write(2) reads the s.len bytes of `s`
unsafe { write(fd, s.ptr, s.len) }
```

### Why `*T` stays usable

Loimi has no separate reference type. Methods take `self: *T`, `&x` is a
`*T`, `new(a, v)` returns one, and `Vec.ptr_at(i)` hands one out.
Banning `*T` would ban methods.

What makes a pointer dangerous is where it points, and safe code cannot
change that. Every `*T` it holds came from `&place`, `new(..)` or a
function call. Pointer arithmetic, casts and slicing a pointer are the
only ways to move it. So `*T` in a safe module means what `&mut T` means
in Rust, minus the borrow checker.

### `#[raw]`: the contract for functions

A safe module may call any function of a module that is not safe, with
any argument it can build. That is sound only if the function is sound
for every such argument, that is, pointers to one value. A function is
marked `#[raw]` if it:

- reads or writes beyond one value (`mem.copy(dst, src, n)`);
- frees (`mem.release(p)`);
- returns memory that is not one valid value (`mem.alloc<T>(n)`: n
  zeroed `T`s, maybe not valid `T`s);
- needs a pointer from a particular allocator (`Malloc.free_bytes`);
- trusts an index or cache word without checking it (`obj`'s inline
  cache readers, such as `Model.load_ref(o, r)`).

Outside safe modules a `#[raw]` function is an ordinary function, so the
example interpreters are unaffected. In a safe module, calling it needs
`unsafe { }`, as `p[i]` does. Extern functions are `#[raw]` implicitly.
`new(a, v)` calls the `#[raw]` `a.alloc_bytes` itself with the right
size, so it is allowed.

### Other modules' structs

A struct with a raw-pointer field carries its module's invariants (for
`Vec`: `ptr` holds `cap` values, `len <= cap`), and that module's
`unsafe` blocks rely on them. So a safe module may read such a struct's
non-pointer fields (`v.len`) and call its methods, but may not:

- write its fields or take their address;
- read its pointer fields;
- build one with a literal.

A "raw-pointer field" is a field whose type holds a `*T` by value:
`*T`, `?*T`, tuples, arrays and options of them, and function types
taking or returning one. The property is transitive through struct and
enum fields, so a struct holding a `Vec` counts. Copying a whole value
is allowed (that is the aliasing part of the lifetime hole, section 2).
The rule applies to structs of other *safe* modules too.

### What is exempt

- vm handlers and vm functions (their operand accesses are unchecked
  because the verifier ran), and code generated from a `vm` declaration.
  Since handler bodies are exempt, a `#![safe]` module cannot declare a
  `vm` at all.
- Every module without `#![safe]`.

Generic functions are checked as their defining module says. A safe
module's generic body is checked; `mem.Vec.push` instantiated from a
safe module is not, since it is `mem`'s code.

## 2. What the rule does not check (the known holes)

1. **Lifetimes.** A `*T` or slice into a container is valid until the
   container changes. These all dangle after a push, insert, resize or
   free: `Vec.items()`, `Vec.ptr_at(i)`, `HashMap.get_ptr`,
   `OrderedMap.entry(e)` and `next`, `diag.SourceMap.file(id)`,
   `diag.Diagnostics.at(i)`. Also, `&local` returned from its function
   dangles, `Arena.free` invalidates everything `new(&arena, ..)` made,
   and a copy of a `Vec` shares its buffer and dangles when the original
   grows.
   - std's rule for its own APIs: *a container's methods leave the
     container valid* (`Vec.free` leaves an empty `Vec`, `HashMap.free`
     an empty map), so only references obtained earlier can dangle.
   - Uses within one function, through its calls, are compile errors in
     every module ([`lifetimes.md`](lifetimes.md)). A pointer stored in
     memory and loaded elsewhere is not checked.
   - Index-based APIs (`Vec.at(i)`, `OrderedMap` entry indices with
     `version`) are the safe alternative where they are cheap.
   - Built with `--sanitize=address`, most stale uses are reported at
     run time (section 7).
2. **GC references in host memory.** A `Gc<T>` kept where the collector
   does not scan (a `Vec<Gc<T>>` that is not a root) dangles after a
   collection. `gc.Heap` roots and `--gc-check-roots` are the tools.
3. **Trusted modules.** A safe module is only as safe as the functions
   it calls in modules that are not safe: those must honor the `#[raw]`
   contract. std's are audited (section 3); an example interpreter's are
   not.
4. **Zero values from elsewhere.** A safe module cannot make a null
   `*T`, but a module that is not safe can hand one over (in a struct it
   zeroed). A null `*T` faults on x86-64 and AArch64 Linux, but LLVM
   treats the load as undefined.
5. **Data races.** Not addressed. `std.gil` serializes guest threads;
   atomics are allowed on any `*T`.
6. **Invariants without raw pointers.** Another module's struct whose
   fields are only slices and integers can be written field by field. A
   module whose `unsafe` code relies on an invariant between such fields
   must keep them behind a raw pointer or check them. No std module does
   this.
7. **Asserted contracts.** `#[raw]` on a safe module's own function is
   an unchecked claim, like Rust's `unsafe impl`. For example, `new(&a,
   v)` trusts an `alloc_bytes` marked `#[raw]` to return `size` bytes.
8. **Overly strict cases.** A generic type of another module
   instantiated with the safe module's own pointer type (`lib.Box<*u8>`)
   counts as foreign: reading `b.v` is flagged though the pointer came
   from the safe module. In the other direction, a struct of the safe
   module reached through another module's generic type
   (`lib.Lookup<Mine>` bound in a pattern) is not followed.
9. **GC objects.** Section 8 covers stores through slices, zero-filled
   trailing arrays and who may write an object. Not covered: a module
   that is not safe can store through `&o.f` or `o.items.ptr` (raw
   pointers: calling `gc.write_barrier` is its job), and a slice of an
   object's trailing array read across a collection dangles if nothing
   else keeps the object alive (hole 1, on the GC heap).

## 3. Audit of std

`LOIMI_SAFETY_AUDIT=1 loimi build ..` checks every module as if it were
`#![safe]` and prints the number of raw-pointer sites per module and
kind. `LOIMI_SAFETY_AUDIT=sites` lists each site.

Run on std before the migration, it split std in two:

| group | modules (sites) |
|---|---|
| nearly pointer-free | `lex`, `parse`, `diag` (0), `hash` (1), `bc` (11), `text` (16), `io` (33) |
| pointers are the point | `mem` (68), `gc` (147), `obj` (48), `bigint` (295), `gil` (34), `map` (27), `dict` (41) |

Most `*T` uses everywhere are `p.f` through a pointer to a struct, which
the rule allows.

Use-after-free bugs it found, all fixed:

- `StrBuf.push_str(b.as_str())` (a buffer appending its own text) read
  freed memory when the push grew the buffer. It now uses `Vec.extend`
  (section 4).
- `HashMap.free` and `OrderedMap.free` kept the capacity of the table
  they released (section 5).

## 3a. Bypasses closed

Programs that tried to reach raw memory in a safe module without
`unsafe`, found by deliberately attacking the rule. Each is rejected,
and each is a `tests/fail/safe_*.lm`.

| attempt | why it is rejected |
|---|---|
| `let (p, _) = h.tup` / `h.tup.0` on another module's `(*u8, u64)` field | a field holding a pointer through a tuple, array, option or function type is a pointer field |
| `h.arr[0]` on `[*u8; 2]` (e.g. `gc.Heap.free`) | same |
| `let cb = h.cb; cb(&x)` on a `fn(*u8)` field | same |
| `match b { lib.Buf.Bytes(p, _) => p }` on another module's enum or `value` type | binding a foreign pointer payload; `Bytes(_, n)` is fine |
| `lib.Buf.Bytes(&one_byte, 100)` | building such a variant, like a struct literal |
| `o.inner = i` where `Outer { inner: Inner }`, `Inner { p: *u8 }` | "has raw-pointer fields" is transitive |
| `var p: *u8;` / `var s: S;` | zero-initialized: a null pointer (also a null `fn` or `Gc`, through arrays, tuples, structs) |
| `S { ..zero }` with a `[*u8; 2]` or `(*u8, u8)` field | same |
| `new(&a, v)` with an allocator whose `alloc_bytes` returns `&one_byte` | heap overflow; rejected unless `alloc_bytes` is `#[raw]` (`mem.Arena` and `mem.Malloc` are) |
| a `vm` declared in a safe module | handler bodies are exempt; declare it in a module of its own |

Attempts that were already rejected: `p[i]` in a closure, `defer`,
`inline for` body or generic instance; pointer casts to function or
slice types; `0 as *u8` in a constant or global; calling a `#[raw]`
`alloc_bytes` through an unbounded generic (checked per instance);
`free` on a `closure fn`; `x as V` for a `value` type.

Closing these needed no change to std's safe modules and changed no
example's IR.

## 4. The safe modules and the `#[raw]` contracts

| module | `unsafe` blocks | `#[raw]` | what the migration changed |
|---|---:|---|---|
| `lex`, `parse`, `diag` | 0 | | `#![safe]` only |
| `hash` | 0 | | `str.eq` is `mem.equal_bytes` (also no `memcmp` on a null pointer for empty strings) |
| `text` | 1 | | `StrBuf.push_str` is `bytes.extend(s)`; `Interner`'s slot table is a `Vec<u64>` (bounds-checked `at`/`set`); `parse_f64` copies into its buffer by a checked loop and calls `strtod` in `unsafe` |
| `io` | 4 | `free_file` | one `write_some(fd, s: []u8)` helper and one `format_g14` (the two C calls); `Out` buffers in a `Vec<u8>`; `read_file` (C stdio) is one `unsafe` block; `free_file` frees what `read_file` returned |
| `bc` | 0 | | `relax`'s position table is a `Vec<u64>` slice; `unreachable` → `trap()` |
| `map`, `dict` | section 5 | `map`'s `probe_*` core | section 5 |

The `unsafe` blocks of `text` and `io` are all C calls, each with its
justification:

```
text.lm  // unsafe: strtod reads the NUL-terminated copy in `buf` (s.len < 64) and points `end` into it
io.lm    // unsafe: write(2) reads the s.len bytes of `s`
io.lm    // unsafe: snprintf writes at most the 64 bytes it is told `buf` has
io.lm    // unsafe: C stdio on a NUL-terminated copy of `path`; fread fills at most the n bytes malloc'd
io.lm    // unsafe: the caller passes memory read_file malloc'd
```

**`Vec.extend(s)`**, added for the migration, appends a slice. It is
safe when `s` lies inside the vector's own buffer: if the buffer must
grow, the slow path finds `s` again in the new buffer before copying, so
`v.extend(v.items())` works. The fast path (room left) is a compare and
a `memcpy`. `Vec.filled(n, x)` was added too.

**The modules that are not safe.** `mem`, `gc`, `obj`, `bigint` and
`gil` are the core (allocator, collector, inline caches, limb
arithmetic, threads), where nearly every function handles pointers.
Their public contracts are documented, with `#[raw]` on the functions
that safe callers cannot satisfy:

| module | `#[raw]` functions |
|---|---|
| `mem` | `alloc`, `try_alloc`, `grow`, `release`, `copy`, `sort_by`, `try_sort_by`, `Arena.alloc_bytes`, `Malloc.alloc_bytes`, `Malloc.free_bytes`, `poison`, `unpoison` |
| `obj` | the inline cache API that trusts its cache words (slot refs, validity cells, Poly entry indices, `Hit`s): `Shapes.valid`, `Poly.add`, `MegaCache.new` (`n` a power of two), `put`, `free`, `Model.load_ref`, `store_ref`, `load_inline`, `load_out`, `proto_load`, `hit`, `poly_get`, `poly_get_at`, `poly_set`, `add_hit`. The rest of `obj`'s public API checks its indices or takes only values it made. |
| `gc` | `barrier`, `write_barrier` (they read the header before the object) |

`num`, `math`, `os`, `rt`, `test`, `prof` and `debug` are small modules,
mostly C interop.

## 5. `std.map` and `std.dict`

### `std.map`

`std.map` is `#![safe]`. Its `unsafe` blocks rest on the tables' own
invariants: `slots` holds `cap` slots and `cap` is a power of two; an
`OrderedMap`'s `idx` holds `mask + 1` slots naming entries below `used`,
and `used <= cap <=` the allocation. Only `map`'s own code can break
them, since other modules cannot write the fields of a struct with raw
pointers.

- `HashMap` goes through one private accessor, `slot(i)`, that masks
  the index (`m.slots + (i & (m.cap - 1))`, so it is in bounds; LLVM
  folds the second mask), plus `new`, `grow` and `free`.
- The CPython-style probing core (`probe_find`, `probe_slot_of`,
  `probe_lookup`, `probe_put`, `probe_rebuild`, `probe_kill`,
  `probe_find_with`) takes a raw index pointer, a mask and an entries
  pointer. Each is `#[raw]`, with one `unsafe` body and a documented
  contract.
- `OrderedMap` has its allocation, resize, probing, `clear` and `free`
  in `unsafe` blocks, and a bounds-checked entry accessor.

Two API contracts came out of the migration:

- `OrderedMap.entry(e)`, `set_value(e)` and `remove_at(e)` panic ("not
  a live entry index") unless `e` is a live entry. Before, they read and
  wrote out of bounds, and `remove_at` of a dead entry looped forever.
- `HashMap.free` and `OrderedMap.free` point the map at a shared zeroed
  table (a fresh one for slots over 512 bytes), which `grow`, `resize`,
  `clear` and `free` never write or release. The map stays usable and
  the next insert allocates (`tests/run/map_free_reuse.lm`).

### `std.dict`

`std.dict` is `#![safe]` by a different argument. `Dict` and `KeyDict`
are GC objects holding `Gc`s and trailing arrays, not raw pointers, so
the field rule does not protect them. Instead, dict's memory safety does
not depend on its fields (`mask`, `used`, the index slots):

- The probes are private safe functions over the trailing arrays as
  slices (`find_in`, `lookup_in`, `put_in`, `slot_of`, `rebuild_in`, and
  `key_*` versions for `KeyDict`), not map's `#[raw]` `probe_*`.
- Each probe takes its mask from the index's own length (`mask_of(idx)`:
  panics on an empty index, else length − 1). LLVM then removes the
  bounds check from the slot reads; entry reads keep one compare per
  candidate key. `d.mask` is kept up to date for the API but not
  trusted.
- A bad field can cause a panic or an endless probe, never an access
  outside the tables. `remove_at(e)` panics unless `e` is live; `key`,
  `value`, `hash_at` and `set_value` are bounds-checked but not checked
  for liveness (a dead entry reads as zero bytes).

It has two `unsafe` blocks and no `#[raw]` function:

```
dict.lm  // unsafe: memset writes exactly the s.len * size_of<T>() bytes of `s`.          (zero)
dict.lm  // unsafe: reads only the size_of<K>() bytes of the one K at `k` (as u64 words when the size is a multiple of 8).   (key_live)
```

Stores through the trailing arrays passed as slices (`rebuild_in`,
`zero`) get their write barrier from the compiler after the call
(section 8.1). `place()` stores a whole entry through `d.ents.e[e]`,
which gets the barrier as any object store does.

**Cost** (Python, release, best of 10, twice): geomean 1.002 and 0.999
(range 0.978–1.017). `richards` is about 1% slower in every run (+0.3%
to +1.3%), from the per-candidate bounds check in attribute lookups that
miss the inline caches. Bounds-checking the slot reads as well cost
`richards` 2.3–3% and was dropped. No vm op handler's machine code
changed.

## 6. Cost

**The check itself costs nothing at run time.** Adding `#![safe]` to
`lex`, `parse` and `diag` and `#[raw]` to `mem` left the release IR of
every example interpreter identical, apart from source locations in
panic messages.

**The migrations change IR only in the migrated functions:**

- `hash.str.eq` (one more compare, for the empty string);
- `text.StrBuf.push_str` (now `Vec.extend`), `text.Interner.*`
  (bounds-checked slot reads), `text.parse_f64` (a checked copy loop);
- `io.print*`, `io.write_*` and `io.Out.*`;
- `bc.Code.relax`.

Measured on the interpreters that call them at run time (release,
`--align-functions=64`, base and new interleaved). The machine was
shared, so single benchmarks moved by up to ±8% between runs; reruns of
the outliers came back within 1.5%.

| suite | runs | new/base geomean | range |
|---|---:|---:|---|
| Python (11 programs, string-heavy `hexiom`, `chaos`) | best of 7 | 0.996 | 0.97–1.02 |
| Lox (10 programs) | best of 5 | 1.005 | 0.92–1.08 |
| Prolog (30 programs, output through `io.Out`) | best of 3 | 0.991 | 0.92–1.04 |
| Python front end: 20,000 function definitions, parse and compile | best of 7 | 1.008 | |

All within noise. Mini, MiniJS and Scheme use the changed functions only
at compile time or for printing.

**`std.map`'s migration** made `HashMap.insert` about 5% smaller in
every interpreter (the probe returns the slot pointer, so callers do not
index again). `get` is unchanged or ±1 instruction, and no op handler
changed size. Python, Lox and Prolog benchmarks showed no measurable
change.

## 7. Catching dangling pointers at run time (ASan builds)

`--sanitize=address` reports a use of freed memory, with the stacks of
the use, the free and the allocation. That covers most of the lifetime
hole (section 2, hole 1) by itself:

- A `Vec` or map that grows calls `realloc`, which under ASan always
  moves and frees the old buffer.
- `Vec.free`, map `free` and `Arena.free` call `free`.

What ASan cannot see is memory a container keeps but no longer uses.
std marks that memory with ASan's poisoning API
(`__asan_poison_memory_region`, `__asan_unpoison_memory_region`), so a
stale access is reported as `use-after-poison`. The calls are statements
under `#[cfg(sanitize_address)]`, which `--sanitize=address` sets; every
other build drops them.

### What std poisons

Through `std.mem.poison` and `unpoison` (both `#[raw]`, ASan builds
only):

| container | poisoned | stale use caught |
|---|---|---|
| `mem.Vec` (so `text.StrBuf`, `diag.SourceMap`, `diag.Diagnostics`, every `Vec`-backed table) | the spare capacity `[len, cap)`: on `with_capacity`, after growth, on `pop`, `truncate`, `clear`; `push`, `extend` and `filled` unpoison what they write | `ptr_at(i)`/`last` pointers and `items()` slices kept across `pop`, `truncate`, `clear`; reads of spare capacity through the raw buffer |
| `mem.Arena` | each chunk when allocated; `alloc_bytes` unpoisons what it hands out | reads and writes past an arena allocation |
| `map.OrderedMap` | entries `[used, cap)`: on creation, after `resize`, on `clear` | `entry`, `get_ptr`, `next` pointers kept across `clear` |
| freed memory (ASan itself) | — | anything kept across growth or `free` of any container, and `Arena.free` |

Not reported:

- a `HashMap` slot pointer kept across `remove` or `clear` (the slots
  stay in the table, and probing reads them);
- an `OrderedMap` entry kept across `remove_at` (dead entries stay below
  `used` until the next resize);
- objects in the GC heap (use `--gc-check-roots`);
- anything in an interpreter's own allocators.

Code outside `std.mem` that writes a `Vec`'s spare capacity before
raising `len` must unpoison it, under the same `cfg`. `std.bigint`
(limbs written through `space(v)` and `room(n)`) and the collector's
remembered set (`rs.objs.ptr[n] = obj`) do.

### Results

One bug was found: `Arena.free` released its chunks but kept the current
chunk's pointer, offset and size. The next `alloc_bytes` wrote into
freed memory, and a second `free` freed it twice. `Arena.free` now
resets them, so the next allocation starts a new chunk
(`tests/run/arena_free_reuse.lm`).

No example interpreter tripped a report. Every test program of Python,
Prolog, Scheme, Lox, MiniJS, Stak, Mini and the Wasm C tests ran clean
under `--sanitize=address`, as debug builds and (for most) as release
builds, normally and with `--gc-stress` (about 600 runs).

### Tests

`tests/asan/*.lm`, run by `asan_stale_pointers` in `tests/golden.rs`
(without `ulimit -v`, which ASan cannot run under): one program per
table row that must be reported, and one that uses every poisoned
container correctly and must run clean.

### Rejected

- *Checked handles* (a generation counter per container, compared at
  every dereference): needs a second, debug-only pointer representation
  and a check at every `*p` and `s[i]`.
- *Filling freed memory with a pattern*: gives garbage rather than a
  report, and `malloc` soon reuses the block.

## 8. GC objects

Three rules for GC objects, in every module, not only safe ones: a
missing write barrier breaks the collector wherever it happens.

### 8.1 Stores through slices of an object's trailing array

The compiler emits the write barrier after a store of a reference into
a place it can see is inside an object: `o.f = v`, `o.items[i] = v`,
`o.items[a..b][i] = v`.

A slice is a plain (pointer, length) pair. Without a rule, `let s =
o.items; s[i] = v`, or passing `o.items` to a function that stores into
it, would store a young object into an old one with no barrier, and a
minor collection would free it. (Verify mode reports this as "missing
write barrier": `tests/run/gc_barrier_slice_arg.lm`.)

**The rule** is checked in `src/objref.rs`, after type checking. An
*object slice* is `o.items`, a sub-slice of it, or `o.arr[..]` of an
array field, whose elements are of a traced type (`Gc`, a `value` with
references, a struct holding one). It may be:

- indexed, sub-sliced, iterated, or asked for `.len`. Stores
  `o.items[i] = v` get the barrier. `o.items.ptr` is a raw pointer, and
  stores through it need `gc.write_barrier`.
- bound by `let` to a local that is only read through.
- passed straight to a function, as described below.

For each function, every parameter of type `[]T` or `*T` with traced `T`
gets two yes/no answers: may the function store a reference through it
(also through `.ptr`), and may it keep it beyond the call (in a `var`, a
field, a return value, a capture)? Passing it on to a parameter that
does either counts as doing it. This is a fixpoint over the call graph;
a function value or an extern counts as both.

- A call that may store through an object slice gets the barrier on the
  object after the call. That call must not collect (`uses gc`): a minor
  collection between the store and the barrier would miss the young
  object.
- A call that may keep it is an error: the copy would outlive the
  barrier.

Everything else is an error: keeping the slice (`var`, return, field,
capture), storing through a `let` that holds one, passing a held one to
a function that stores or keeps, passing one to a function that keeps
it or to one that may collect and store.

```
error: a slice of an object's traced trailing array is stored through (held in a local): a store through it would skip the write barrier
  --> tests/fail/gc_slice_kept.lm:11:5
note: store through the object (`o.items[i] = v` gets the write barrier); a slice of it may be bound by `let` and read, or passed to a function that cannot collect (the barrier follows the call)
```

Details:

- A store of a value without references (`Nil`, an integer field of an
  entry) needs no barrier and does not count as a store.
- In safe modules, `&o.f` of a traced place follows the same rule
  (taking an address is not a raw-pointer operation there).
- Elsewhere `&o.f` is a raw pointer. But passing it (also as a method's
  receiver) to a call that may collect and store through it is an error
  in every module, since the barrier after the call would not cover it.
- `unsafe { }` exempts both.

**Effect on the examples.** No example had a missing barrier: the only
stores through object slices were `std.dict`'s, which called the barrier
by hand. Python's `value.list_items(l)` returned a slice, so its 11
callers now write the slice inline. `std.dict`'s hand-written barrier
after `rebuild_in` is replaced by the compiler's.

**A use-after-free found on the way** (the lifetime hole, not the
barrier). Python's `list.index`, `list.count` and list/tuple `<` held a
slice of the list's array while calling `__eq__`, which may grow the
list and collect. The old array was freed while the scan went on reading
it (`examples/python/tests/mutating_eq.py` failed under `--gc-stress`).
They now read the list's current items at each step, as CPython does
(`value.seq_at`).

The rule does not catch this, since the slice is only read.
`LOIMI_OBJREF_AUDIT=1` lists object slices passed to, or held across,
calls that may collect, for review. The remaining ones in the examples
keep their object alive.

**Rejected:**

- A slice type that remembers its object (`ObjSlice<T>`, three words):
  changes every signature that takes `[]Value` and makes slices of host
  memory pay.
- A barrier that finds the object from an interior address: costs every
  store through any traced slice, register files included.

Both are heavy for what the audit found: six such slices in three
interpreters, all but one read-only.

### 8.2 Zero-filled trailing arrays

`new(h, X { .. }, n)` allocates zeroed memory. For `[Gc<T>]` that is n
null references, which outside `unsafe` cannot be made; for a `value`
type it is whatever the word 0 encodes. So zero-filling is allowed only
where zero is a valid element:

- numbers, `bool`, options (None), slices;
- raw pointers, outside safe modules;
- tuples, arrays and structs of those (without private `__` fields);
- an enum, if its variant with discriminant 0 has a valid-when-zero
  payload;
- a `value` type, if the word 0 encodes a variant (a nanbox's unboxed
  `0.0`, a low-tag or boxed variant with tag 0) whose payload is not a
  reference.

Generic objects are checked per instance. Elsewhere, write one of:

- `new(h, X { .. }, n, v)`, which fills the array with `v` (a loop of
  stores; no barrier, since the object is young);
- `?Gc<T>` elements;
- `unsafe { new(h, X { .. }, n) }`, for code that sets every element
  before anything reads it.

A length that is the literal `0` needs none of these.

All value types of the example interpreters zero-fill to their unboxed
`0.0`, and `std.dict`'s entries to all-zero keys (dead entries). Five
places allocate `Gc` arrays and fill them right after: Lox's and Mini's
`CLOSURE` (upvalue cells) and the `Protos` tables of the Mini, MiniJS
and Scheme compilers. They use `unsafe { new(..) }` with a comment,
because a fill value adds a loop to `CLOSURE` and `?Gc` elements add a
test to every upvalue access.

### 8.3 The objects of a `#![safe]` module

A safe module can rely on its own structs' fields only if no other
module can write them. The field rule of section 1 protects only structs
with raw pointers, and only from safe modules. Two more rules:

- **In every module**, an `object` declared in a safe module is built
  (`new(h, M.X { .. })`) and written (`o.f = v`, `o.items[i] = v`,
  `&o.f`) only by that module's code or in `unsafe { }`. Reading is
  unrestricted. `std.dict` needed this: Python wrote `m.popped =
  m.ents.e.len` to mark an attribute dict full, and now calls
  `m.use_up_room()`.
- **In a safe module**, another module's `object` is treated like a
  struct with raw pointers: writing its fields or trailing array, taking
  their address and building one need `unsafe`.

Modules that are not safe can still write a safe module's objects
through raw pointers and through slices of their trailing arrays. They
are trusted, as for any `#[raw]` contract (section 2, hole 3).

Rejected: field visibility (`pub(read)`). Loimi's only field privacy is
the file-private `__` prefix, and tying objects to their safe module
needs no new syntax.

### 8.4 Cost

The release IR of Lox, Mini, MiniJS, Prolog, Scheme, Stack, Wasm, the
Mini embedding, the regex library and the tutorial is unchanged (panic
locations aside). Python's changes are where intended:

| function | change |
|---|---|
| `std.dict.Dict/KeyDict.resize`, `Dict.copy` | the compiler's barrier after `rebuild_in` replaces dict's own call (same code after inlining) |
| `Dict/KeyDict.remove_at`, `Dict.clear` | one barrier (a header-byte test) after zeroing entries; `KeyDict.resize` also logs the old entries (it passes them to dict's `unsafe` key test, counted as a store) |
| `builtins.call`, `construct`, `module_call`, `ops.setslice`, `vm.Py.op.TOTUPLE` | `list_items` inlined by hand (0–2 instructions difference) |
| `builtins.call_method_builtin` (`list.index`, `count`), `ops.order`, `ops.seq_order`, new `value.seq_at`/`seq_len` | the use-after-free fix |
| `value.attrs_replace`, new `KeyDict.use_up_room` | the dict field write moved into `std.dict` |

No vm op handler's machine code changed. Python's 11 benchmarks (best of
3, `bench/python/compare.py --exe`): geomean new/base 0.999 (range
0.976–1.019), within noise.
