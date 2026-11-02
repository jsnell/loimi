# 4. Values, slow paths and errors

Pip is dynamically typed: a register can hold an integer, a float, a
boolean or `nil`, and `+` has to look at both operands to know what to
do. This chapter adds a value type, the fast and slow paths every
dynamic-language handler has, and run-time errors that report the line
they happened on.

Code: [`examples/tutorial/step4/`](../../examples/tutorial/step4).
The vm is now called `Pip`, and `value.lm` is new.

```text
print 7 / 2;           // 3
print 7.0 / 2;         // 3.5
print 2147483647 + 1;  // 2147483648.0
print 1 == 1.0;        // true
print 2 + 3 * true;    // error: line 1: arithmetic on a boolean value
```

## A NaN-boxed value

C interpreters represent values in one of a few ways: a tagged struct
(`TValue` in Lua: a tag word and a payload word), NaN-boxing (LuaJIT,
JavaScriptCore: doubles as themselves, everything else hidden in the
bit patterns of NaNs), or pointer tagging (OCaml, many Schemes: small
integers with a low bit set). Each comes with a page of macros that
build, test and unpack values, and each macro is a chance to get the
bit layout wrong. Changing representation later means rewriting all of
them.

In Loimi the representation is a declaration:

```loimi
pub value Value repr nanbox {
    Num(f64) unboxed,
    Int(i32),
    Nil,
    Bool(bool),
}
```

A `Value` is one 64-bit word. `Num` is `unboxed`: a double is stored as
itself. The other variants live in the NaN space, each with its own
tag. The compiler picks the tags; you get constructors (`Int(3)`,
`Value.Nil`), patterns, `==` on the bits, and a cast to `u64` if you
need the raw word.

Two properties come for free that hand-written NaN-boxing regularly
misses. Constructing a `Num` canonicalizes NaNs, so a guest computation
can never produce a bit pattern that reads as a tagged value (a
pointer, from chapter 7 on). And the compiler skips the
canonicalization where it can prove it unnecessary: the sum of two
unboxed doubles is already safe, as the assembly below shows.

`repr lowtag` (pointer tagging, 61-bit integers) and `repr boxed` (two
words) are the alternatives; switching is a one-word change, so you can
measure rather than guess.

Why `Int(i32)` and not 48 bits? Mini, the Lua-subset example, measured
both: overflow-checked arithmetic on a 48-bit payload costs two extra
shifts per operation, and that cost 3-9% on its integer benchmarks
([`bench/mini/RESULTS.md`](../../bench/mini/RESULTS.md), "Int(i48):
measured, not adopted"). LuaJIT's dual-number mode chose 32 bits for the
same reason.

## Handlers with a fast path

`ADD` must handle two integers, two floats, mixed operands, overflow,
and type errors. Only the first two matter for speed:

```loimi
        op #name(dst: reg, a: reg, b: reg) : ABC {
            match (a, b) {
                (Int(x), Int(y)) => if let Some(r) = x.#checked(y) { dst = Int(r); next },
                (Num(x), Num(y)) => { dst = Num(x sym y); next }
                _ => {}
            }
            cold { dst = arith(kind, a, b)?; next }
        }
```

(The `#name` and `sym` come from an `inline for`, below.)

`match (a, b)` over a tuple of values is compiled knowing the bit
layout. With NaN-boxing, "both are `Int`" is a single test: xor each
word with the `Int` tag, or the results together, and check that the
high 16 bits are zero. "Both are `Num`" is `umax(a, b) < first tag`.
Here is the release build of `ADD`:

```text
<vm.Pip.op.ADD>:
  mov    %edi,%eax                  ; edi: the instruction word
  movabs $0xfff9000000000000,%rdx   ; the Int tag
  movzbl %ah,%ecx
  mov    %edi,%esi
  shr    $0xd,%esi
  and    $0x7f8,%esi
  mov    (%r14,%rsi,1),%rdi         ; a (r14: regs)
  mov    %eax,%esi
  shr    $0x18,%esi
  mov    (%r14,%rsi,8),%rsi         ; b
  mov    %rdi,%r8
  xor    %rdx,%r8
  mov    %rsi,%r9
  xor    %rdx,%r9
  or     %r8,%r9
  shr    $0x30,%r9                  ; (Int, Int)?
  jne    float_path
  add    %edi,%esi                  ; 32-bit add
  jo     slow                       ; overflow
  mov    %esi,%eax
  or     %rdx,%rax                  ; re-tag
  mov    %rax,(%r14,%rcx,8)
  mov    0x4(%r13),%edi             ; next
  add    $0x4,%r13
  movzbl %dil,%eax
  jmp    *0x40c4d0(,%rax,8)
float_path:
  cmp    %rsi,%rdi
  mov    %rsi,%r8
  cmova  %rdi,%r8
  cmp    %rdx,%r8                   ; (Num, Num)?
  jae    slow
  movq   %rdi,%xmm0
  movq   %rsi,%xmm1
  addsd  %xmm0,%xmm1
  movsd  %xmm1,(%r14,%rcx,8)        ; no NaN check needed
  ...                               ; next
slow:
  mov    %eax,%edi
  jmp    vm.Pip.op.ADD.cold0
```

## `cold` blocks

The interesting part is what is *not* there. The slow path calls
`arith`, a real function that may build an error. In C, a call anywhere
in a handler makes the compiler treat every caller-saved register as
clobbered, so it spills the interpreter's state on entry and reloads
it at every exit, on the fast path too. C programmers fight this with
`UNLIKELY()`, manual outlining and `noinline`, and check the assembly
after every change.

A `cold { }` block says "this is the slow path". In tail-call mode,
when the block ends in a control word, the compiler moves it to a
function of its own with the handler's signature, and the handler
reaches it with a tail call (`jmp vm.Pip.op.ADD.cold0` above). The fast
path contains no call, needs no stack frame and spills nothing; the
cold function decodes the operands it needs again from the instruction
word, which travels in its register. In goto and switch mode, cold
blocks stay inline and are marked unlikely.

The slow path itself is ordinary code:

```loimi
/// Everything the handlers' fast paths leave out: integer overflow,
/// mixed integer and float operands, division, and type errors.
fn arith(op: Arith, a: Value, b: Value) -> Result<Value, str> {
    if let (Int(x), Int(y)) = (a, b) {
        let (x, y) = (x as i64, y as i64);
        let r = match op {
            Add => x + y,
            Sub => x - y,
            Mul => x * y,
            Div => { if y == 0 { return Err("division by zero"); } x / y }
            Mod => { if y == 0 { return Err("division by zero"); } x % y }
        };
        // A result that does not fit an Int becomes a float.
        if r >= -2147483648 and r <= 2147483647 { return Ok(Value.Int(r as i32)); }
        return Ok(Value.Num(r as f64));
    }
```

Note where things go. Mini's log has an instructive failure: adding
mixed int/float cases to the *hot* part of its arithmetic handlers made
one benchmark 18% slower, and moving them into the cold block fixed it.
The hot path should test for the one or two cases that matter and
nothing else.

## Generating ops with `inline for`

ADD, SUB and MUL differ in three tokens. `inline for` copies a template
once per element, at compile time, substituting tokens:

```loimi
    // Three ops from one template: `#name` makes an identifier, and `sym`
    // is pasted as a token.
    inline for (name, checked, sym, kind) in [
        ("ADD", "add_checked", +, Arith.Add),
        ("SUB", "sub_checked", -, Arith.Sub),
        ("MUL", "mul_checked", *, Arith.Mul),
    ] {
```

`#name` turns the string `"ADD"` into the identifier `ADD`, so the
template declares `op ADD`, `op SUB` and `op MUL`; `x.#checked(y)`
becomes `x.add_checked(y)`. An element can be any token sequence, an
operator or a whole expression:

```loimi
    inline for (name, test, or_equal) in [("LT", x < y, false), ("LE", x <= y, true)] {
        op #name(dst: reg, a: reg, b: reg) : ABC {
            match (a, b) {
                (Int(x), Int(y)) => { dst = Bool(test); next }
                (Num(x), Num(y)) => { dst = Bool(test); next }
                _ => {}
            }
            cold { dst = Bool(less(a, b, or_equal)?); next }
        }
    }
```

This is what X-macros do in C, with names that resolve normally and
errors that point into the template.

## Guards and falling out of a `match`

DIV can only take the integer fast path for a positive divisor (zero is
an error and `MIN / -1` overflows). Two ways to say that look alike:

```loimi
    op DIV(dst: reg, a: reg, b: reg) : ABC {
        match (a, b) {
            (Int(x), Int(y)) if y > 0 => { dst = Int(x / y); next }
            (Num(x), Num(y)) => { dst = Num(x / y); next }
            _ => {}
        }
        cold { dst = arith(Arith.Div, a, b)?; next }
    }
```

A guard (`pat if cond =>`) that fails moves on to the next arm. An arm
whose *body* is `if cond { ...; next }` would instead fall out of the
whole `match` when `cond` is false, skipping later arms; in a handler
that usually means skipping a fast path by accident. The compiler warns
(`match_fallout`) when an arm's body is an `if` without `else` that ends
in a control word and a later arm could match the same values. ADD's
`(Int, Int)` arm above is fine: a later arm cannot match two integers.

## Errors: `raise` and `on raise`

A run-time error must say where it happened, which means knowing `pc`.
In a C interpreter, `pc` lives in a local (a register) for speed, so
every place that might raise an error has to store it to memory first;
Lua's `savepc` and `Protect()` macros exist for this, and forgetting one
gives you errors reported at the wrong line.

In a Loimi handler, `raise e` (or `?` on an `Err` of the vm's error
type) transfers to the vm's `on raise` handler, *with the live state*.
`on raise` is itself a handler: it sees `pc` as it was in the op that
failed, without any syncing:

```loimi
    // `raise msg` (and `?` on an Err) in any handler lands here, with the
    // state as it was in that handler: `pc` still points at the
    // instruction that failed.
    on raise(msg: str) {
        let i = ((pc as u64) - (ctx.code as u64)) / 4;
        ctx.error_line = ctx.lines[i];
        fail msg
    }
```

The vm's raise type comes from `on raise`'s parameter, here a `str`.
`cold { dst = arith(kind, a, b)?; next }` raises whatever `arith`
returned in its `Err`. A real interpreter would search an exception
table here and `jump` to a handler; Pip just stops.

The line comes from a table: `bc.Code` tracks a source position per
instruction. The compiler calls `c.code.set_loc(line)` as it goes, and
`locs_into` expands the positions to one per word when the chunk is
finished. (`encode_locs` makes a compact delta-encoded table instead,
about two bytes per line change, for interpreters that keep many
functions around.)

## The context

`ctx.code`, `ctx.lines` and `ctx.error_line` live in the vm's
**context**, a pointer to everything handlers need that is not hot
enough to deserve a register of its own:

```loimi
pub vm Pip {
    context ctx: *Machine;
    state { pc: *u32, regs: *Value, k: *Value }
    verify_context lim: *Limits;
```

The context is one more register; its fields cost a load. The entry
sets it like the state:

```loimi
    entry run(m: *Machine, r: *Value, consts: *Value) -> Result<(), str> {
        ctx = m;
        pc = m.code;
        regs = r;
        k = consts;
        jump pc
    }
```

The error line is returned through `ctx` rather than in the `Err`,
which keeps `str` as the error type everywhere. The entry may return
any type, a `Result` of a struct with a message and a line included:
in tail-call mode, a result that is not a single scalar travels from
`exit` to the entry through a thread-local slot.

## Printing values

`value.write` formats a value into a `StrBuf`; `std.num` gives floats
their shortest round-trip form, and `mark_float` adds `.0` to floats
that look like integers, as Lua and Python do:

```loimi
pub fn write(b: *text.StrBuf, v: Value) {
    match v {
        Int(i) => b.push_int(i as i64),
        Num(x) => {
            let start = b.len();
            num.write_shortest(b, x);
            num.mark_float(b, start);
        }
        Bool(t) => b.push_str(if t { "true" } else { "false" }),
        Nil => b.push_str("nil"),
    }
}
```

## Constants

Float constants and large integers go through the constant pool, now a
`bc.Pool<Value>`. The pool deduplicates through a hash map, so `Value`
needs the `hash.Hash` trait. A trait impl in Loimi contains no code: it
asserts that the methods exist.

```loimi
pub fn Value.hash(v: Value) -> u64 { hash.mix(v as u64) }
pub fn Value.eq(a: Value, b: Value) -> bool { a == b }
impl hash.Hash for Value;
```

Hashing and comparing the bits keeps `0`, `0.0` and `-0.0` apart, which
a pool that compared numerically would merge.

Next: [variables and control flow](05-variables.md).
