# 9. Testing an interpreter

An interpreter has more ways to be wrong than most programs: the
compiler can emit wrong code, a handler can mishandle a rare type
combination, the GC can miss a root that matters once in a million
allocations, and a verifier can accept code that walks off the end of
the register file. This chapter collects the testing this tutorial has
used, and adds the piece that finds memory-safety bugs: a bytecode
fuzzer.

Code: [`examples/tutorial/step9/`](../../examples/tutorial/step9): the
interpreter from step 8 with a step-budget variant, and `fuzz.lm`.

## Conformance tests and the test matrix

Each step has a `tests/` directory: `NAME.pip` is a program and
`NAME.expected` its output (stdout and stderr together). Tests named
`err_*` must also exit with an error. `examples/tutorial/check.sh`
builds each step and runs every test

- in all three dispatch modes (`tailcall`, `goto`, `switch`), since
  they are separate code generators and any one of them can be wrong in
  its own way; and
- from step 7 on, a second time with `--gc-stress`, which collects at
  every safepoint and turns a missing root into a wrong answer or a
  crash in the first run that depends on it (chapter 7).

```sh
examples/tutorial/check.sh            # every step
examples/tutorial/check.sh step7      # one step
examples/tutorial/check.sh -u step7   # rewrite the .expected files
```

`-u` is for when output changes on purpose; read the diff before
committing it. Write tests for the cases the fast paths skip:
`2147483647 + 1` (overflow into a float), `-7 % 3`, `1 == 1.0`, `nil ==
false`, arithmetic on every wrong type, a global read after the globals
table has grown (`recache.pip`), and an `err_` test for every error
message.

For a language with an existing implementation, the expected output
should come from that implementation, not from yours. Mini's
conformance tests are Lua programs whose `.expected` files are PUC-Lua's
output; MiniJS's are node's.

## A step budget, as a `variant`

The fuzzer runs random programs, and random programs loop forever. The
usual fix is a counter checked in the backward branches of the
interpreter, which every normal run then pays for. Loimi's
instrumentation **variants** avoid that: a variant is a second dispatch
table whose handlers run some code before the real handler.

```loimi
    variant budget wraps [JMP, JF, JNLT, JNLE, JNLTK, JNLEK, JNGTK, JNGEK, JNEQ, JEQ, JNEQK, JEQK,
                          CALL, ADDK_JMP, ADD_JMP, SUBK_CALL] {
        before {
            ctx.steps -= 1;
            if ctx.steps < 0 { cold { raise "step budget exhausted" } }
        }
    }
```

Only ops that can jump are wrapped: every loop goes through one of
them, and the others keep their plain handlers. (Superinstructions are
ops too; the ones whose parts jump are listed.) A second entry switches
tables before dispatching:

```loimi
    entry run_budget(m: *Machine, main: *Proto) -> Result<(), str> {
        use_variant budget;
```

`run` dispatches through the normal table, and its handlers do not
know the variant exists. On the benchmarks, step 9 runs as fast as
step 8. The same mechanism gives you a tracer, a debugger's breakpoint
check or a coverage counter without a second copy of the interpreter.

`wraps all` would work too, and builds as fast in every dispatch mode
(the compiler adds wrapper blocks only to the entries that switch to the
variant, here `run_budget`), but wrapping only the 16 ops that can jump
keeps the step-counting overhead off straight-line code.

## The fuzzer

The verifier is what makes it safe for handlers to skip bounds checks
(chapter 3). If a `verify` clause is missing or wrong, the vm reads or
writes outside its arrays. Nothing in the normal tests exercises this,
because the compiler only emits valid code. The fuzzer feeds the
verifier *invalid* code and runs whatever it accepts.

Completely random words are almost all rejected at the first word, which
tests little. Code close to valid is where holes hide, so `fuzz.lm`
starts from real programs: it compiles a corpus file, changes one to
three words, and verifies the result:

```loimi
/// Change one word of `p`'s code.
fn mutate(r: *Rng, p: *Proto) {
    let i = r.below(p.code.len);
    var w = p.code[i];
    let k = r.below(100);
    if k < 40 {
        // One operand byte: usually a small number, sometimes anything.
        let shift = 8 * (1 + r.below(3));
        let v = if r.chance(80) { r.below(p.nregs as u64 + 3) } else { r.below(256) };
        w = (w & (((255 as u64) << shift) as u32 ^ 0xFFFFFFFF)) | ((v << shift) as u32);
    } else if k < 65 {
        // Another opcode, same operands.
        w = (w & 0xFFFFFF00) | (r.below(Pip.NUM_OPS) as u32);
    } else if k < 85 {
        w = w ^ (((1 as u64) << r.below(32)) as u32);
    } else if k < 95 {
        // Another target, if the word is a jump.
        let off = r.below(p.code.len + 4) as i64 - i as i64 - 2;
        if let Some(j) = Pip.set_label(w, off) { w = j; }
    } else {
        w = r.next() as u32;
    }
    p.code[i] = w;
}
```

The generated tools make this short: `Pip.NUM_OPS` and `Pip.set_label`
come from the vm, and with `-v` the fuzzer prints each rejected word
with the generated `Pip.disasm_insn`:

```text
rejected:    6  GETGLOBAL dst=r0 n=g6 [cache slot=0]
rejected:    2  <invalid 0xa9d828b4>
rejected:    3  ADD_F dst=r0 a=r4 b=r0
rejected:   64  JMP target -> 324
```

Programs that pass go through the generated peephole pass and run under
the step budget. Besides crashes, the fuzzer checks two properties the
vm must keep: code still verifies after `Pip.optimize` has fused supers,
and after running, when sites have quickened and caches hold whatever
the program left in them:

```loimi
    if !verify_all(&prog.protos, nnames, true) {
        st.rejected += 1;
    } else {
        for i in 0..prog.protos.len { Pip.optimize(prog.protos.at(i).code); }
        if !verify_all(&prog.protos, nnames, false) { bug(seed, "code does not verify after Pip.optimize"); }
        match Pip.run_budget(&m, prog.main) {
            Ok(_) => st.finished += 1,
            Err(msg) => if msg.eq("step budget exhausted") { st.budget += 1; } else { st.raised += 1; },
        }
        if !verify_all(&prog.protos, nnames, false) { bug(seed, "code does not verify after running"); }
    }
```

Build it with AddressSanitizer, which works in all three dispatch
modes, and run it in batches (each run's memory is freed, but the
lexer's tables are not, so a long batch grows):

```sh
loimi build examples/tutorial/step9/fuzz.lm --sanitize=address -o fuzz
cd examples/tutorial/step9
./fuzz 1 2000 tests/*.pip
./fuzz 1 2000 tests/*.pip --gc-stress
```

```text
3000 programs: 1887 rejected by the verifier, 572 finished, 268 raised an error, 273 ran out of steps
```

About a third of the mutants pass the verifier, which is the useful
range: too few and the handlers are never reached, too many and the
mutations are not testing the verifier.

`cargo test` runs 3000 programs (without ASan). While writing this
chapter, 200,000 programs under ASan, and 100,000 more with GC stress,
found one real problem: `array(2147483647)` asked the allocator for 16
GiB, which the step budget cannot prevent. Step 9 caps arrays and
strings at 2^24 elements:

```loimi
        // The limit was found by the fuzzer: array(2147483647) asked the
        // allocator for 16 GiB.
        if let Int(len) = n and len >= 0 and len <= MAX_LEN {
```

Mini's fuzzer found nine holes in its `verify` clauses, all counts whose
covered range was never checked (CALL's argument count, SETLIST's
length, CONCAT with fewer than two operands). Pip's equivalents, CALL's
`nargs` and NEWLIST's `listn`, had clauses from the start because of
that experience. When the fuzzer does find one, the fix is usually a
new operand kind with a `verify` clause, as in chapter 6, which costs
nothing in the handler.

What the fuzzer cannot test is what the verifier does not see: that the
prototype's metadata matches the code (`nregs`, `nparams`), or that a
CALL's target was verified. The fuzzer keeps those valid by mutating
only code, because in Pip they come from the compiler, never from
bytecode. An interpreter that loads bytecode files must check them when
loading.

## Clean-room benchmarks

The step 8 benchmarks were written by the same person as the
interpreter, which is a problem: it is easy, without meaning to, to
write benchmarks that exercise exactly what you optimized. Mini's and
MiniJS's benchmarks were written **clean-room**: a separate agent got
only the language specification and the reference interpreters, with
instructions not to look at the implementation
([`bench/mini/CLEANROOM_PROMPT.md`](../../bench/mini/CLEANROOM_PROMPT.md)).
It chose the programs (fib, binary-trees, n-body, spectral-norm,
fannkuch, string building, and its own picks), sized them, and recorded
the reference output. The same agent wrote the conformance tests, so
they test the specification rather than the implementation's reading of
it.

To do this for your language you need a specification good enough for
someone else to write programs from (Mini's is
[`examples/mini/SPEC.md`](../../examples/mini/SPEC.md)), and a reference
implementation for expected outputs and for comparison. The comparison
scripts (`bench/mini/compare.py`) then run each benchmark under every
implementation and dispatch mode, interleaved, best of N.

## Where to go from here

Pip is deliberately small. Natural next steps, each a chapter's worth:

- `and`/`or` as jump chains (checklist item 5 in chapter 8);
- closures with upvalues, as in [`examples/mini`](../../examples/mini);
- tables or objects with hidden classes and inline caches, which
  `std.obj` provides (see [`examples/minijs`](../../examples/minijs));
- an exception table in `on raise`, to make errors catchable;
- string interning, so string equality is one compare.

[`docs/language.md`](../language.md) and [`docs/stdlib.md`](../stdlib.md)
are the references for everything used here; the three example
interpreters show the same techniques at full size.
