/* Panics in tests/embed/panic.lm become error returns. Expected output:
 * panic_host.expected. */
#include <signal.h>
#include <stdio.h>
#include <sys/wait.h>
#include <unistd.h>
#include "panic.h"

static void say(const char *what) {
    /* The message names the source as the build saw it: keep the file name. */
    loimi_str m = last_panic();
    const char *p = m.ptr, *end = m.ptr + m.len;
    for (const char *q = m.ptr; q < end; q++)
        if (*q == '/') p = q + 1;
    printf("%s: %.*s\n", what, (int)(end - p), p);
}

static int64_t plus_one(int64_t x) { return x + 1; }
static int64_t oob_inside(int64_t x) { return lookup_unprotected((uint64_t)x); }
static int64_t caught_inside(int64_t x) { return lookup((uint64_t)x); }

/* The callback of the vm's HOST op. */
enum { CB_NORMAL, CB_PANIC, CB_DEEP, CB_CAUGHT } cb_mode = CB_NORMAL;
static unsigned max_depth, after[8];
static int n_after;

static int64_t cb(RCtx *c, int64_t a) {
    if (rvm_depth() > max_depth) max_depth = rvm_depth();
    switch (cb_mode) {
    case CB_NORMAL: /* a nested run of x + 1, after a collection */
        rvm_collect(c);
        return rvm_call(c, 2, a);
    case CB_PANIC: /* a nested run that fails a bounds check */
        return rvm_call(c, 1, a);
    case CB_DEEP: /* nested a + 1 deep, then the failing one */
        return a > 0 ? rvm_call(c, 0, a - 1) : rvm_call(c, 1, 9);
    case CB_CAUGHT: { /* the same, caught by the innermost call */
        if (a > 0) return rvm_call(c, 0, a - 1);
        int64_t r = rvm_call_caught(c, 1, 9);
        if (n_after < 8) after[n_after++] = rvm_depth();
        return r;
    }
    }
    return 0;
}

int main(void) {
    printf("lookup %lld %lld\n", (long long)lookup(2), (long long)lookup(9));
    say("lookup");
    int64_t odd[] = { 1, 3, 5 }, mixed[] = { 1, 4, 6 };
    printf("first_even %d %d\n", first_even(mixed, 3), first_even(odd, 3));
    say("first_even");
    printf("must_be_positive %d %d\n", must_be_positive(3), must_be_positive(-3));
    say("must_be_positive");
    printf("divide %lld %lld\n", (long long)divide(7, 2), (long long)divide(7, 0));
    say("divide");
    check_name(LOIMI_STR("ok"));
    check_name(LOIMI_STR(""));
    say("check_name");
    Table *t = table_new(10);
    printf("table_new %s %s\n", t ? "ok" : "NULL", table_new((uint64_t)1 << 62) ? "ok" : "NULL");
    say("table_new");
    table_free(t);
    void *b = try_bytes(100);
    printf("try_bytes %s %s\n", b ? "ok" : "NULL", try_bytes((uint64_t)1 << 62) ? "ok" : "NULL");
    free_bytes(b);
    printf("with_callback %lld %lld %lld %lld\n", (long long)with_callback(plus_one, 1),
           (long long)with_callback(oob_inside, 9), (long long)with_callback(caught_inside, 9),
           (long long)with_callback(caught_inside, 1));
    printf("caught_in_loimi %lld %lld %lld\n", (long long)caught_in_loimi(3), (long long)caught_in_loimi(4),
           (long long)caught_in_loimi(99));
    say("caught_in_loimi");
    World *w = world_new();
    printf("world %lld %lld %lld", (long long)world_fail(w, 2), (long long)world_fail(w, 100), (long long)world_churn(w, 10));
    printf(" %s\n", shadow_stack_empty() ? "empty" : "left over");
    world_free(w);
    /* Panics in nested vm runs (see rvm_* in panic.lm). */
    w = world_new();
    RCtx *c = rvm_new(w, cb);
    printf("rvm normal %lld depth %u\n", (long long)rvm_run(c, 0, 1), rvm_depth());
    cb_mode = CB_PANIC;
    printf("rvm panic in nested run %lld depth %u", (long long)rvm_run(c, 0, 9), rvm_depth());
    printf(" %s\n", shadow_stack_empty() ? "empty" : "left over");
    say("rvm");
    cb_mode = CB_DEEP;
    printf("rvm panic 4 runs deep %lld depth %u (max %u)", (long long)rvm_run(c, 0, 3), rvm_depth(), max_depth);
    printf(" %s\n", shadow_stack_empty() ? "empty" : "left over");
    cb_mode = CB_CAUGHT;
    printf("rvm caught in between %lld depth %u", (long long)rvm_run(c, 0, 3), rvm_depth());
    printf(" %s, depths after the catch:", shadow_stack_empty() ? "empty" : "left over");
    for (int i = 0; i < n_after; i++) printf(" %u", after[i]);
    printf("\n");
    cb_mode = CB_NORMAL;
    printf("rvm normal again %lld %lld depth %u churn %lld", (long long)rvm_run(c, 0, 1), (long long)rvm_run(c, 1, 2),
           rvm_depth(), (long long)world_churn(w, 10));
    printf(" %s\n", shadow_stack_empty() ? "empty" : "left over");
    rvm_free(c);
    world_free(w);
    /* Still working after all that. */
    printf("lookup again %lld\n", (long long)lookup(0));
    /* Without a catch anywhere, a panic still aborts the host. */
    fflush(stdout);
    pid_t pid = fork();
    if (pid == 0) {
        close(2);
        lookup_unprotected(9);
        _exit(0);
    }
    int st = 0;
    waitpid(pid, &st, 0);
    printf("uncaught: %s\n", WIFSIGNALED(st) && WTERMSIG(st) == SIGABRT ? "SIGABRT" : "other");
    return 0;
}
