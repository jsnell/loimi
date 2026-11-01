// The same VM as bench/fib.lm, written as an expert would in C:
// computed goto, NaN-boxing, overflow builtins, same bytecode.
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef uint64_t Value;
#define TAG_INT 0xFFF9ull
#define LIMIT (TAG_INT << 48)
#define IS_INT(v) (((v) >> 48) == TAG_INT)
#define BOTH_INT(a, b) (((((a) ^ LIMIT) | ((b) ^ LIMIT)) >> 48) == 0)
#define BOTH_NUM(a, b) (((a) > (b) ? (a) : (b)) < LIMIT)
#define MK_INT(i) (LIMIT | (uint32_t)(i))
#define AS_INT(v) ((int32_t)(uint32_t)(v))
static inline double as_num(Value v) { double d; memcpy(&d, &v, 8); return d; }
static inline Value mk_num(double d) { Value v; if (d != d) return 0x7FF8000000000000ull; memcpy(&v, &d, 8); return v; }
static double to_num(Value v) { return IS_INT(v) ? AS_INT(v) : v < LIMIT ? as_num(v) : 0.0; }
__attribute__((noinline, cold)) static Value arith_slow(int op, Value a, Value b) {
    double x = to_num(a), y = to_num(b); return mk_num(op == 0 ? x + y : x - y);
}

enum { LOADI, MOV, ADD, SUB, JLT, CALL, RET };
typedef struct { uint32_t *ret; Value *regs; } Frame;

#define OP(w) ((w) & 0xff)
#define A(w) (((w) >> 8) & 0xff)
#define B(w) (((w) >> 16) & 0xff)
#define C(w) ((w) >> 24)
#define SBX(w) ((int32_t)(w) >> 16)
#define SC(w) ((int32_t)(w) >> 24)

static Value run(Frame *frames, uint32_t *pc, Value *regs) {
    static void *labels[] = { &&L_LOADI, &&L_MOV, &&L_ADD, &&L_SUB, &&L_JLT, &&L_CALL, &&L_RET };
    long depth = 0;
    uint32_t w;
#define DISPATCH() do { w = *pc; goto *labels[OP(w)]; } while (0)
#define NEXT() do { pc++; DISPATCH(); } while (0)
    DISPATCH();
L_LOADI: regs[A(w)] = MK_INT(SBX(w)); NEXT();
L_MOV: regs[A(w)] = regs[B(w)]; NEXT();
L_ADD: {
    Value a = regs[B(w)], b = regs[C(w)];
    if (BOTH_INT(a, b)) { int32_t r; if (!__builtin_add_overflow(AS_INT(a), AS_INT(b), &r)) { regs[A(w)] = MK_INT(r); NEXT(); } }
    else if (BOTH_NUM(a, b)) { regs[A(w)] = mk_num(as_num(a) + as_num(b)); NEXT(); }
    regs[A(w)] = arith_slow(0, a, b); NEXT();
}
L_SUB: {
    Value a = regs[B(w)], b = regs[C(w)];
    if (BOTH_INT(a, b)) { int32_t r; if (!__builtin_sub_overflow(AS_INT(a), AS_INT(b), &r)) { regs[A(w)] = MK_INT(r); NEXT(); } }
    else if (BOTH_NUM(a, b)) { regs[A(w)] = mk_num(as_num(a) - as_num(b)); NEXT(); }
    regs[A(w)] = arith_slow(1, a, b); NEXT();
}
L_JLT: {
    Value a = regs[A(w)], b = regs[B(w)];
    if (__builtin_expect(BOTH_INT(a, b), 1)) { if (AS_INT(a) < AS_INT(b)) { pc += SC(w); DISPATCH(); } NEXT(); }
    if (to_num(a) < to_num(b)) { pc += SC(w); DISPATCH(); }
    NEXT();
}
L_CALL: {
    Frame *f = &frames[depth++];
    f->ret = pc + 1; f->regs = regs;
    regs += A(w); pc += SBX(w); DISPATCH();
}
L_RET: {
    Value v = regs[A(w)];
    if (depth == 0) return v;
    Frame *f = &frames[--depth];
    regs[0] = v; regs = f->regs; pc = f->ret; DISPATCH();
}
}

static uint32_t abc(int op, int a, int b, int c) { return op | a << 8 | b << 16 | (uint32_t)c << 24; }
static uint32_t asbx(int op, int a, int sbx) { return op | a << 8 | (uint32_t)(sbx & 0xffff) << 16; }

int main(void) {
    uint32_t code[] = {
        asbx(LOADI, 0, 32), asbx(CALL, 0, 2), abc(RET, 0, 0, 0),
        asbx(LOADI, 1, 2), abc(JLT, 0, 1, 9 & 0xff), asbx(LOADI, 1, 1), abc(SUB, 2, 0, 1),
        asbx(CALL, 2, -4), asbx(LOADI, 1, 2), abc(SUB, 3, 0, 1), asbx(CALL, 3, -7),
        abc(ADD, 0, 2, 3), abc(RET, 0, 0, 0), abc(RET, 0, 0, 0),
    };
    Frame *frames = malloc(1024 * sizeof(Frame));
    Value *regs = malloc(4096 * sizeof(Value));
    Value r = run(frames, code, regs);
    printf("%d\n", AS_INT(r));
    return 0;
}
