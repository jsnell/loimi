/* Independent Mini states on different threads (docs/language.md §10.1).
 *
 *   loimi build examples/mini/embed.lm --lib -o libmini.a --header mini.h
 *   clang-19 -I. examples/embed/threads.c libmini.a -lm -pthread -o threads
 *
 * Four threads each create their own state and run allocation-heavy
 * scripts at the same time: tables of strings sorted with a comparator
 * that calls back through a C native into a nested run, odd threads under
 * GC stress (a collection at every safepoint). Every thread also makes
 * its state fail with an internal error (a panic caught by the export's
 * #[catch_panic]) while the others keep running, checks that the message
 * it gets back is its own, and carries on with a fresh state. Then the
 * states change hands: each thread runs a state another thread created
 * (one thread at a time per state is all a state asks for). The output is
 * printed by the main thread after joining, so it is deterministic.
 *
 * Build with -fsanitize=thread (and the library with
 * --sanitize=thread) to check for data races.
 */
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "mini.h"

#define NTHREADS 4

/* apply(f, ...): f(...), called back from C (a nested run). */
static int32_t native_apply(MiniState *S, void *ud) {
    (void)ud;
    uint64_t n = mini_argc(S);
    for (uint64_t i = 0; i < n; i++) mini_push_arg(S, i);
    if (mini_call(S, n == 0 ? 0 : n - 1) != MiniStatus_Ok) return mini_raise_top(S);
    mini_return_top(S);
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

/* Builds n records with string keys, sorts them through apply (a nested
 * run per comparison), and counts the ordered neighbours: n - 1. */
static const char *WORK =
    "local function mk(n)\n"
    "  local t = {}\n"
    "  for i = 1, n do t[i] = {k = tag .. ':' .. ((i * 7919) % n), v = i} end\n"
    "  return t\n"
    "end\n"
    "local t = mk(N)\n"
    "table.sort(t, function(a, b) return apply(function() return a.k < b.k end) end)\n"
    "local ok = 0\n"
    "for i = 2, N do if t[i - 1].k <= t[i].k then ok = ok + 1 end end\n"
    "local s = ''\n"
    "for i = 1, 50 do s = s .. string.sub(t[i].k, 1, 1) end\n"
    "rounds = (rounds or 0) + 1\n"
    "result = ok + #s";

typedef struct {
    int id;
    MiniState *S;        /* this thread's state, handed on after phase 1 */
    MiniState *given;    /* phase 2: a state another thread created */
    int work_ok;         /* rounds whose result was right */
    int rounds;
    int caught;          /* internal error caught, with this thread's message */
    char boom_msg[96];
    int runtime_ok;      /* a runtime error with this thread's message */
    int stressed_many;   /* collections under stress: more than the rounds */
    int phase2_ok;
} Job;

#define ROUNDS 6
#define N 300

static MiniState *new_state(Job *j) {
    MiniState *S = mini_new();
    if (!S) { fprintf(stderr, "mini_new failed\n"); exit(1); }
    mini_register(S, "apply", native_apply, NULL);
    mini_register(S, "boom", native_boom, NULL);
    char tag[16];
    snprintf(tag, sizeof tag, "t%d", j->id);
    mini_set_string(S, "tag", (loimi_str){ tag, strlen(tag) });
    mini_set_integer(S, "N", N);
    if (j->id % 2) mini_set_gc_stress(S, true);
    return S;
}

static int run_work(MiniState *S) {
    if (mini_dostring(S, (void *)WORK, strlen(WORK)) != MiniStatus_Ok) {
        loimi_str e = mini_error(S);
        fprintf(stderr, "work failed: %.*s\n", (int)e.len, e.ptr);
        return 0;
    }
    double r;
    return mini_get_number(S, "result", &r) && r == (N - 1) + 50;
}

static void *phase1(void *arg) {
    Job *j = arg;
    j->S = new_state(j);
    uint64_t gc0 = mini_gc_count(j->S);
    for (int r = 0; r < ROUNDS; r++) {
        j->work_ok += run_work(j->S);
        j->rounds++;
        if (r == ROUNDS / 2) {
            /* A runtime error: the message names this thread. */
            const char *bad = "error('bad ' .. tag)";
            if (mini_dostring(j->S, (void *)bad, strlen(bad)) == MiniStatus_Runtime) {
                loimi_str e = mini_error(j->S);
                char want[32];
                snprintf(want, sizeof want, "bad t%d", j->id);
                j->runtime_ok = e.len >= strlen(want) && memcmp(e.ptr + e.len - strlen(want), want, strlen(want)) == 0;
            }
            /* An internal error two runs deep, while the other threads run.
             * The panic message buffer is per thread: it must be ours. */
            const char *b = "apply(apply, boom, tag)";
            if (mini_dostring(j->S, (void *)b, strlen(b)) == MiniStatus_Internal) {
                loimi_str e = mini_error(j->S);
                snprintf(j->boom_msg, sizeof j->boom_msg, "%.*s", (int)e.len, e.ptr);
                char want[64];
                snprintf(want, sizeof want, "internal error: boom: t%d", j->id);
                j->caught = strcmp(j->boom_msg, want) == 0;
            }
            /* The state is poisoned; a fresh one carries on. */
            mini_free(j->S);
            j->S = new_state(j);
            gc0 = mini_gc_count(j->S);
        }
    }
    j->stressed_many = mini_gc_count(j->S) - gc0 > 1000;
    return NULL;
}

static void *phase2(void *arg) {
    Job *j = arg;
    int ok = run_work(j->given);
    double rounds = 0;
    ok = ok && mini_get_number(j->given, "rounds", &rounds) && rounds == ROUNDS - ROUNDS / 2;
    j->phase2_ok = ok;
    return NULL;
}

int main(void) {
    Job jobs[NTHREADS];
    pthread_t th[NTHREADS];
    memset(jobs, 0, sizeof jobs);
    for (int i = 0; i < NTHREADS; i++) {
        jobs[i].id = i;
        if (pthread_create(&th[i], NULL, phase1, &jobs[i]) != 0) { perror("pthread_create"); return 1; }
    }
    for (int i = 0; i < NTHREADS; i++) pthread_join(th[i], NULL);
    for (int i = 0; i < NTHREADS; i++) {
        Job *j = &jobs[i];
        printf("t%d: %d/%d rounds ok, runtime error %s, internal error %s (%s)%s\n", j->id, j->work_ok, j->rounds,
               j->runtime_ok ? "ok" : "WRONG", j->caught ? "caught" : "WRONG", j->boom_msg,
               j->id % 2 ? (j->stressed_many ? ", stressed" : ", NOT STRESSED") : "");
    }
    /* Phase 2: thread i runs the state thread i + 1 created. */
    for (int i = 0; i < NTHREADS; i++) jobs[i].given = jobs[(i + 1) % NTHREADS].S;
    for (int i = 0; i < NTHREADS; i++) pthread_create(&th[i], NULL, phase2, &jobs[i]);
    for (int i = 0; i < NTHREADS; i++) pthread_join(th[i], NULL);
    for (int i = 0; i < NTHREADS; i++)
        printf("t%d ran t%d's state: %s\n", i, (i + 1) % NTHREADS, jobs[i].phase2_ok ? "ok" : "WRONG");
    /* And the main thread, which has run no Mini code yet, uses them too. */
    int ok = 0;
    for (int i = 0; i < NTHREADS; i++) ok += run_work(jobs[i].S);
    printf("main thread ran all %d states: %d ok\n", NTHREADS, ok);
    for (int i = 0; i < NTHREADS; i++) mini_free(jobs[i].S);
    return 0;
}
