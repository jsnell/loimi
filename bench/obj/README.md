# std.obj inline-cache microbenchmark

How much a property read through `std.obj`'s inline cache costs,
compared with a plain register move and with the slower lookups.

`ic.lm` runs a vm loop of 8 property reads per iteration, 10 million
iterations, on an object with four inline properties. It does the read
three ways, plus a loop of `MOVE`s in the same places as the baseline:

```sh
LOIMI_STD=. target/release/loimi run bench/obj/ic.lm --release
```

Results on an Intel i7-6700 with clang 19, tail-call dispatch,
2026-11-02, with other jobs running on the machine:

| Read | Loop time | Cost per read over `MOVE` |
|---|---|---|
| `MOVE` (baseline) | 91 ms | |
| monomorphic inline cache hit (`GETPROP_IN`) | 156 ms | 0.8 ns |
| global megamorphic cache (`get_mega`) | 480 ms | 4.9 ns |
| uncached lookup (`get`: walks the transition chain) | 620 ms | 6.6 ns |

A cache hit costs about three cycles more than a register move. This is
the hit path of `GETPROP_IN`, from `objdump -d` of a `--release` build of
`tests/run/std_obj_ic.lm`:

```
mov    %r15d,%ecx                 ; decode the `o` register operand
shr    $0xd,%ecx
and    $0x7f8,%ecx
mov    (%r14,%rcx,1),%rcx         ; o = regs[b]
mov    %rcx,%rdx
shr    $0x30,%rdx
cmp    $0xfffb,%edx               ; is it an Obj?
jne    cold
movabs $0xffffffffffff,%rdx
and    %rdx,%rcx                  ; unbox the pointer
mov    (%rcx),%rdx                ; o.shape
mov    (%rdx),%edx                ; o.shape.id
cmp    0x4(%r13),%edx             ; == cache.shape ?
jne    cold
mov    %r15d,%eax
mov    0x8(%r13),%edx             ; cache.slot
mov    0x18(%rcx,%rdx,8),%rcx     ; o.inline[slot]
movzbl %ah,%eax
mov    %rcx,(%r14,%rax,8)         ; regs[a] = value
mov    0xc(%r13),%r15d            ; next: load the next word and dispatch
add    $0xc,%r13
movzbl %r15b,%eax
jmp    *0x405d00(,%rax,8)
```

- The cache itself is the two loads and the compare marked `o.shape`,
  `o.shape.id` and `== cache.shape`, plus the slot load.
- The rest is operand decoding, the type test that any property read
  needs, and dispatch.
- The out-of-line variant (`GETPROP_OUT`) adds one load (`o.overflow`)
  and masks off the SlotRef bit.
- Keeping the shape id in the object, next to the shape pointer, would
  save one dependent load per hit. `std.obj` reads it through the shape
  so that objects stay one word smaller.
