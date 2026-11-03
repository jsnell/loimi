/* A C host driving Mini's debugging API (examples/mini/embed.lm, built
 * with `--cfg debug`): the API a debug adapter uses (examples/mini/dap).
 *
 *   loimi build --cfg debug examples/mini/embed.lm --lib -o libmini.a --header mini.h
 *   clang-19 -I. examples/embed/debug.c libmini.a -lm -pthread -o debug && ./debug
 *
 * The stop callback prints each stop (reason, frames, variables) and
 * answers from a script of resume actions. Covered: stop on entry,
 * breakpoints by file and line (pending until code is loaded, moved off
 * a blank line), conditions (true, false, failing), hit conditions,
 * step/next/finish, locals with upvalues, globals without the library,
 * lazy expansion of nested tables, evaluation (expressions, statements,
 * errors, calls that would hit breakpoints but do not), deleting
 * breakpoints, break on error, and a pause from another thread; all
 * under GC stress.
 */
#include <pthread.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

#include "mini.h"

#define S_(lit) LOIMI_STR(lit)

static loimi_str str(const char *s) { return (loimi_str){ s, strlen(s) }; }

static const char *reason_name(MiniStop r) {
    switch (r) {
    case MiniStop_Entry: return "entry";
    case MiniStop_Breakpoint: return "breakpoint";
    case MiniStop_Step: return "step";
    case MiniStop_Pause: return "pause";
    case MiniStop_Error: return "error";
    }
    return "?";
}

static void print_var(void *ud, loimi_str name, loimi_str value, uint32_t child) {
    int depth = *(int *)ud;
    printf("%*s%.*s = %.*s%s\n", 4 + 2 * depth, "", (int)name.len, name.ptr, (int)value.len, value.ptr, child ? " {...}" : "");
}

/* Print the variables of `ref`, expanding tables `depth` levels deep. */
struct expand { MiniState *s; int depth; int max; };
static void expand_var(void *ud, loimi_str name, loimi_str value, uint32_t child);
static void vars(MiniState *s, uint32_t ref, int depth, int max) {
    struct expand e = { s, depth, max };
    mini_debug_variables(s, ref, 0, 0, expand_var, &e);
}
static void expand_var(void *ud, loimi_str name, loimi_str value, uint32_t child) {
    struct expand *e = ud;
    print_var(&e->depth, name, value, child);
    if (child && e->depth < e->max) vars(e->s, child, e->depth + 1, e->max);
}

static void eval(MiniState *s, uint32_t level, const char *expr) {
    loimi_str out;
    uint32_t child;
    MiniStatus st = mini_debug_eval(s, level, str(expr), &out, &child);
    printf("    eval #%u %s -> %s%.*s%s\n", level, expr, st == MiniStatus_Ok ? "" : "error: ", (int)out.len, out.ptr, child ? " {...}" : "");
    if (child) vars(s, child, 1, 1);
}

/* The script: what to do at each stop. */
static const MiniResume *script;
static int stops;

static MiniResume on_stop(MiniState *s, void *ud, MiniStop reason, uint32_t bp) {
    (void)ud;
    fflush(stdout);
    printf("stop %d: %s", ++stops, reason_name(reason));
    if (reason == MiniStop_Breakpoint) printf(" %u", bp);
    loimi_str msg = mini_debug_message(s);
    if (msg.len) printf(" (%.*s)", (int)msg.len, msg.ptr);
    printf("\n");
    uint32_t n = mini_debug_depth(s);
    for (uint32_t i = 0; i < n; i++) {
        loimi_str func, file;
        uint32_t line;
        if (!mini_debug_frame(s, i, &func, &file, &line)) printf("  frame %u missing\n", i);
        else printf("  #%u %.*s:%u %.*s\n", i, (int)file.len, file.ptr, line, (int)func.len, func.ptr);
    }
    if (mini_debug_frame(s, n, &(loimi_str){0}, &(loimi_str){0}, &(uint32_t){0})) printf("  frame past the end\n");
    printf("  locals:\n");
    vars(s, mini_debug_scope(s, 0, MiniScope_Locals), 0, 2);
    return *script++;
}

static const MiniResume first[] = {
    MiniResume_Continue, /* entry */
    MiniResume_Next,     /* breakpoint 1 at line 3, when a == 3 */
    MiniResume_Finish,   /* line 4 */
    MiniResume_Step,     /* back at line 10 */
    MiniResume_Continue, /* line 11 */
    MiniResume_Continue, /* breakpoint 3, from its 4th hit on: i = 4 */
    MiniResume_Continue, /* and i = 5: deleted */
    MiniResume_Continue, /* the failing condition */
    MiniResume_Continue, /* the error */
};

static const char *prog =
    "local base = 100\n"                        /* 1 */
    "local function add(a, b)\n"                /* 2 */
    "  local s = a + b + base\n"                /* 3 */
    "  return s\n"                              /* 4 */
    "end\n"                                     /* 5 */
    "\n"                                        /* 6 */
    "local t = {10, 20, name = \"t\", sub = {k = true}}\n" /* 7 */
    "total = 0\n"                               /* 8 */
    "for i = 1, 5 do\n"                         /* 9 */
    "  total = total + add(i, t[1])\n"          /* 10 */
    "end\n"                                     /* 11 */
    "print(\"total\", total)\n"                 /* 12 */
    "local u = nil\n"                           /* 13 */
    "print(u.field)\n";                         /* 14 */

/* Stop 1 is at entry; set more there. */
static MiniResume on_stop_first(MiniState *s, void *ud, MiniStop reason, uint32_t bp) {
    if (stops == 0) {
        /* Line 6 is blank: the editor moves it to 7. */
        printf("code line for 6: %u\n", mini_debug_code_line(s, S_("prog.mini"), 6));
        uint32_t b = mini_debug_set_break(s, S_("prog.mini"), 10);
        printf("set 10: id %u sites %u, bad hit condition: %d\n", b, mini_debug_break_sites(s, b),
               mini_debug_condition(s, b, S_(""), S_("about 3")));
        printf("hit condition >= 4: %d\n", mini_debug_condition(s, b, S_(""), S_(">= 4")));
        uint32_t c = mini_debug_set_break(s, S_("prog.mini"), 12);
        printf("condition that fails: %d\n", mini_debug_condition(s, c, S_("nosuch.field"), S_("")));
        printf("delete 99: %d\n", mini_debug_delete(s, 99));
    }
    MiniResume r = on_stop(s, ud, reason, bp);
    if (stops == 2) {
        /* At the conditional breakpoint in add: expressions. */
        eval(s, 0, "a * 2 + b");
        eval(s, 0, "base");   /* an upvalue */
        eval(s, 0, "t");      /* not visible in add: a global, nil */
        eval(s, 1, "t.sub");  /* in the caller's frame */
        eval(s, 1, "add(3, 0)"); /* would hit breakpoint 1 (a == 3), but it is suspended */
        eval(s, 0, "seen = a");  /* a statement */
        eval(s, 0, "seen");
        eval(s, 0, "1 +");
        eval(s, 0, "nil .. 1");
        eval(s, 1, "add(nil, 1)"); /* an error in a function: its line */
        printf("  globals:\n");
        vars(s, mini_debug_scope(s, 0, MiniScope_Globals), 0, 0);
    }
    if (stops == 7) {
        printf("delete %u: %d\n", bp, mini_debug_delete(s, bp));
    }
    return r;
}

/* A loop that only the pause ends: the host sets `done` at the stop. */
static const char *spin =
    "n = 0\n"
    "while not done do n = n + 1 end\n"
    "print(\"spun\", n > 0)\n";

static void *pauser(void *arg) {
    struct timespec ts = { 0, 50 * 1000 * 1000 };
    nanosleep(&ts, NULL);
    mini_debug_pause(arg);
    return NULL;
}

static MiniResume on_pause(MiniState *s, void *ud, MiniStop reason, uint32_t bp) {
    (void)ud;
    (void)bp;
    loimi_str func, file;
    uint32_t line;
    mini_debug_frame(s, 0, &func, &file, &line);
    printf("stop: %s at %.*s:%u\n", reason_name(reason), (int)file.len, file.ptr, line);
    eval(s, 0, "done = true");
    return MiniResume_Continue;
}

int main(void) {
    MiniState *s = mini_new();
    mini_set_gc_stress(s, true); /* evaluations and expansions keep what they use alive */
    mini_debug_attach(s, on_stop_first, NULL);
    /* Set before the code is loaded: pending, then applied. */
    uint32_t b1 = mini_debug_set_break(s, S_("dir/prog.mini"), 3);
    printf("set 3 before loading: id %u sites %u\n", b1, mini_debug_break_sites(s, b1));
    printf("condition: %d\n", mini_debug_condition(s, b1, S_("a == 3"), S_("")));
    if (mini_debug_load(s, S_("prog.mini"), str("x = = 1")) != MiniStatus_Syntax) printf("no syntax error?\n");
    loimi_str e = mini_error(s);
    printf("syntax error: %.*s\n", (int)e.len, e.ptr);
    if (mini_debug_load(s, S_("prog.mini"), str(prog)) != MiniStatus_Ok) return 1;
    printf("breakpoint 1 now has %u site(s)\n", mini_debug_break_sites(s, b1));
    mini_debug_stop_on_entry(s);
    mini_debug_break_on_error(s, true);
    script = first;
    MiniStatus st = mini_call(s, 0);
    fflush(stdout);
    e = mini_error(s);
    printf("run: status %d: %.*s\n", st, (int)e.len, e.ptr);
    printf("stops: %d of %d scripted\n", stops, (int)(sizeof first / sizeof first[0]));

    /* Pause from another thread. */
    mini_debug_attach(s, on_pause, NULL);
    mini_debug_break_on_error(s, false);
    if (mini_debug_load(s, S_("spin.mini"), str(spin)) != MiniStatus_Ok) return 1;
    pthread_t t;
    pthread_create(&t, NULL, pauser, s);
    st = mini_call(s, 0);
    pthread_join(t, NULL);
    fflush(stdout);
    printf("spin: status %d\n", st);
    mini_free(s);
    return 0;
}
