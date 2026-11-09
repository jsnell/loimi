# 9. Testing an interpreter

An interpreter has more ways to be wrong than most programs:

- The compiler can emit wrong code.
- An op can mishandle a rare combination of types.
- The GC can miss a root that matters once in a million allocations.
- The verifier can accept code that walks off the end of the register
  file.

This chapter collects the testing this tutorial has used, and adds two
things: unit tests written in Loimi (`std.test`), and a bytecode fuzzer,
which finds memory-safety bugs.

The code is in [`examples/tutorial/step9/`](../../examples/tutorial/step9).
It is the interpreter from step 8 with a step limit added, plus the
fuzzer in `fuzz.lm` and the unit tests in `unit_test.lm`.

## Conformance tests and the test matrix

Each step has a `tests/` directory. In it, `NAME.pip` is a program, and
`NAME.expected` is its output (stdout and stderr together). Tests named
`err_*` must also exit with an error.

`examples/tutorial/check.sh` builds each step and runs every test:

- in all three dispatch modes (`tailcall`, `goto`, `switch`), because
  each is generated differently and any one of them can be wrong in its
  own way
- from step 7 on, a second time with `--gc-stress`, which collects at
  every safepoint. That turns a missing root into a wrong answer or a
  crash (chapter 7).

```sh
examples/tutorial/check.sh            # every step
examples/tutorial/check.sh step7      # one step
examples/tutorial/check.sh -u step7   # rewrite the .expected files
```

Use `-u` when the output changes on purpose, and read the diff before
committing it.

`check.sh` does not use `--gc-check-roots` (chapter 7). After a change
to rooting or to the root scanner, it is worth repeating the stress
runs with a build made with `loimi build main.lm --gc-check-roots`. It
reports a bad root where a plain stress run might only print a wrong
answer. Step 9's tests pass in that mode, and `loimi test
--gc-check-roots unit_test.lm` checks the unit tests the same way.

Write tests for the cases the fast paths skip:

- `2147483647 + 1` (overflow into a float)
- `-7 % 3`
- `1 == 1.0` and `nil == false`
- arithmetic on every wrong type
- a global read after the globals table has grown (`recache.pip`)
- an `err_` test for every error message

If your language already has an implementation, take the expected
output from that implementation, not from yours. Mini's conformance
tests are Lua programs whose `.expected` files are PUC-Lua's output.
MiniJS's come from node.

## Unit tests

Conformance tests see the interpreter from outside. To test its parts,
such as the lexer, the compiler's output or a helper with an edge case,
write Loimi functions marked `#[test]`. Step 9's are in `unit_test.lm`:

```loimi
#[test]
fn lexes_two_char_operators() {
    var arena = mem.Arena.new();
    defer arena.free();
    var lx = lexer.Lexer.new("a <= b != 1.5;", 0, &arena);
    let kinds = [T.Name, T.Le, T.Name, T.NotEq, T.Float, T.Semi, T.Eof];
    for i in 0..kinds.len {
        let t = test.ok(lx.next());
        test.assert(t.kind == kinds[i]);
        if t.kind == T.Float { test.eq(t.float, 1.5); }
    }
    lx.free();
}
```

```sh
loimi test examples/tutorial/step9/unit_test.lm          # every test
loimi test examples/tutorial/step9/unit_test.lm arith    # names containing "arith"
```

`loimi test` replaces the file's `main` with one that runs each test in
turn. It prints `test NAME ... ok`, or `FAILED` with the reason, and
exits with status 1 if any test failed. A failing assertion or any
other panic, such as a failed bounds check or an `unwrap` of `None`,
fails only its own test.

The assertions in `std.test`:

| Assertion | Checks that |
|---|---|
| `test.eq(got, want)` | the two values are equal |
| `test.assert(cond)` | the condition is true |
| `test.ok(result)` | a `Result` is `Ok`, and returns its value |
| `test.some(option)` | an option is `Some`, and returns its value |
| `test.has(text, part)` | `text` contains `part` |

A failed assertion reports the line of the test that called it:

```text
test arithmetic ... FAILED
    unit_test.lm:79:9: test.eq failed
        got:  7
        want: 8
```

A test can run the interpreter in the same process. Such a test
declares `uses gc`, like any function that may collect. The interpreter
prints with `io.print`, so the test captures stdout to compare it:

```loimi
    var out = text.StrBuf.new();
    // `print` writes to stdout: capture it.
    test.capture_start();
    let r = Pip.run(&m, prog.main);
    test.capture_end(&out);
```

```loimi
#[test]
fn arithmetic() uses gc {
    for i in 0..2 {
        let stress = i == 1;
        test.eq(run("print 1 + 2 * 3;", stress), "7\n");
        test.eq(run("print 2147483647 + 1;", stress), "2147483648.0\n");
    }
}
```

`test.golden(text, path)` compares text with a file. `loimi test
--update` rewrites the file instead. This suits output that changes on
purpose, such as the compiler's bytecode: the test `sum_bytecode`
disassembles a loop and compares it with `tests/sum.disasm`.

`check.sh` runs each step's `*_test.lm` in every dispatch mode.

## A step budget, as a `variant`

The fuzzer runs random programs, and random programs often loop
forever. The usual fix is a step counter, checked in the interpreter's
jumps, which every normal run then pays for.

Loimi's **variants** avoid that cost. A variant is a second dispatch
table, whose ops run some extra code before the real op:

```loimi
    variant budget wraps [JMP, JF, JNLT, JNLE, JNLTK, JNLEK, JNGTK, JNGEK, JNEQ, JEQ, JNEQK, JEQK,
                          CALL, ADDK_JMP, ADD_JMP] {
        before {
            ctx.steps -= 1;
            if ctx.steps < 0 { cold { raise "step budget exhausted" } }
        }
    }
```

Only the ops that can jump are wrapped, because every loop goes through
one of them. The other ops are not changed. Supers are ops too, so the
ones that contain a jump are listed.

A second entry switches to the variant's table before running the first
instruction:

```loimi
    entry run_budget(m: *Machine, main: *Proto) -> Result<(), str> uses gc {
        use_variant budget;
```

`run` uses the normal table, and its ops don't know the variant exists.
On the benchmarks, step 9 runs as fast as step 8. The same mechanism
gives you a tracer, a debugger's breakpoint check or a coverage counter,
without a second copy of the interpreter.

`wraps all` would also work, and builds just as fast in every dispatch
mode. But wrapping only the 15 ops that can jump keeps the step
counting out of straight-line code.

## The fuzzer

The verifier is what makes it safe for ops to skip bounds checks
(chapter 3). If a `verify` clause is missing or wrong, the vm reads or
writes outside its arrays. The normal tests never exercise this,
because the compiler only emits valid code. The fuzzer feeds the
verifier *invalid* code, and runs whatever it accepts.

Completely random code is almost always rejected at the first word,
which tests little. Holes hide in code that is close to valid. So
`fuzz.lm` starts from real programs: it compiles a test program,
changes one to three words, and verifies the result:

```loimi
/// Change one word of `p`'s code (before it is verified).
fn mutate(r: *Rng, p: *Proto) {
    let i = r.below(p.words.len);
    var w = p.words[i];
    let k = r.below(100);
    if k < 40 {
        // One operand byte: usually a small number, sometimes anything.
        let shift = 8 * (1 + r.below(3));
        let v = if r.chance(80) { r.below(p.nregs as u64 + 3) } else { r.below(256) };
        w = (w & ((255 << shift) ^ 0xFFFFFFFF)) | ((v << shift) as u32);
    } else if k < 65 {
        // Another opcode, same operands.
        w = (w & 0xFFFFFF00) | (r.below(Pip.NUM_OPS) as u32);
    } else if k < 85 {
        w = w ^ (1 << r.below(32));
    } else if k < 95 {
        // Another target, if the word is a jump.
        let off = r.below(p.words.len + 4) as i64 - i as i64 - 2;
        if let Some(j) = Pip.set_label(w, off) { w = j; }
    } else {
        w = r.next() as u32;
    }
    p.words[i] = w;
}
```

The generated functions keep this short: `Pip.NUM_OPS` and
`Pip.set_label` come from the vm. With `-v`, the fuzzer prints why each
program was rejected (`e.write`), and the bad word (with the generated
`Pip.disasm_insn`):

```text
rejected: word 6 (GETGLOBAL): operand `n` (gname) fails `(it as u32) < nnames`
    6  GETGLOBAL dst=r0 n=g6 [cache slot=0]
rejected: word 2: invalid opcode
    2  <invalid 0xa9d828b4>
rejected: word 3 (JMP): operand `target` (label) fails `is_insn_boundary(pc + it)`
    3  JMP target -> 196616
```

The fuzzer changes a prototype's `words`. `verify_all` verifies them and
stores each proof in the prototype's `code`, as `main` does, so
`run_budget` can only start code that passed. Programs that pass are
optimized with `Pip.optimize` and run with the step limit.

Besides looking for crashes, the fuzzer checks that the code still
verifies at two points:

- after `Pip.optimize` has fused supers
- after running, when instructions have been quickened and caches hold
  whatever the program left in them

```loimi
    if !verify_all(&prog.protos, nnames, true) {
        st.rejected += 1;
    } else {
        for i in 0..prog.protos.len {
            let p = prog.protos.at(i);
            p.code = Pip.optimize(p.code);
        }
        if !verify_all(&prog.protos, nnames, false) { bug(seed, "code does not verify after Pip.optimize"); }
        match Pip.run_budget(&m, prog.main) {
            Ok(_) => st.finished += 1,
            Err(msg) => if msg.eq("step budget exhausted") { st.budget += 1; } else { st.raised += 1; },
        }
        if !verify_all(&prog.protos, nnames, false) { bug(seed, "code does not verify after running"); }
    }
```

Build the fuzzer with AddressSanitizer, which works in all three
dispatch modes. Run it in batches: each program's memory is freed, but
the lexer's tables are not, so a long batch grows.

Limit its memory too. A program that allocates without limit can take
over the whole machine before anyone notices. `ulimit -v` does not
work with ASan, which reserves terabytes of address space at startup.
Instead, set ASan's own limit, which stops the process with a report
when its memory use passes the limit:

```sh
cd examples/tutorial/step9
loimi build fuzz.lm --sanitize=address -o fuzz
export ASAN_OPTIONS=hard_rss_limit_mb=2000
./fuzz 1 2000 tests/*.pip
./fuzz 1 2000 tests/*.pip --gc-stress
```

For a build without ASan, `ulimit -v` works as usual.

```text
3000 programs: 1861 rejected by the verifier, 572 finished, 293 raised an error, 274 ran out of steps
```

About a third of the changed programs pass the verifier. That is the
useful range: with too few, the ops are never reached, and with too
many, the changes are not testing the verifier.

`cargo test` runs 3000 programs, without ASan. A longer run of 200,000
programs under ASan, and 100,000 more with GC stress, found one real
problem: `array(2147483647)` asked the allocator for 16 GiB.

The step limit can't prevent that, because it limits time, not memory.
Anything a single op can grow needs its own limit. Strings are the
other case: a loop that joins a string to itself doubles it each time,
so it reaches gigabytes in about 30 iterations, well within any step
limit. Step 9 limits arrays and strings to 2^24 elements:

```loimi
        // The limit was found by the fuzzer: array(2147483647) asked the
        // allocator for 16 GiB.
        if let Int(len) = n and len >= 0 and len <= MAX_LEN {
```

In Mini, the fuzzer found nine holes in the `verify` clauses. All were
counts whose range was never checked, such as CALL's argument count,
SETLIST's length, and CONCAT with fewer than two operands. Pip's
versions, CALL's `nargs` and NEWLIST's `listn`, had clauses from the
start for that reason. When the fuzzer does find a hole, the fix is
usually a new operand kind with a `verify` clause, as in chapter 6,
which costs nothing in the op.

The fuzzer can't test what the verifier doesn't see:

- that the prototype's data (`nregs`, `nparams`) matches the code
- that a CALL's target was verified

The fuzzer keeps those valid by changing only code, because in Pip
they come from the compiler, never from bytecode. An interpreter that
loads bytecode files must check them when loading.

## Clean-room benchmarks

The step 8 benchmarks were written by the same author as the
interpreter. That is a problem: it is easy, without meaning to, to
write benchmarks that exercise exactly what you optimized.

Mini's and MiniJS's benchmarks were written **clean-room**. A separate
AI agent got only the language specification and the reference
interpreters, and was told not to look at the implementation (its
instructions are in
[`bench/mini/CLEANROOM_PROMPT.md`](../../bench/mini/CLEANROOM_PROMPT.md)).
It chose the programs (fib, binary-trees, n-body, spectral-norm,
fannkuch, string building, and its own picks), sized them, and recorded
the reference output. The same agent wrote the conformance tests, so
they test the specification, not the implementer's reading of it.

To do this for your language, you need:

- a specification good enough for someone else to write programs from
  (Mini's is [`examples/mini/SPEC.md`](../../examples/mini/SPEC.md))
- a reference implementation, for the expected output and for
  comparison

The comparison script (`bench/mini/compare.py`) then runs each
benchmark under every implementation and dispatch mode, taking turns,
and keeps the best of N runs.

Two tools from `std.prof` help with timing inside one program:

- `Bench` compares alternatives you can call as functions, such as two
  ways of running a loop, or an optimization switched on and off. It
  runs warmup rounds, then timed rounds that take turns, and reports
  the minimum and median of each.
- `prof.op_counts("Pip")` reads the `--vm-stats` op counts from inside
  the program, for example to `reset` them after a warmup, or to check
  that a super was used.

## Where to go from here

Pip is deliberately small. Natural next steps, each worth a chapter:

- `and`/`or`, compiled to jumps (see the table at the end of chapter 8);
- closures with upvalues, as in [`examples/mini`](../../examples/mini);
- tables or objects with hidden classes and inline caches, which
  `std.obj` provides (see [`examples/minijs`](../../examples/minijs));
- an exception table in `on raise`, to make errors catchable;
- string interning, so string equality is one compare.

For the details of everything used here, see the
[language reference](../language.md) and the
[standard library reference](../stdlib.md). The example interpreters in
[`examples/`](../../examples), such as Mini, MiniJS, Scheme and Lox,
show the same techniques at full size.
