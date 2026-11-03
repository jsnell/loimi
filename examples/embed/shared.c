/* One Mini state shared by several threads, one at a time (a global
 * interpreter lock; docs/language.md §10.1, docs/stdlib/gc.md).
 *
 *   loimi build examples/mini/embed.lm --lib --cfg gil -o libmini.a --header mini.h
 *   clang-19 -I. examples/embed/shared.c libmini.a -lm -pthread -o shared
 *
 * Built with `--cfg gil`, a state has a lock that a thread must hold to
 * call the API. The main thread creates the state (and holds the lock),
 * defines functions and shared tables, then:
 *
 *  1. Hand-over at a back-edge: it starts a thread that attaches (and
 *     so waits for the lock) to set `ready`, and runs `spin()`, a Lua
 *     loop that ends only when `ready` is set. Nothing in the loop calls
 *     a native or allocates: it ends because the running code polls the
 *     lock at the loop's back-edge and hands it over.
 *  2. Shared state under GC stress: four threads attach and call the same
 *     Lua functions on shared tables, while the main thread waits with the
 *     lock released. `work` builds a table of strings, calls `nap` (a C
 *     native that releases the lock while it sleeps) every few
 *     iterations, sums through a coroutine, and adds to a shared table and
 *     a shared counter; `nested` naps two runs deep (C -> Lua -> C). Between
 *     calls each thread releases the lock with a string on its own API
 *     value stack, which other threads' collections must keep. A
 *     collection runs at every safepoint (stress), so collections happen
 *     on every thread while the others are stopped mid-call.
 *
 * The output is printed by the main thread after joining, so it is
 * deterministic. Build with -fsanitize=thread (and the library with
 * --sanitize=thread) to check for data races.
 */
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include "mini.h"

#define NTHREADS 4
#define ROUNDS 6
#define N 200

static MiniState *S;

static void pause_us(long us) {
    struct timespec ts = { 0, us * 1000 };
    nanosleep(&ts, NULL);
}

/* nap(): let go of the lock for a moment, as blocking I/O would. */
static int32_t native_nap(MiniState *s, void *ud) {
    (void)ud;
    mini_unlock(s);
    pause_us(20);
    mini_lock(s);
    return 0;
}

/* apply(f, ...): f(...), called back from C (a nested run). */
static int32_t native_apply(MiniState *s, void *ud) {
    (void)ud;
    uint64_t n = mini_argc(s);
    for (uint64_t i = 0; i < n; i++) mini_push_arg(s, i);
    if (mini_call(s, n == 0 ? 0 : n - 1) != MiniStatus_Ok) return mini_raise_top(s);
    mini_return_top(s);
    return 0;
}

static const char *SETUP =
    "total = 0\n"
    "counts = {}\n"
    "ready = false\n"
    "function work(id)\n"
    "  local n = N\n"
    "  local t = {}\n"
    "  for i = 1, n do\n"
    "    t[i] = 'w' .. id .. ':' .. i\n"
    "    if i % 16 == 0 then nap() end\n"
    "  end\n"
    "  local gen = coroutine.wrap(function()\n"
    "    for i = 1, n do coroutine.yield(#t[i]) end\n"
    "  end)\n"
    "  local len = 0\n"
    "  for i = 1, n do len = len + gen() end\n"
    "  counts[id] = (counts[id] or 0) + len\n"
    "  total = total + 1\n"
    "  return len\n"
    "end\n"
    "function nested(id)\n"
    "  return apply(function() nap(); return id * 10 end)\n"
    "end\n"
    "function spin()\n"
    "  local n = 0\n"
    "  while not ready do n = n + 1 end\n"
    "  return n\n"
    "end\n"
    "function set_ready() ready = true end\n";

/* The length `work(id)` returns: the strings 'w<id>:<i>'. */
static long expected_len(int id) {
    long len = 0;
    char buf[32];
    for (int i = 1; i <= N; i++) len += snprintf(buf, sizeof buf, "w%d:%d", id, i);
    return len;
}

static int call_int(const char *fn, int arg, double *out) {
    mini_push_global(S, (void *)fn);
    mini_push_integer(S, arg);
    if (mini_call(S, 1) != MiniStatus_Ok) {
        loimi_str e = mini_error(S);
        fprintf(stderr, "%s(%d) failed: %.*s\n", fn, arg, (int)e.len, e.ptr);
        mini_pop(S, 1);
        return 0;
    }
    int ok = mini_to_number(S, -1, out);
    mini_pop(S, 1);
    return ok;
}

static void *setter(void *arg) {
    (void)arg;
    mini_thread_attach(S);
    mini_push_global(S, "set_ready");
    int ok = mini_call(S, 0) == MiniStatus_Ok;
    mini_pop(S, 1);
    if (!mini_thread_detach(S)) ok = 0;
    return ok ? (void *)1 : NULL;
}

typedef struct {
    int id;
    int work_ok, nested_ok, kept_ok, detached;
} Job;

static void *worker(void *arg) {
    Job *j = arg;
    mini_thread_attach(S);
    for (int r = 0; r < ROUNDS; r++) {
        double v;
        if (call_int("work", j->id, &v) && v == expected_len(j->id)) j->work_ok++;
        if (call_int("nested", j->id, &v) && v == j->id * 10) j->nested_ok++;
        /* A value on this thread's API stack while it lets go of the lock. */
        char keep[32];
        int n = snprintf(keep, sizeof keep, "keep-%d-%d", j->id, r);
        mini_push_string(S, (loimi_str){ keep, (size_t)n });
        mini_unlock(S);
        pause_us(50);
        mini_lock(S);
        loimi_str k = mini_to_string(S, -1);
        if (k.len == (size_t)n && memcmp(k.ptr, keep, n) == 0) j->kept_ok++;
        mini_pop(S, 1);
    }
    j->detached = mini_thread_detach(S);
    return NULL;
}

int main(void) {
    alarm(120); /* a hand-over that never happens hangs: fail instead */
    S = mini_new();
    if (!S) return 1;
    mini_register(S, "nap", native_nap, NULL);
    mini_register(S, "apply", native_apply, NULL);
    mini_set_integer(S, "N", N);
    if (mini_dostring(S, (void *)SETUP, strlen(SETUP)) != MiniStatus_Ok) {
        loimi_str e = mini_error(S);
        fprintf(stderr, "setup failed: %.*s\n", (int)e.len, e.ptr);
        return 1;
    }

    /* 1. The setter waits for the lock while the main thread spins. */
    pthread_t st;
    pthread_create(&st, NULL, setter, NULL);
    double spins = 0;
    int spun = call_int("spin", 0, &spins);
    mini_unlock(S);
    void *setter_ok;
    pthread_join(st, &setter_ok);
    mini_lock(S);
    printf("spin: ended by another thread: %s\n", spun && setter_ok ? "yes" : "NO");
    printf("main thread detaches: %s\n", mini_thread_detach(S) ? "YES (wrong)" : "no (it created the state)");

    /* 2. Four threads share the state, under GC stress. */
    mini_set_gc_stress(S, true);
    uint64_t gc0 = mini_gc_count(S);
    uint64_t sw0 = mini_lock_switches(S);
    Job jobs[NTHREADS];
    pthread_t th[NTHREADS];
    memset(jobs, 0, sizeof jobs);
    mini_unlock(S);
    for (int i = 0; i < NTHREADS; i++) {
        jobs[i].id = i + 1;
        pthread_create(&th[i], NULL, worker, &jobs[i]);
    }
    for (int i = 0; i < NTHREADS; i++) pthread_join(th[i], NULL);
    mini_lock(S);
    mini_set_gc_stress(S, false);
    for (int i = 0; i < NTHREADS; i++) {
        Job *j = &jobs[i];
        printf("t%d: work %d/%d, nested %d/%d, kept its value %d/%d, detached %s\n", j->id, j->work_ok, ROUNDS,
               j->nested_ok, ROUNDS, j->kept_ok, ROUNDS, j->detached ? "yes" : "NO");
    }
    /* The shared table and counter, seen from the main thread. */
    const char *check =
        "local ok = true\n"
        "for id = 1, 4 do ok = ok and counts[id] == ROUNDS * LEN[id] end\n"
        "summary = 'total ' .. total .. ', counts ' .. (ok and 'ok' or 'WRONG')";
    char lens[128];
    snprintf(lens, sizeof lens, "ROUNDS = %d LEN = {%ld, %ld, %ld, %ld}", ROUNDS, expected_len(1), expected_len(2),
             expected_len(3), expected_len(4));
    mini_dostring(S, lens, strlen(lens));
    if (mini_dostring(S, (void *)check, strlen(check)) != MiniStatus_Ok) {
        loimi_str e = mini_error(S);
        fprintf(stderr, "check failed: %.*s\n", (int)e.len, e.ptr);
    }
    loimi_str sum = mini_get_string(S, "summary");
    printf("%.*s\n", (int)sum.len, sum.ptr);
    printf("collections while shared: %s\n", mini_gc_count(S) - gc0 > 500 ? "many" : "FEW");
    printf("lock hand-overs: %s\n", mini_lock_switches(S) - sw0 > 100 ? "many" : "FEW");
    mini_free(S);
    return 0;
}
