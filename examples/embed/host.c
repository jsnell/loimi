/* A C host embedding the Mini interpreter (examples/mini/embed.lm).
 *
 *   loimi build examples/mini/embed.lm --lib -o libmini.a --header mini.h
 *   clang-19 -I. examples/embed/host.c libmini.a -lm -o host && ./host
 *
 * or, as a shared object:
 *
 *   loimi build examples/mini/embed.lm --shared -o libmini.so --header mini.h
 *   clang-19 -I. examples/embed/host.c -L. -lmini -Wl,-rpath,. -o host
 *
 * It runs two interpreters side by side, exchanges globals with them,
 * registers C functions as Mini natives, reports syntax and runtime
 * errors, and runs an allocation-heavy script with GC stress on.
 */
#include <stdio.h>
#include <string.h>

#include "mini.h"

static void run(MiniState *S, const char *name, const char *src) {
    fflush(stdout); /* Mini prints through the same stdio buffer, but be explicit */
    MiniStatus st = mini_dostring(S, (void *)src, strlen(src));
    if (st != MiniStatus_Ok) {
        loimi_str e = mini_error(S);
        printf("[%s] status %d: %.*s\n", name, st, (int)e.len, e.ptr);
    }
}

/* add(a, b, ...): the sum of its arguments. `ud` counts calls. */
static int32_t native_add(MiniState *S, void *ud) {
    double sum = 0;
    for (uint64_t i = 0; i < mini_argc(S); i++) {
        if (mini_arg_type(S, i) != MiniType_Number)
            return mini_raise(S, LOIMI_STR("add: numbers expected"));
        sum += mini_arg_number(S, i);
    }
    ++*(int *)ud;
    mini_return_number(S, sum);
    return 0;
}

/* greet(name): "hello, <name>" (a new string in the caller's heap). */
static int32_t native_greet(MiniState *S, void *ud) {
    (void)ud;
    loimi_str name = mini_arg_string(S, 0);
    char buf[256];
    int n = snprintf(buf, sizeof buf, "hello, %.*s", (int)name.len, name.ptr);
    mini_return_string(S, (loimi_str){ buf, (size_t)n });
    return 0;
}

/* reenter(): tries to run Mini code from inside a native. Refused. */
static int32_t native_reenter(MiniState *S, void *ud) {
    MiniState *other = ud;
    const char *src = "print('nested')";
    MiniStatus st = mini_dostring(other, (void *)src, strlen(src));
    loimi_str e = mini_error(other);
    printf("nested mini_dostring: status %d: %.*s\n", st, (int)e.len, e.ptr);
    /* The getters and setters do not collect, so they are allowed. */
    mini_set_integer(other, "touched", 1);
    mini_return_bool(S, st == MiniStatus_Reenter);
    return 0;
}

/* stash(s): copies a string into the other state's global `stashed`. */
static int32_t native_stash(MiniState *S, void *ud) {
    mini_set_string((MiniState *)ud, "stashed", mini_arg_string(S, 0));
    return 0;
}

int main(void) {
    MiniState *A = mini_new();
    MiniState *B = mini_new();
    int adds = 0;

    /* Globals in, globals out; the two states do not share them. */
    mini_set_number(A, "x", 1.5);
    mini_set_integer(B, "x", 40);
    mini_set_string(A, "who", LOIMI_STR("A"));
    mini_set_string(B, "who", LOIMI_STR("B"));
    run(A, "A", "y = x * 2  print(who, x, y)");
    run(B, "B", "y = x + 2  print(who, x, y)");
    double y;
    if (mini_get_number(A, "y", &y)) printf("A.y = %g\n", y);
    if (mini_get_number(B, "y", &y)) printf("B.y = %g\n", y);
    printf("types: %d %d %d %d\n", mini_get_type(A, "y"), mini_get_type(A, "who"),
           mini_get_type(A, "print"), mini_get_type(A, "nothing"));

    /* Natives. */
    mini_register(A, "add", native_add, &adds);
    mini_register(A, "greet", native_greet, NULL);
    mini_register(A, "reenter", native_reenter, A);
    mini_register(B, "stash", native_stash, A);
    run(A, "A", "print(add(1, 2, 3.5), greet('world'))  msg = greet('mini') .. '!'");
    loimi_str msg = mini_get_string(A, "msg");
    printf("A.msg = %.*s (%zu bytes)\n", (int)msg.len, msg.ptr, msg.len);
    run(A, "A", "print(reenter())");
    printf("A.touched = %d\n", mini_get_type(A, "touched") == MiniType_Number);
    run(B, "B", "stash('from B ' .. who)");
    msg = mini_get_string(A, "stashed");
    printf("A.stashed = %.*s\n", (int)msg.len, msg.ptr);

    /* Errors: syntax, runtime, from a native. The state stays usable. */
    run(A, "A", "print('never printed')\nlocal t = {\nprint(1 +)");
    run(A, "A", "local function f(t) return t.x.y end\nf({})");
    run(A, "A", "print(add(1, 'two'))");
    run(A, "A",
        "local function counter() local n = 0 return function() n = n + 1 return n end end\n"
        "c = counter() c() error('stop at ' .. c())");
    run(A, "A", "print('after errors:', c(), c(), add(10, 20))");
    printf("adds = %d\n", adds);

    /* GC stress: collect at every safepoint, both states alive, natives
     * allocating strings. Builds and checks a table of 2000 strings. */
    mini_set_gc_stress(A, true);
    uint64_t before = mini_gc_count(A);
    run(A, "A",
        "local t = {}\n"
        "for i = 1, 2000 do t[i] = greet('n' .. i) .. ':' .. add(i, i) end\n"
        "local ok = 0\n"
        "for i = 1, 2000 do if t[i] == 'hello, n' .. i .. ':' .. (2 * i) .. '.0' then ok = ok + 1 end end\n"
        "print('stress ok', ok, t[1], t[2000])\n"
        "result = #t");
    mini_set_gc_stress(A, false);
    printf("collections under stress: %s\n", mini_gc_count(A) - before > 2000 ? "many" : "few");
    if (mini_get_number(A, "result", &y)) printf("A.result = %g\n", y);
    run(B, "B", "local s = '' for i = 1, 300 do s = s .. who end print(#s)");

    mini_free(A);
    /* B outlives A. */
    run(B, "B", "print('B still here', x, y)");
    mini_free(B);
    return 0;
}
