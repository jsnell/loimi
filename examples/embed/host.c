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
 * errors, and runs an allocation-heavy script with GC stress on. Natives
 * call back into Mini (mini_call, mini_dostring), nested several levels
 * deep, with errors passing through them, also under GC stress; and the
 * host calls Mini functions directly, and a native of one state runs the
 * other while both are under GC stress. Last, an internal error in a nested
 * run comes back as MiniStatus_Internal instead of aborting the host.
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

/* reenter(): runs Mini code from inside a native, in the native's own
 * state (nested above the caller), then sets a global of it. */
static int32_t native_reenter(MiniState *S, void *ud) {
    MiniState *other = ud;
    const char *src = "print('nested', who)  nested_runs = (nested_runs or 0) + 1";
    MiniStatus st = mini_dostring(other, (void *)src, strlen(src));
    printf("nested mini_dostring: status %d\n", st);
    mini_set_integer(other, "touched", 1);
    mini_return_bool(S, st == MiniStatus_Ok);
    return 0;
}

/* apply(f, ...): f(...), called back from C. An error in f passes through
 * unchanged (mini_raise_top re-raises the error value mini_call pushed). */
static int32_t native_apply(MiniState *S, void *ud) {
    (void)ud;
    uint64_t n = mini_argc(S);
    for (uint64_t i = 0; i < n; i++) mini_push_arg(S, i);
    if (mini_call(S, n == 0 ? 0 : n - 1) != MiniStatus_Ok) return mini_raise_top(S);
    mini_return_top(S);
    return 0;
}

/* ask_a(x): calls A's global `square` from a native of B: a run of A of
 * its own, while B is running. */
static int32_t native_ask_a(MiniState *S, void *ud) {
    MiniState *a = ud;
    mini_push_global(a, "square");
    mini_push_number(a, mini_arg_number(S, 0));
    MiniStatus st = mini_call(a, 1);
    double r = 0;
    if (st != MiniStatus_Ok || !mini_to_number(a, -1, &r)) {
        loimi_str e = mini_error(a);
        mini_pop(a, 1);
        return mini_raise(S, e);
    }
    mini_pop(a, 1);
    mini_return_number(S, r);
    return 0;
}

/* boom(s): an internal error ("boom: <s>"), as if the interpreter had a bug. */
static int32_t native_boom(MiniState *S, void *ud) {
    (void)ud;
    loimi_str s = mini_arg_string(S, 0);
    char buf[64];
    int n = snprintf(buf, sizeof buf, "boom: %.*s", (int)s.len, s.ptr);
    mini_debug_panic(S, (loimi_str){ buf, (size_t)n });
    return 0;
}

/* The deepest down(n) that fits under the nesting limit. */
#define DEEPEST "199"

/* Call global `f`(x) from the host, between runs, and print the result. */
static void call_global(MiniState *S, const char *f, double x) {
    mini_push_global(S, (void *)f);
    mini_push_number(S, x);
    MiniStatus st = mini_call(S, 1);
    double r;
    if (st == MiniStatus_Ok && mini_to_number(S, -1, &r)) {
        printf("%s(%g) = %g\n", f, x, r);
    } else if (st == MiniStatus_Ok) {
        loimi_str v = mini_to_string(S, -1);
        printf("%s(%g) = '%.*s'\n", f, x, (int)v.len, v.ptr);
    } else {
        loimi_str e = mini_error(S);
        printf("%s(%g) failed: %.*s\n", f, x, (int)e.len, e.ptr);
    }
    mini_pop(S, 1);
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
    mini_register(A, "apply", native_apply, NULL);
    mini_register(B, "stash", native_stash, A);
    mini_register(B, "ask_a", native_ask_a, A);
    run(A, "A", "print(add(1, 2, 3.5), greet('world'))  msg = greet('mini') .. '!'");
    loimi_str msg = mini_get_string(A, "msg");
    printf("A.msg = %.*s (%zu bytes)\n", (int)msg.len, msg.ptr, msg.len);
    run(A, "A", "print(reenter())  print('runs', nested_runs)");
    printf("A.touched = %d\n", mini_get_type(A, "touched") == MiniType_Number);

    /* Calling back: Mini -> C -> Mini, nested, with errors passing through. */
    run(A, "A",
        "function square(x) return x * x end\n"
        "print(apply(square, 12), apply(apply, square, 3))\n"
        "function down(n) if n == 0 then return 0 end return apply(down, n - 1) + 1 end\n"
        "print('down', down(50))");
    run(A, "A", "print(down(1000))");
    run(A, "A", "local function bad(x) error('bad ' .. x) end\nprint(apply(bad, 'arg'))");
    run(A, "A", "print(apply(down, -1 + 'x'))");
    run(A, "A",
        "local t = {5, 1, 4, 2, 3}\n"
        "table.sort(t, function(a, b) return apply(function() return a > b end) end)\n"
        "print(table.concat(t, ' '))\n"
        "table.sort(t, function(a, b) if a == 1 then error('cmp ' .. b) end return a < b end)");
    printf("A.top after errors = %llu\n", (unsigned long long)mini_top(A));
    /* The host calls Mini functions directly. */
    call_global(A, "square", 9);
    call_global(A, "down", 20);
    call_global(A, "greet", 1);
    call_global(A, "nothing", 1);
    run(B, "B", "print('B asks A', ask_a(5), ask_a(x))");
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

    /* GC stress during re-entry: every callback allocates, the comparator
     * builds strings, and collections run inside nested runs. */
    mini_set_gc_stress(A, true);
    before = mini_gc_count(A);
    run(A, "A",
        "local t = {}\n"
        "for i = 1, 200 do t[i] = apply(function(s, n) return s .. n end, 'k', (i * 7919) % 200) end\n"
        "table.sort(t, function(a, b) return apply(function() return (a .. '') < (b .. '') end) end)\n"
        "local ok = 0\n"
        "for i = 2, 200 do if t[i - 1] <= t[i] then ok = ok + 1 end end\n"
        "print('sorted under stress', ok, t[1], t[200], down(30))");
    mini_set_gc_stress(A, false);
    printf("collections in nested runs: %s\n", mini_gc_count(A) - before > 1000 ? "many" : "few");

    /* Into another state under GC stress: B's natives run A, which
     * allocates. A's collections see B's frames on the thread's shadow
     * stack and must leave B's objects alone. */
    run(A, "A", "function square(x) local t = {} for i = 1, 30 do t[i] = {s = 'a' .. i} end return x * x end");
    mini_set_gc_stress(A, true);
    mini_set_gc_stress(B, true);
    run(B, "B",
        "local keep = {}\n"
        "for i = 1, 100 do keep[i] = {s = 'b' .. i} end\n"
        "local s = 0\n"
        "for i = 1, 200 do\n"
        "  local t = {x = 'v' .. i, y = keep}\n"
        "  s = s + ask_a(i)\n"
        "  if t.x ~= 'v' .. i then error('corrupt ' .. i) end\n"
        "end\n"
        "local bad = 0\n"
        "for i = 1, 100 do if keep[i].s ~= 'b' .. i then bad = bad + 1 end end\n"
        "print('B asks A under stress', s, bad)");
    mini_set_gc_stress(A, false);
    mini_set_gc_stress(B, false);

    mini_free(A);
    /* B outlives A. */
    run(B, "B", "print('B still here', x, y)");

    /* Internal errors. A native calls back into Mini, and in that nested
     * run another native fails as a bug in the interpreter would
     * (mini_debug_panic). The inner mini_call returns
     * MiniStatus_Internal; apply passes it on, and the outer
     * mini_dostring returns it too. The state is poisoned: free it.
     * Other states carry on, nesting as deep as before. */
    MiniState *C = mini_new();
    mini_register(C, "apply", native_apply, NULL);
    mini_register(C, "boom", native_boom, NULL);
    run(C, "C", "print(apply(apply, boom, 'nested'))  print('never printed')");
    run(C, "C", "print('never printed either')");
    mini_free(C);
    mini_register(B, "apply", native_apply, NULL);
    run(B, "B",
        "function down(n) if n == 0 then return 0 end return apply(down, n - 1) + 1 end\n"
        "print('B down', down(" DEEPEST "))");
    mini_free(B);
    return 0;
}
