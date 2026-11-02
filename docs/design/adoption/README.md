# Adopting std.bc and std.obj in the examples

> **Historical.** `std-bc.patch` is a record of the plan, not something
> to apply: it predates `Verified<V>` and still uses the old API
> (`Mini.verify(code, proto) -> ?u64` with the code in a trailing array,
> `Mini.optimize(code)` on raw words). The examples have since moved on;
> see `language.md` §6 for verification as it is now.

**Applied (2026-11-02), on the current code rather than with this patch:**
Mini's compiler uses std.bc (with `Mini.set_label`, labels for `break`
and `if` exits), std.lex, std.parse and std.diag; Scheme's code generator
uses std.bc. The emitted bytecode is byte-identical to the hand-written
versions. The patch below is kept as the historical plan.

`std-bc.patch` moves the code generators of `examples/mini` and
`examples/scheme` onto `std.bc` (apply with `git apply`). It was checked
on a copy of both interpreters: every Mini script and conformance test
(89 files) and every Scheme test (19 files), with and without GC stress,
produce the same output, errors and exit status as before.

## Mini (`compile.lm`: 1630 → 1587 lines; `value.lm`: +6)

| Today | With std.bc |
|---|---|
| `FuncState.code`, `.lines` (two `Vec`s kept in step by `Parser.emit`) | `code: bc.Code<u32>`; `emit` is `set_loc` + `op` |
| `p.emit(0); // inline cache word` after `GETGLOBAL`, `GETFIELD`, `SETFIELD`, `SELF` (6 places) | removed: `op` reserves the cache words from `Mini.op_size` |
| `Parser.patch`: JMP vs 16-bit field by hand, range check | `patch_jump` over two `bc.Field`s, `code.patch` |
| `Parser.constant`: linear search, string-content comparison, a chunk-wide `strings` map | `consts: bc.Pool<V>` (strings are already interned by `value.new_str`, so bit equality is enough; the `strings` map was redundant) |
| `nactive`/`freereg`/`maxreg` and `reserve`/`free_reg` | `regs: bc.Regs` (`try_alloc`, `free`, `leave`, `activate`, `end_stmt`, `frame_size`) |
| `p.fs.lines.truncate`, `p.fs.lines.set(at, line)` | `code.truncate`; `emit_line(w, line)` |
| `finish`: copy loops for code and lines | `copy_into`, `locs_into` |

`value.lm` gains `V.hash`/`V.eq` and `impl hash.Hash for V;` (by bits).

## Scheme (`compile.lm`: 1713 → 1698 lines; `value.lm`: +6)

| Today | With std.bc |
|---|---|
| `FS.code`, `.lines`, `Gen.emit` | `bc.Code<u32>` |
| `freereg`/`maxreg`, `Gen.alloc` | `bc.Regs` (Scheme has no locals in the allocator's sense: `regs.top` is saved and restored as `freereg` was) |
| `Gen.patch` | `patch_jump` + `code.patch` |
| `Gen.constant` (linear search) | `bc.Pool<V>` |
| `finish` copy loops | `copy_into`, `locs_into` |

## Further steps (not in the patch)

- Both compilers keep one `u32` line per code word in the `Proto`.
  `code.encode_locs` makes a delta-encoded table (about 2 bytes per line
  change), read with `bc.line_at(table, pc)` where the vm reports an
  error (Mini `vm.lm` line 840, Scheme `vm.lm` line 54). That changes the
  `Lines` object to bytes and the two lookups: a memory saving, not a
  line saving.
- Mini's peephole checks (`at + 1 == p.pc()` in `jump_if`, `concat`,
  `cond`) could use `code.last()`, which also refuses when a jump
  targets the current pc. The current checks are correct for Mini's
  grammar, so the patch leaves them.
- Labels (`code.label`/`bind`/`jump`) could replace Mini's `breaks`
  vector and Scheme's `patch_all` lists; both are short already.
- `code.thread_jumps` after `finish` would retarget jumps to jumps
  (`if` at the end of a loop body); measure before adopting.

## std.obj

Neither example has record-like objects with symbol keys, so neither
should adopt std.obj as is:

- Mini's tables are Lua tables: arbitrary keys, `t.x = nil` deletes,
  `next` iterates array and hash parts. Shapes would turn every
  non-last deletion into dictionary mode, and the field cache it would
  replace (`GETFIELD`: slot index plus one key compare) already costs
  about what a shape check does. Using std.obj would mean a hybrid table
  (shape part for string keys, hash part for the rest) and would add
  code to `value.lm`, not remove it.
- Scheme's records (`define-record-type`) compile to fixed vector
  indices, and globals to fixed indices; there is nothing to look up by
  name at run time.

std.obj is for a JS-, Python- or Self-style example, where it replaces
the object representation, the property lookup and the cache logic
(`tests/run/std_obj_ic.lm` shows the vm side).
