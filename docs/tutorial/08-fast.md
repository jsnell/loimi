# 8. Making it fast

Step 7 is a complete interpreter. This chapter measures it and makes it
faster, using Loimi's tools and the lessons from Mini, the Lua-like
example interpreter. Mini's performance log,
[`bench/mini/RESULTS.md`](../../bench/mini/RESULTS.md), records each
technique tried and what it gained.

The code is in [`examples/tutorial/step8/`](../../examples/tutorial/step8),
and the benchmarks in [`step8/bench/`](../../examples/tutorial/step8/bench).

## Measure first

`step8/bench/run.py` builds steps with `--release` and times four
programs:

| Program | What it exercises |
|---|---|
| `fib` | calls |
| `loop` | integer and float arithmetic on locals |
| `sieve` | arrays |
| `strings` | allocation |

It takes turns running each executable, and keeps the best of N runs.
That is because the load on a shared machine changes during a run, and
slows down whichever program happens to be running at the time.

```sh
examples/tutorial/step8/bench/run.py -n 5 step7 step8
examples/tutorial/step8/bench/run.py -n 5 step8 -- --dispatch=goto
```

Treat differences under about 5% as noise unless they repeat. In Mini,
many changes seemed to help in one run, but were within noise when
measured side by side. That is the rule, not the exception.

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

Tail calls are the default for a reason. But on a CPU with a modern
branch predictor the differences are smaller than often claimed, and in
Mini `goto` won on some benchmarks. Trying another mode costs one
flag, so try it.

## Reading the generated code

Every op is its own function, named `vm.Pip.op.NAME`. So
`objdump -d --disassemble=vm.Pip.op.ADDK pip` shows one op, and any
sampling profiler reports time per op.

`loimi build --emit-llvm -o pip.ll` writes the LLVM IR, where an op
looks like this:

```text
define internal preserve_nonecc void @"vm.Pip.op.ADDK"(ptr %p0, ptr %p1, ptr %p2, ptr %p3, ptr %p4, i32 %insn)
```

The parameters are the context, the four state variables and the
instruction word. The op returns `void`: the entry's result, a
`Result<(), str>`, is passed back through memory. (An entry that
returns a single number, such as an `i64`, returns it directly.)

In the assembly of a hot op, check for three things:

- **No stack frame** (`push`, `sub $..., %rsp`). A plain call on any
  path gives the op a frame on *every* path. The fix is to move the
  call into `cold`.
- **No reloads of state** from memory.
- **One dispatch sequence per exit**: load the next word, advance `pc`,
  index the table, jump.

`loimi build --release -W handler_frame` finds stack frames for you. It
warns about every op whose fast path sets up a frame, and names the
calls that may have caused it. On step 8 it lists five ops:

- DEFGLOBAL, NEWLIST, NEWARRAY and PRINT, which run rarely
- SETINDEX, where the frame is a `push`/`pop` pair around the call to
  `std.gc.remember` in the write barrier (chapter 7)

You don't need to worry about calls that LLVM would add on its own.
In tail-call mode, ops are compiled so that a loop that zeroes or
copies memory stays a loop, instead of becoming a call to `memset` or
`memcpy`. A call to a C function you declared yourself, such as
`memset`, is still a call: put it in `cold`, or use a loop.

## Constant operands

`--vm-stats` builds an interpreter that counts every op it runs, and
every pair of ops that run one after the other. It prints the counts
when it exits:

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

A quarter of all instructions load a small constant into a register,
for the next instruction to use (`n < 2`, `n - 1`). Lua's answer is
instructions that take a constant operand directly. In Loimi, that is
an op whose second operand has kind `kst` instead of `reg`. With
`inline for`, these "K" forms come from the same template:

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

The compiler uses them when the right operand is a number literal, and
its index in the constant pool fits the 8-bit field:

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

Compare-and-branch gets K forms too. With a constant on the right,
`x > k` can't be flipped into `k < x` without a register for `k`. So
there are four K forms, one per comparison:

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
destination is the topmost temporary, it can serve as the call's base:

```loimi
            // If `dst` is the topmost temporary, it can be the base
            // itself, and the result needs no MOVE.
            let mark = c.regs.mark();
            let in_place = c.regs.is_temp(dst) and dst + 1 == mark;
            let base = if in_place { c.regs.alloc(args.len as u32); dst } else { c.regs.alloc(1 + args.len as u32) };
```

Together, these take fib from 0.466 to 0.369 s, and loop from 0.494 to
0.407 s.

## Float-first quickening

`loop` does `x = x * 0.5 + 1.0`. The arithmetic ops test `(Int, Int)`
first, so every float operation pays for a failed integer test. Testing
floats first would slow down integer code instead.

The answer, taken from Mini, is to let each instruction decide for
itself. The arithmetic ops become families. The generic member tests
integers first, and turns itself into a float-first member the first
time it sees two floats:

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

Both members handle every case, so an instruction never needs to switch
back. (One that kept switching back and forth would be slower than
either.) In the generated code, `become` is a single byte store into the
code, on the float path of the generic member:

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

This takes loop from 0.407 to 0.372 s. The integer benchmarks don't
change.

`--disasm-after` shows the quickened code:

```text
fn step:
    0  MULK_F dst=r3 a=r0 b=k0
    1  ADD_F dst=r2 a=r3 b=r1
    2  RET src=r2
```

## Superinstructions

A **superinstruction** ("super") runs several ops with one dispatch.

`--vm-stats` ends with suggested supers, already written as
declarations. After them, commented out, it lists frequent supers that
overlap the suggested ones. Only one of two overlapping supers can be
used where they meet. Here is the output for step 8 without its supers:

```text
    super GETGLOBAL_C_SUBK_CALL = GETGLOBAL_C; SUBK; CALL;    // 29860700, 18.2%, saves 59721400 dispatches
    super ADD_RET = ADD; RET;    // 14930351, 9.1%, saves 14930351 dispatches
...
//  super SUBK_CALL = SUBK; CALL;    // 29860702, 18.2%, saves 29860702 dispatches; overlaps GETGLOBAL_C_SUBK_CALL
//  super GETGLOBAL_C_SUBK = GETGLOBAL_C; SUBK;    // 29860700, 18.2%, saves 29860700 dispatches; overlaps GETGLOBAL_C_SUBK_CALL
```

Step 8 uses pairs, which are simpler to read. The three-op super works
the same way:

```loimi
    super GETGLOBAL_SUBK = GETGLOBAL; SUBK;
    super GETGLOBAL_C_SUBK = GETGLOBAL_C; SUBK;
    super ADD_RET = ADD; RET;
    super ADDK_JMP = ADDK; JMP;
    super ADD_JMP = ADD; JMP;
```

The compiler builds each super by inlining its parts. The generated
function `Pip.optimize(v)` finds matching sequences and fuses them in
place. The first word's opcode becomes the super, and the following
words stay as they are. So a jump into the middle of the sequence still
runs the original instruction, and fusing needs no analysis of jumps.

`Pip.optimize` takes the proof from `Pip.verify` and returns it, since
fused code is still valid. (The verifier accepts supers too, and
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

`Pip.optimize` goes from left to right. At each instruction it fuses
the longest super that starts there, then moves on.

That is why the suggested `SUBK_CALL` is not declared. In fib, every
`SUBK; CALL` is part of `GETGLOBAL; SUBK; CALL` (the call
`fib(n - 1)`). `GETGLOBAL_SUBK` takes the `SUBK` first, so `SUBK_CALL`
would never run. Declaring both gets a `super_overlap` warning from the
compiler.

Quickening and supers work together. `Pip.optimize` runs on code that
has not been quickened yet, so it sees `GETGLOBAL; SUBK`. When the
GETGLOBAL part quickens, the super becomes the matching super with that
part replaced, `GETGLOBAL_C_SUBK`. If there is no such super, it is
split back into its parts. That is why both are declared.

And the result:

| benchmark | without supers | with supers |
|---|---:|---:|
| fib | 0.369 | 0.359 |
| loop | 0.372 | 0.367 |
| sieve | 0.770 | 0.749 |
| strings | 0.147 | 0.146 |

The difference is within noise. Mini had the same experience: ten
supers taken from `--vm-stats` made no difference beyond noise, because
a modern CPU predicts dispatch well. Supers helped Mini only together
with other changes (12% on one benchmark). Supers are cheap to try in
Loimi, which is the point: try them, measure, and keep what pays.

## Overall

| benchmark | step 7 | step 8 |
|---|---:|---:|
| fib | 0.466 | 0.359 |
| loop | 0.494 | 0.367 |
| sieve | 0.783 | 0.749 |
| strings | 0.142 | 0.146 |

`fib` and `loop` are 23–26% faster. `sieve` is limited by array
accesses, and `strings` by allocation and copying, which none of these
changes touched.

`--vm-stats` also counts how often each `cold` block runs. This shows
when a fast path misses a common case. On `sieve`:

```text
cold blocks entered:
       2211180  MUL (cold block at examples/tutorial/step8/vm.lm:238:17)
       2211180  JNLE (cold block at examples/tutorial/step8/vm.lm:368:13)
            29  NEWARRAY (cold block at examples/tutorial/step8/vm.lm:397:68)
```

`i * i` overflows `Int(i32)` for every prime above 46340. So MUL takes
its cold path and returns a float. The comparison `j <= n` that follows
then compares a float with an integer, which neither of JNLE's fast
cases covers. Each is about 0.5% of the ops run, so it is not worth a
fast case here.

On `strings`, every `+` runs ADD's cold block (joining strings), which
is expected.

The GC has a cost too. Step 6, which has no GC, runs `fib` in 0.404 s,
against step 7's 0.466 s. The difference comes from clearing the
callee's registers in CALL (chapter 7) and from the larger value type.

## A checklist

What paid off in Mini, roughly from most to least, for a Lua-like
interpreter:

| Technique | Gain in Mini |
|---|---|
| **A size-class allocator** for small objects. `std.gc` has one. | 2× on allocation-heavy benchmarks |
| **Inline caches** on property and global access (chapter 5) | up to 40% on object-heavy code |
| **Interned strings**, so comparing strings or table keys is one compare | 1.7× on hash_strings |
| **Constant operands** (`ADDK`, `x < 2`), and **compare-and-branch** instead of compare then test | 4–13% |
| **Conditions compiled to jumps**: `a and b` in an `if` jumps straight past the body, without building a boolean. Pip has no `and`/`or` yet; this is how to add them. | 18% on queens |
| **Per-instruction quickening** for float-heavy code, with every family member handling every case | 15% on mandelbrot, 12% on nbody |
| **Counted integer `for` loops**, as in Lua 5.4: compute the number of iterations once, so the loop op needs no overflow or direction test | 4–9% |
| **One allocation per object**: arrays at the end of the object instead of in a separate buffer | 15–25% on binary_trees |
| **Separate opcodes for "jump if true" and "jump if false"** (JLT and JLTN) instead of an operand that says which, and **supers**, measured | |

Things that did not pay off in Mini:

- clearing registers lazily, in the root scanner
- comparisons that test floats first
- register forwarding (`#[forward]`) on every super; it helps only on
  chains where each op waits for the previous one's result
- mixed-type cases in hot ops

Through all of it: keep the hot path to the one or two common cases,
put everything else in `cold`, and look at the assembly of the ops that
`--vm-stats` says matter.

Next: [testing an interpreter](09-testing.md).
