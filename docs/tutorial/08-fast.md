# 8. Making it fast

Step 7 is a complete interpreter. This chapter is about measuring it and
making it faster, with the tools Loimi provides and the lessons from
Mini's performance log ([`bench/mini/RESULTS.md`](../../bench/mini/RESULTS.md)),
where these techniques were tried on a Lua subset that ended up faster
than PUC-Lua on every benchmark.

Code: [`examples/tutorial/step8/`](../../examples/tutorial/step8), and
benchmarks in [`step8/bench/`](../../examples/tutorial/step8/bench).

## Measure first

`step8/bench/run.py` builds steps with `--release` and times four
programs: `fib` (calls), `loop` (integer and float arithmetic on
locals), `sieve` (arrays), `strings` (allocation). It runs the
executables interleaved and keeps the best of N runs, because a shared
machine's load changes during a run and affects whichever binary
happens to run then.

```sh
examples/tutorial/step8/bench/run.py -n 5 step7 step8
examples/tutorial/step8/bench/run.py -n 5 step8 -- --dispatch=goto
```

Treat differences under about 5% as noise unless they repeat. Mini's log
reports many changes that seemed to help in one run and were within
noise in an interleaved A/B; it is the rule rather than the exception.

All numbers below are seconds, best of 5, on a shared 8-core x86-64
machine.

## Dispatch modes

The same source, three ways:

| benchmark | tailcall | goto | switch |
|---|---:|---:|---:|
| fib | 0.359 | 0.377 | 0.398 |
| loop | 0.367 | 0.368 | 0.378 |
| sieve | 0.749 | 0.790 | 0.865 |
| strings | 0.146 | 0.144 | 0.148 |

Tail calls are the default for a reason, but the differences are
smaller than folklore suggests on a modern branch predictor, and Mini
saw `goto` win on some benchmarks. Since it costs one flag to find out,
find out.

## Reading the generated code

Every handler is its own function, named `vm.Pip.op.NAME`, so
`objdump -d --disassemble=vm.Pip.op.ADDK pip` shows one op, and any
sampling profiler attributes time per op. `loimi build --emit-llvm -o
pip.ll` writes the LLVM IR, where a handler looks like this:

```text
define internal preserve_nonecc void @"vm.Pip.op.ADDK"(ptr %p0, ptr %p1, ptr %p2, ptr %p3, ptr %p4, i32 %insn)
```

The parameters are the context, the four state variables and the
instruction word. The entry's result, a `Result<(), str>`, does not
come back in registers: `exit` stores it to a thread-local slot that
the entry reads, so handlers return `void` (an entry returning a
scalar such as an `i64` returns it directly). Things to look for in
a hot handler's assembly:

- **no stack frame** (`push`, `sub $..., %rsp`): a plain call on any
  path gives the handler a frame on *every* path. Mini found this in
  GETINDEX and SETINDEX; the cure is `cold`. Calls LLVM would add
  itself are not a worry in tail-call handlers: they are compiled so
  that a zeroing or copying loop stays a loop instead of becoming a
  `memset`/`memcpy` call (a call to a C function you declared, such as
  `memset` itself, is still a call: put it in `cold` or use a loop).
  `loimi build --release -W handler_frame` finds these for you: it
  warns about every handler whose fast path sets up a frame and names
  the calls that may have caused it. On step 8 it lists five handlers:
  DEFGLOBAL, NEWLIST, NEWARRAY and PRINT, which run rarely, and
  SETINDEX, where the frame is a `push`/`pop` pair around the call to
  `std.gc.remember` in the compiler's write barrier (chapter 7);
- **no reloads of state** from memory;
- **one dispatch sequence per exit**: load the next word, advance `pc`,
  index the table, jump.

## Constant operands

`--vm-stats` builds an interpreter that counts every op executed and
every adjacent pair, and prints them at exit:

```sh
loimi build examples/tutorial/step7/main.lm --release --vm-stats -o pip-stats
./pip-stats examples/tutorial/step8/bench/fib.pip
```

```text
      59721406   23.5%  LOADI
      29860703   11.8%  MOVE
      14930351    5.9%  ADD
      29860702   11.8%  SUB
      29860700   11.8%  GETGLOBAL_C
      29860703   11.8%  JNLT
      29860703   11.8%  CALL
      29860703   11.8%  RET

suggested superinstructions (each at least 1% of executed ops; none overlaps another or an
existing super, so they can all be added together):
    super GETGLOBAL_C_LOADI_SUB_CALL = GETGLOBAL_C; LOADI; SUB; CALL;    // 29860700, 11.8%, saves 89582100 dispatches
    super LOADI_JNLT = LOADI; JNLT;    // 29860703, 11.8%, saves 29860703 dispatches
    ...
```

A quarter of all instructions load a small constant into a register for
the next instruction to use (`n < 2`, `n - 1`). Lua's answer is
instructions that take a constant operand directly. In Loimi that is an
op whose second operand has kind `kst` instead of `reg`, and with
`inline for` the K forms come from the same template:

```loimi
    inline for (name, fname, checked, sym, kind, bkind) in [
        ("ADD", "ADD_F", "add_checked", +, Arith.Add, reg),
        ("SUB", "SUB_F", "sub_checked", -, Arith.Sub, reg),
        ("MUL", "MUL_F", "mul_checked", *, Arith.Mul, reg),
        ("ADDK", "ADDK_F", "add_checked", +, Arith.Add, kst),
        ("SUBK", "SUBK_F", "sub_checked", -, Arith.Sub, kst),
        ("MULK", "MULK_F", "mul_checked", *, Arith.Mul, kst),
    ] {
```

The compiler uses them when the right operand is a number literal whose
constant index fits the 8-bit field:

```loimi
/// If `n` is a number literal whose constant index fits an 8-bit
/// operand, that index.
fn C.small_const(c: *C, n: *Node) -> ?u8 {
    let v = match n.expr {
        Int(i) => if i >= -2147483648 and i <= 2147483647 { Value.Int(i as i32) } else { return None; },
        Float(x) => Value.Num(x),
        _ => return None,
    };
    let k = c.consts.add(v)?;
    if k < 256 { Some(k as u8) } else { None }
}
```

Compare-and-branch gets K forms too. With a constant on the right, `x >
k` cannot be flipped into `k < x` without a register for `k`, so there
are four orders:

```loimi
    inline for (name, bkind, test, slow) in [
        ("JNLT", reg, x < y, less(a, b, false)),
        ("JNLE", reg, x <= y, less(a, b, true)),
        ("JNLTK", kst, x < y, less(a, b, false)),
        ("JNLEK", kst, x <= y, less(a, b, true)),
        ("JNGTK", kst, x > y, less(b, a, false)),
        ("JNGEK", kst, x >= y, less(b, a, true)),
    ] {
```

The MOVE after every call (chapter 6) goes too. When the call's
destination is the topmost temporary, it can be the call's base:

```loimi
            // If `dst` is the topmost temporary, it can be the base
            // itself, and the result needs no MOVE.
            let mark = c.regs.mark();
            let in_place = c.regs.is_temp(dst) and dst + 1 == mark;
            let base = if in_place { c.regs.alloc(args.len as u32); dst } else { c.regs.alloc(1 + args.len as u32) };
```

Together: fib 0.466 → 0.369 s, loop 0.494 → 0.407 s.

## Float-first quickening

`loop` does `x = x * 0.5 + 1.0`. The arithmetic handlers test
`(Int, Int)` first, so every float operation pays for a failed integer
test. Testing floats first would slow down integer code instead. Mini's
answer (RESULTS.md, item 10) is to let each *site* decide: the arithmetic
ops become families whose generic member is integer-first and turns
itself into a float-first member the first time it sees two floats.

```loimi
        family #name(dst: reg, a: reg, b: bkind) : ABC {
            generic {
                match (a, b) {
                    (Int(x), Int(y)) => if let Some(r) = x.#checked(y) { dst = Int(r); next },
                    (Num(x), Num(y)) => { dst = Num(x sym y); become #fname; next }
                    _ => {}
                }
                cold { dst = arith_slow(ctx, kind, a, b)?; next }
            }
            #fname {
                match (a, b) {
                    (Num(x), Num(y)) => { dst = Num(x sym y); next }
                    (Int(x), Int(y)) => if let Some(r) = x.#checked(y) { dst = Int(r); next },
                    _ => {}
                }
                cold { dst = arith_slow(ctx, kind, a, b)?; next }
            }
        }
```

Both members handle every case, so a site never needs to switch back
(a site that flip-flops would be worse than either). In the generated
code, `become` is a single byte store into the code stream, on the
float path of the generic member:

```text
<vm.Pip.op.ADDK>:
  ...
  movq   %r8,%xmm0
  movq   %rsi,%xmm1
  addsd  %xmm0,%xmm1
  movsd  %xmm1,(%r14,%rcx,8)
  movb   $0xc,0x0(%r13)          ; become ADDK_F
  mov    0x4(%r13),%esi          ; next
  ...
```

loop 0.407 → 0.372 s; the integer benchmarks do not change.

`--disasm-after` shows the quickened code:

```text
fn step:
    0  MULK_F dst=r3 a=r0 b=k0
    1  ADD_F dst=r2 a=r3 b=r1
    2  RET src=r2
```

## Superinstructions

`--vm-stats` ends with suggested superinstructions, already written as
declarations, and, commented out, frequent ones that overlap them (only
one of two overlapping supers can fuse where they meet). On step 8
without its supers:

```text
    super GETGLOBAL_C_SUBK_CALL = GETGLOBAL_C; SUBK; CALL;    // 29860700, 18.2%, saves 59721400 dispatches
    super ADD_RET = ADD; RET;    // 14930351, 9.1%, saves 14930351 dispatches
...
//  super SUBK_CALL = SUBK; CALL;    // 29860702, 18.2%, saves 29860702 dispatches; overlaps GETGLOBAL_C_SUBK_CALL
//  super GETGLOBAL_C_SUBK = GETGLOBAL_C; SUBK;    // 29860700, 18.2%, saves 29860700 dispatches; overlaps GETGLOBAL_C_SUBK_CALL
```

Step 8 takes the pairs, which are simpler to read; the three-op super
works the same way.

A **superinstruction** runs several ops with one dispatch:

```loimi
    super GETGLOBAL_SUBK = GETGLOBAL; SUBK;
    super GETGLOBAL_C_SUBK = GETGLOBAL_C; SUBK;
    super ADD_RET = ADD; RET;
    super ADDK_JMP = ADDK; JMP;
    super ADD_JMP = ADD; JMP;
```

The compiler builds each super's handler by inlining the parts. The
generated peephole pass `Pip.optimize(v)` fuses matching sequences
in place: the first word's opcode becomes the super, and the following
words stay as they are, so a jump into the middle still runs the
original instruction, and fusion needs no control-flow analysis.

It takes the proof from `Pip.verify` and returns it: fusing keeps the
code valid, so it stays verified. (The verifier accepts supers too, and
chapter 9's fuzzer checks that optimized code still verifies.)
`Pip.optimize_words(words)` does the same on a plain slice.

```loimi
    // Verify every function before running anything, then fuse
    // superinstructions: `Pip.optimize` takes the proof and returns it.
    for i in 0..prog.protos.len {
        let p = prog.protos.at(i);
        match Pip.verify(p.words, p, names.names.len as u32) {
            Ok(v) => p.code = Pip.optimize(v),
```

`Pip.optimize` goes left to right and fuses, at each instruction, the
longest super that starts there, then moves on. That is why the
suggested `SUBK_CALL` is not declared: it shares the `SUBK` with
`GETGLOBAL_SUBK`, and in fib every `SUBK; CALL` is part of
`GETGLOBAL; SUBK; CALL` (the call `fib(n - 1)`), where `GETGLOBAL_SUBK`
takes the `SUBK` first, so `SUBK_CALL` would never run. Declaring both
gets a `super_overlap` warning from the compiler.

Quickening and supers interact: `Pip.optimize` runs on generic code, so
it sees `GETGLOBAL; SUBK`. When the GETGLOBAL part quickens, the super's
head is rewritten to the super with that part replaced
(`GETGLOBAL_C_SUBK`) if one is declared, and otherwise split back into
its parts. Hence the two declarations.

And the result:

| benchmark | without supers | with supers |
|---|---:|---:|
| fib | 0.369 | 0.359 |
| loop | 0.372 | 0.367 |
| sieve | 0.770 | 0.749 |
| strings | 0.147 | 0.146 |

Within noise. This matches Mini's experience: ten supers taken from
`--vm-stats` "changed nothing beyond noise" because dispatch on a modern
core is well predicted; later, supers helped only together with other
changes (12% on one benchmark, after compare-and-branch ops got their
polarity in the opcode). Supers are cheap to try in Loimi, which is the
point: try them, measure, keep what pays.

## Overall

| benchmark | step 7 | step 8 |
|---|---:|---:|
| fib | 0.466 | 0.359 |
| loop | 0.494 | 0.367 |
| sieve | 0.783 | 0.749 |
| strings | 0.142 | 0.146 |

`fib` and `loop` are 23-26% faster. `sieve` is bounded by array
accesses and `strings` by allocation and copying, which none of these
changes touched.

`--vm-stats` also counts how often each `cold` block is entered, which
shows a fast path that misses a common case. On `sieve`:

```text
cold blocks entered:
       2211180  MUL (cold block at examples/tutorial/step8/vm.lm:238:17)
       2211180  JNLE (cold block at examples/tutorial/step8/vm.lm:368:13)
            29  NEWARRAY (cold block at examples/tutorial/step8/vm.lm:397:68)
```

`i * i` overflows `Int(i32)` for every prime above 46340, so MUL goes
cold and returns a float, and the comparison `j <= n` that follows
mixes a float with an integer, which neither of JNLE's fast cases
covers. Each is about 0.5% of the ops executed, so not worth a
fast case here; on `strings`, every `+` is ADD's cold block (string
joining), which is expected. Step 6 (no GC) runs `fib` in 0.404 s against step 7's
0.466: the price of clearing a callee's registers in CALL (chapter 7)
and of the larger value type.

## A checklist

From Mini's log, roughly in order of payoff for a Lua-like interpreter:

1. **A size-class allocator** for small objects (Mini: 2x on
   allocation-heavy benchmarks). `std.gc` has one.
2. **Inline caches** on property and global access (Mini: up to 40% on
   object-heavy code). Chapter 5.
3. **Interned strings**, so string equality and table-key comparison
   are one compare (Mini: hash_strings 1.7x).
4. **Constant operands** (`ADDK`, `x < 2`), and **compare-and-branch**
   instead of compare-then-test (Mini: 4-13%; Pip: above).
5. **Conditions compiled to jump chains**: `a and b` in an `if` jumps
   straight to the exit, without building a boolean (Mini: queens -18%).
   Pip has no `and`/`or` yet; this is how to add them.
6. **Per-site quickening** for float-heavy code (Mini: mandelbrot -15%,
   nbody -12%), with every member complete so sites never flip-flop.
7. **Counted integer `for` loops**, as in Lua 5.4: compute the
   iteration count once, so the loop op has no overflow or direction
   test (Mini: 4-9%).
8. **One allocation per object**: trailing arrays in the object instead
   of a separate buffer (Mini: binary_trees 15-25%).
9. **Polarity in the opcode** (JLT and JLTN rather than a polarity
   operand), and **supers**, measured.
10. Things that did not pay in Mini: lazy register clearing, float-first
    compares, forwarding on every super (`#[forward]` helps only on
    latency-bound chains), mixed-type cases in hot handlers.

Through all of it: keep the hot path to the one or two common cases,
put everything else in `cold`, and look at the assembly of the handlers
that `--vm-stats` says matter.

Next: [testing an interpreter](09-testing.md).
