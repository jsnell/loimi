# 6. Functions and calls

```text
fn fib(n) {
    if n < 2 { return n; }
    return fib(n - 1) + fib(n - 2);
}
print fib(20);
```

Functions are values: `fn fib` compiles to a **prototype**, which is
stored in the global `fib`.

Calls stay inside the interpreter. A call pushes a frame and jumps to
the callee's first instruction, and a return pops the frame and jumps
back. Nothing recurses on the C stack. So deep recursion in a Pip
program is limited by the vm's own stacks, which report overflow as an
ordinary error.

The code is in [`examples/tutorial/step6/`](../../examples/tutorial/step6).

## Prototypes and frames

Each function compiles to its own code, constants and register count:

```loimi
/// A compiled function. Prototypes live in ordinary memory and are never
/// freed: a program's functions live as long as the program.
pub struct Proto {
    name: str,
    nparams: u32,
    /// The size of the function's register window.
    nregs: u32,
    /// The bytecode, as the compiler wrote it.
    words: []u32,
    /// The proof that `words` passed `Pip.verify`. The compiler sets a
    /// placeholder; main stores the proof once every function is
    /// compiled (the verifier needs the final number of globals).
    code: Verified<Pip>,
    /// The source line of each code word.
    lines: []u32,
    consts: []Value,
}
```

`code` is the `Verified<Pip>` from chapter 3. It can't be made at the
same time as the prototype. The verifier checks global numbers against
the number of names in the whole program, which is known only once
everything is compiled.

So the compiler fills `code` with `Pip.placeholder()`, and `main`
replaces it with the real proof later. The placeholder wraps a single
word whose opcode belongs to no op, so it traps if it ever runs.

A function value is a pointer to its prototype. A nanbox variant can
hold any pointer:

```loimi
pub value Value repr nanbox {
    Num(f64) unboxed,
    Int(i32),
    Nil,
    Bool(bool),
    Fn(*Proto),
}
```

The machine gets two stacks: a value stack, which holds the registers
of every active call, and a frame stack:

```loimi
/// One activation record. `ret` is where the caller continues when this
/// frame's callee returns.
pub struct Frame { proto: *Proto, base: *Value, ret: *u32 }
```

The current frame becomes part of the vm state, next to `pc`, `regs`
and `k`:

```loimi
    state { pc: *u32, regs: *Value, k: *Value, fp: *Frame }
```

## Register windows

Like Lua, Pip passes arguments without copying them. The caller puts
them in consecutive registers, and those become the callee's first
registers. To call `f(x, y)`, the compiler:

1. puts `f` in some register `base`, `x` in `base+1` and `y` in
   `base+2`
2. emits `CALL base, 2`

The callee's registers start at `base+1`, so its parameters are already
in its `r0` and `r1`. Its result goes to the register just below its
own, `regs[-1]`, which is the caller's `base`.

```loimi
    /// R[base] = R[base](R[base+1], ..., R[base+n]). The callee's
    /// registers start at R[base+1], so the arguments are already its
    /// first registers, and the result goes just below its window.
    op CALL(base: reg, n: nargs) : ABC {
        let f = base;
        if let Fn(p) = f {
            if n as u32 != p.nparams { cold { raise "wrong number of arguments" } }
            let callee = regs + base.raw + 1;
            if callee + p.nregs as u64 > ctx.stack_end or fp + 1 == ctx.frames_end {
                cold { raise "stack overflow" }
            }
            fp.ret = pc + 1;
            fp = fp + 1;
            fp.proto = p;
            fp.base = callee;
            regs = callee;
            k = p.consts.ptr;
            jump p.code
        }
        cold { raise "call of a non-function" }
    }
```

Some details:

- `base` is a `reg` operand, so reading it gives the register's value.
  `base.raw` is the field itself: the register number.
- The op sets all four state variables and jumps. The callee runs with
  its own `regs` and `k`, in the same machine registers.
- `jump p.code` jumps to the start of the callee's verified code. The
  vm is `#[require_verified]`, and its entry also jumps to verified
  code (`main.code`), so every function that runs has been verified.
- `fp.ret = pc + 1` is the return address, because CALL is one word.
  For an op with cache words, write `fp.ret = next_pc` instead: the
  address where `next` would continue.

```loimi
    /// Return R[src] to the caller.
    op RET(src: reg) : ABC {
        let v = src;
        if fp == ctx.frames { exit Ok(()) }
        *(regs - 1) = v;
        fp = fp - 1;
        regs = fp.base;
        k = fp.proto.consts.ptr;
        jump fp.ret
    }
```

The `fp == ctx.frames` test is never true for code from Pip's compiler,
which does not allow `return` at the top level. It is there because the
verifier can't know that, and chapter 9's fuzzer will try it.

`on raise` now finds the line through the current frame's prototype. So
an error inside a function reports the line inside that function:

```loimi
    on raise(msg: str) {
        let p = fp.proto;
        let i = p.code.index_of(pc);
        ctx.error_line = p.lines[i];
        fail msg
    }
```

`p.code.index_of(pc)` is the index of the instruction `pc` points at,
in words of the vm's encoding (`(pc - start) / 4` here).

## Verifying a function

The verifier now checks one prototype at a time, against that
prototype. A vm can have several verify contexts, which become
parameters of `Pip.verify` in order:

```loimi
    // The verifier checks one function at a time, against its prototype.
    verify_context proto: *Proto, nnames: u32;

    operand reg = regs[it] verify (it as u32) < proto.nregs display "r{}";
    operand kst = k[it] verify it < proto.consts.len display "k{}";
```

Checking CALL's argument count needs more than a range: `base + n` must
also be inside the frame. A verify clause can read the instruction word
through `pc`, and decode other fields with the generated field readers:

```loimi
    /// CALL's argument count: R[base+1 .. base+n] must be in the frame.
    operand nargs = it verify Pip.ABC_a(*pc) + it < proto.nregs as u64;
```

In the op, `nargs` is just the field's value, so the check costs
nothing at run time. Counts like this are easy to forget to check: in
Mini, the fuzzer found nine of them, and each was fixed with a clause
like this one.

`main` verifies every prototype before running any of them, and stores
each proof in its prototype:

```loimi
    // Verify every function before running anything.
    for i in 0..prog.protos.len {
        let p = prog.protos.at(i);
        match Pip.verify(p.words, p, names.names.len as u32) {
            Ok(v) => p.code = v,
            Err(e) => {
                var b = text.StrBuf.new();
                b.push_str("bytecode failed verification: ");
                e.write(&b);
                error(b.as_str(), p.lines[e.at]);
                return 1;
            }
        }
    }
```

Some things the verifier can't check, so they are up to you:

- that `nparams <= nregs`
- that the stack has room for the main function's frame
- that a prototype's `nregs` and constants don't change after it was
  verified

## Compiling calls

```loimi
        Call(f, args) => {
            // The function and its arguments go to consecutive registers
            // at the top of the frame; the result comes back in the first.
            let mark = c.regs.mark();
            let base = c.regs.alloc(1 + args.len as u32);
            c.expr_to(f, base);
            for i in 0..args.len { c.expr_to(args.at(i), base + 1 + i as u32); }
            c.code.set_loc(n.line);
            c.code.op(Pip.CALL(base as u8, args.len as u8));
            if base != dst { c.code.op(Pip.MOVE(d, base as u8)); }
            c.regs.reset(mark);
        }
```

The base is allocated at the top of the caller's registers. So every
register above it is an unused temporary, which the callee may
overwrite.

A function body compiles with its own `bc.Code`, constant pool and
registers. Parameters are the first locals, and falling off the end
returns `nil`:

```loimi
fn function(unit: *Unit, f: *FnDecl) -> *Proto {
    var c = new_c(unit, true);
    c.code.set_loc(f.line);
    for i in 0..f.params.len { c.add_local(f.params.at(i)); }
    for i in 0..f.body.len { c.stmt(f.body.at(i)); }
    // Falling off the end returns nil.
    let r = c.regs.alloc(1);
    c.code.op(Pip.LOADNIL(r as u8));
    c.code.op(Pip.RET(r as u8));
    c.finish(f.name, f.params.len as u32, f.line)
}
```

Here is `fn add(x, y) { return x + y; } print add(1, 2);`:

```text
fn add:
    0  ADD dst=r2 a=r0 b=r1
    1  RET src=r2
    2  LOADNIL dst=r2
    3  RET src=r2
fn main:
    0  LOADK dst=r0 c=k0
    1  DEFGLOBAL src=r0 n=g0
    2  GETGLOBAL dst=r1 n=g0 [cache slot=0]
    4  LOADI dst=r2 v=1
    5  LOADI dst=r3 v=2
    6  CALL base=r1 n=2
    7  MOVE dst=r0 src=r1
    8  PRINT a=r0
    9  HALT
```

The `MOVE` after the call can be avoided; chapter 8 removes it.

The inline caches from chapter 5 now pay off, because `fib` reads the
global `fib` twice per call. The test `tests/recache.pip` checks the
case where the cache goes stale. It defines globals after a function
has cached a slot, so the table grows and the slot may no longer hold
the name. GETGLOBAL_C then falls back to GETGLOBAL, which fills the
cache again.

Next: [heap objects and the GC](07-gc.md).
