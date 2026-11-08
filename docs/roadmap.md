# Design decisions

Questions the design left open, and how they were settled. The numbers
are those of the original list, which other documents cite ("roadmap,
question 2"). What exists is in [`status.md`](status.md); benchmark
numbers are in [`results.md`](results.md).

2. **Moving GC: no. The heap is non-moving, generational with sticky
   mark bits** (2026-11-02; opt-in per heap, `heap.set_generational(true)`;
   [`stdlib/gc.md`](stdlib/gc.md#generational-mode-sticky-mark-bits-non-moving),
   [`lowering.md`](lowering.md#10-gc-barriers-and-allocation)).
   - *Why not moving.* Every interpreter keeps raw pointers into objects
     across safepoints (Mini's table parts and upvalue pointers, MiniJS's
     `Arr.ptr`, every `k` pointer into a constant array); a copying
     nursery would need each re-derived after every `gc` call. The
     non-moving scheme needs only a write barrier.
   - *The barrier* (object logging, one header byte test after each
     reference store) costs within noise with generations off: Mini
     1.004, MiniJS 1.006 by geometric mean, worst MiniJS `richards` +3%.
   - *Generations on against off* (with epoch marks and aging for large
     objects, 2026-11-03): Mini 0.982, MiniJS 0.987, Scheme 0.986; all
     three turn them on. Since 2026-11-04 marking sets per-chunk bitmaps
     and nothing sweeps (`stdlib/gc.md`, "Mark bitmaps").
   - *Rejected: aging small objects.* Keeping every first survivor young
     traced growing live data twice (`nboyer` 0.798 s against 0.762 s,
     `earley` +9%, `mperm` +7%); small objects are promoted at their
     first survival. There is no card marking: a large array that is
     stored into is retraced whole by each minor collection.

4. **Threads: one thread in a heap at a time.** Independent states run
   on different threads with no locks (the runtime's per-thread state is
   initial-exec TLS, within noise of plain globals: 0.99–1.01). Several
   threads share one heap and interpreter state through `std.gil`, a lock
   per heap handed over at safepoints the interpreter polls (Mini with
   `--cfg gil`: 1.2–1.4% geomean single-threaded). Free threading
   (several threads running vm code in one heap at once) is not
   supported (`language.md` §10.1, Threads).

6. **Copy-and-patch JIT: built, and the template-JIT line stopped there**
   (2026-11-02 to 11-05; [`design/copy-and-patch.md`](design/copy-and-patch.md)).
   `--jit-stencils` generates a baseline JIT from the vm's handlers with
   no extra source: 0.76–0.83 of the interpreters' time by geometric
   mean (Mini, MiniJS, Scheme; Prolog 0.94). Re-patching on quickening,
   patched constant operands and a one-register cache (1.5% slower) each gained nothing; what remains
   needs register allocation across instructions, which the
   ahead-of-time traces do (`design/trace-tier.md`).

7. **A checked register window: no** (2026-11-03). The verifier is the
   trusted-index mechanism (an operand kind's `verify` clause states the
   whole range it is used for); a run-time check on Mini's FOR* and RET1
   accesses cost 2–9% (fib +5%, nbody +7% even with one check per op).

9. **Embedding API: the interpreter's own `export "C"` functions.**
   `loimi build --lib`/`--shared` builds the library and `--header`
   declares them (`language.md` §10.1): an opaque state pointer
   (`MiniState *`), payload-less enums as error codes, `loimi_str` for
   strings, `#[c_const(..)]` on an export for its `const` pointers.
   `Gc<T>` and `value` types do not cross into C: hosts refer to values by position
   on a rooted value stack, as Lua's stack API does, and natives call back
   through nested entries (`uses reenter`). The header is
   `extern "C"`-guarded, usable from C++.

10. **Immediates wider than a format's word.** The wasm interpreter
    keeps 64-bit constants and table and segment indices in inline cache
    words (`cache Imm64 { v: u64 }`), written after `emit`; the verifier
    cannot see cache words, so such indices are bounds-checked at each
    execution. The chosen fix is `const` cache fields: set by the
    emitter, readable by the verifier, kept by `deopt`.

11. **The cost of a three-operand op.** `F64_MUL d, a, b` is 14
    instructions (6 to decode three 16-bit fields, 3 to load, multiply
    and store, 5 to dispatch) against wasm3's 5. Accumulators
    (language.md §7.4; wasm 0.91 of its time) and `#[forward]` supers
    remove the store-to-load round trips. Cheaper decoding does not pay
    on x86-64 (2026-11-06, `design/speed-audit.md` item 2): pre-scaled
    byte-offset fields measured 1.003 and offset-threaded dispatch 1.008
    on PolyBench, and direct threading would save only the table load
    that the predicted indirect jump already hides.
