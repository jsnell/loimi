/* mini-dap: a Debug Adapter Protocol server for Mini, over stdio.
 *
 *   loimi build --cfg debug examples/mini/embed.lm --lib -o libmini.a --header mini.h
 *   clang-19 -I. examples/mini/dap/mini_dap.c libmini.a -lm -pthread -o mini-dap
 *
 * An editor starts `mini-dap` as its debug adapter for Mini scripts and
 * talks DAP to it (JSON messages with Content-Length headers). The
 * adapter runs the script itself, in-process, through Mini's debugging
 * API (examples/mini/embed.lm, `mini_debug_*`).
 *
 * Requests: initialize, launch (program, stopOnEntry), setBreakpoints
 * (with condition and hitCondition), setExceptionBreakpoints (filter
 * "error": uncaught errors, on by default), configurationDone, threads,
 * stackTrace, scopes, variables, evaluate, continue, next, stepIn,
 * stepOut, pause, disconnect, terminate. Events: initialized, stopped,
 * output, exited, terminated.
 *
 * Threads: the main thread runs the script and answers requests; the
 * stop callback answers them while the script is stopped, so that every
 * call into Mini happens on the thread that runs it. A reader thread
 * reads requests from stdin into a queue. While the script runs, it
 * answers `threads` itself and makes the script stop for the others
 * (`mini_debug_pause`, polled at back-edges and calls): a `pause` stops
 * it for the user, anything else (breakpoints set while it runs) for the
 * adapter, which handles the queue and continues it as it was
 * (MiniResume_Keep). The script's stdout goes into a pipe, which a third
 * thread turns into output events; at each stop and at the end, the
 * pipe is drained before the event, so that output comes first.
 *
 * Small on purpose: one script, one thread (Mini's coroutines are not
 * threads to DAP), no attach, no setVariable, no restart. The JSON
 * reader and writer below handle what DAP sends: objects, arrays,
 * strings (with \uXXXX escapes, BMP only), numbers, true, false, null.
 */
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <pthread.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "mini.h"

/* ── JSON ─────────────────────────────────────────────────────────── */

typedef enum { J_NULL, J_BOOL, J_NUM, J_STR, J_ARR, J_OBJ } jtype;

typedef struct json {
    jtype t;
    double num;          /* J_NUM, J_BOOL */
    char *str;           /* J_STR (NUL-terminated) */
    size_t n;            /* J_ARR, J_OBJ: items */
    struct json **items;
    char **keys;         /* J_OBJ */
} json;

static void jfree(json *j) {
    if (!j) return;
    for (size_t i = 0; i < j->n; i++) {
        jfree(j->items[i]);
        if (j->keys) free(j->keys[i]);
    }
    free(j->items);
    free(j->keys);
    free(j->str);
    free(j);
}

static void skip_ws(const char **p) {
    while (**p == ' ' || **p == '\t' || **p == '\n' || **p == '\r') ++*p;
}

/* Append code point `c` to `out` as UTF-8. */
static size_t put_utf8(char *out, unsigned c) {
    if (c < 0x80) { out[0] = (char)c; return 1; }
    if (c < 0x800) { out[0] = (char)(0xC0 | c >> 6); out[1] = (char)(0x80 | (c & 63)); return 2; }
    out[0] = (char)(0xE0 | c >> 12);
    out[1] = (char)(0x80 | ((c >> 6) & 63));
    out[2] = (char)(0x80 | (c & 63));
    return 3;
}

static char *parse_string(const char **p) {
    if (**p != '"') return NULL;
    ++*p;
    size_t cap = 16, len = 0;
    char *s = malloc(cap);
    for (;;) {
        char c = *(*p)++;
        if (c == 0) { free(s); return NULL; }
        if (c == '"') break;
        if (len + 4 >= cap) s = realloc(s, cap *= 2);
        if (c != '\\') { s[len++] = c; continue; }
        c = *(*p)++;
        switch (c) {
        case 'n': s[len++] = '\n'; break;
        case 't': s[len++] = '\t'; break;
        case 'r': s[len++] = '\r'; break;
        case 'b': s[len++] = '\b'; break;
        case 'f': s[len++] = '\f'; break;
        case 'u': {
            unsigned v = 0;
            for (int i = 0; i < 4; i++) {
                char h = *(*p)++;
                v = v * 16 + (unsigned)(h >= 'a' ? h - 'a' + 10 : h >= 'A' ? h - 'A' + 10 : h - '0');
            }
            len += put_utf8(s + len, v);
            break;
        }
        case 0: free(s); return NULL;
        default: s[len++] = c; /* \" \\ \/ */
        }
    }
    s[len] = 0;
    return s;
}

static json *jnew(jtype t) {
    json *j = calloc(1, sizeof *j);
    j->t = t;
    return j;
}

static json *jparse(const char **p) {
    skip_ws(p);
    json *j;
    if (**p == '{' || **p == '[') {
        int obj = **p == '{';
        char close = obj ? '}' : ']';
        j = jnew(obj ? J_OBJ : J_ARR);
        ++*p;
        skip_ws(p);
        if (**p == close) { ++*p; return j; }
        for (;;) {
            char *key = NULL;
            if (obj) {
                skip_ws(p);
                if (!(key = parse_string(p))) { jfree(j); return NULL; }
                skip_ws(p);
                if (*(*p)++ != ':') { free(key); jfree(j); return NULL; }
            }
            json *v = jparse(p);
            if (!v) { free(key); jfree(j); return NULL; }
            j->items = realloc(j->items, (j->n + 1) * sizeof *j->items);
            if (obj) j->keys = realloc(j->keys, (j->n + 1) * sizeof *j->keys), j->keys[j->n] = key;
            j->items[j->n++] = v;
            skip_ws(p);
            char c = *(*p)++;
            if (c == close) return j;
            if (c != ',') { jfree(j); return NULL; }
        }
    }
    if (**p == '"') {
        char *s = parse_string(p);
        if (!s) return NULL;
        j = jnew(J_STR);
        j->str = s;
        return j;
    }
    if (!strncmp(*p, "true", 4)) { *p += 4; j = jnew(J_BOOL); j->num = 1; return j; }
    if (!strncmp(*p, "false", 5)) { *p += 5; return jnew(J_BOOL); }
    if (!strncmp(*p, "null", 4)) { *p += 4; return jnew(J_NULL); }
    char *end;
    double d = strtod(*p, &end);
    if (end == *p) return NULL;
    *p = end;
    j = jnew(J_NUM);
    j->num = d;
    return j;
}

static json *jget(json *o, const char *key) {
    if (!o || o->t != J_OBJ) return NULL;
    for (size_t i = 0; i < o->n; i++)
        if (!strcmp(o->keys[i], key)) return o->items[i];
    return NULL;
}

static const char *jstr(json *o, const char *key, const char *def) {
    json *v = jget(o, key);
    return v && v->t == J_STR ? v->str : def;
}

static double jnum(json *o, const char *key, double def) {
    json *v = jget(o, key);
    return v && (v->t == J_NUM || v->t == J_BOOL) ? v->num : def;
}

/* A growable string, for writing JSON. */
typedef struct { char *p; size_t len, cap; } buf;

static void bput(buf *b, const char *s, size_t n) {
    if (b->len + n + 1 > b->cap) {
        b->cap = (b->len + n + 1) * 2;
        b->p = realloc(b->p, b->cap);
    }
    memcpy(b->p + b->len, s, n);
    b->len += n;
    b->p[b->len] = 0;
}

static void bf(buf *b, const char *fmt, ...) {
    char tmp[512];
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(tmp, sizeof tmp, fmt, ap);
    va_end(ap);
    bput(b, tmp, (size_t)n < sizeof tmp ? (size_t)n : sizeof tmp - 1);
}

/* `s` as a JSON string. */
static void bstr(buf *b, const char *s, size_t n) {
    bput(b, "\"", 1);
    for (size_t i = 0; i < n; i++) {
        unsigned char c = (unsigned char)s[i];
        if (c == '"' || c == '\\') { bput(b, "\\", 1); bput(b, (char *)&c, 1); }
        else if (c == '\n') bput(b, "\\n", 2);
        else if (c == '\t') bput(b, "\\t", 2);
        else if (c == '\r') bput(b, "\\r", 2);
        else if (c < 0x20) bf(b, "\\u%04x", c);
        else bput(b, (char *)&c, 1);
    }
    bput(b, "\"", 1);
}

static void bcstr(buf *b, const char *s) { bstr(b, s, strlen(s)); }
static void blstr(buf *b, loimi_str s) { bstr(b, s.ptr, s.len); }

/* ── the protocol ─────────────────────────────────────────────────── */

static int dap_out;             /* the client's end: the original stdout */
static pthread_mutex_t send_mu = PTHREAD_MUTEX_INITIALIZER;
static int seq = 1;
static FILE *trace;             /* MINI_DAP_LOG: every message, for debugging the adapter */

/* Send one message; `body` is the JSON after `"seq":N,`. */
static void send_msg(buf *body) {
    pthread_mutex_lock(&send_mu);
    buf m = { 0 };
    bf(&m, "{\"seq\":%d,", seq++);
    bput(&m, body->p, body->len);
    bput(&m, "}", 1);
    char hdr[64];
    int n = snprintf(hdr, sizeof hdr, "Content-Length: %zu\r\n\r\n", m.len);
    if (trace) fprintf(trace, "<- %s\n", m.p), fflush(trace);
    if (write(dap_out, hdr, (size_t)n) < 0 || write(dap_out, m.p, m.len) < 0) { /* the client is gone */ }
    pthread_mutex_unlock(&send_mu);
    free(m.p);
    free(body->p);
    body->p = NULL;
    body->len = body->cap = 0;
}

/* Start a response to `req`: `"type":"response",...,"body":` follows if
 * the caller adds one. */
static void respond_start(buf *b, json *req, int success, const char *message) {
    bf(b, "\"type\":\"response\",\"request_seq\":%d,\"success\":%s,\"command\":", (int)jnum(req, "seq", 0), success ? "true" : "false");
    bcstr(b, jstr(req, "command", ""));
    if (message) { bput(b, ",\"message\":", 11); bcstr(b, message); }
}

static void respond(json *req, int success, const char *message, const char *body) {
    buf b = { 0 };
    respond_start(&b, req, success, message);
    if (body) { bput(&b, ",\"body\":", 8); bput(&b, body, strlen(body)); }
    send_msg(&b);
}

static void event(const char *name, const char *body) {
    buf b = { 0 };
    bf(&b, "\"type\":\"event\",\"event\":\"%s\"", name);
    if (body) { bput(&b, ",\"body\":", 8); bput(&b, body, strlen(body)); }
    send_msg(&b);
}

static void output(const char *category, const char *s, size_t n) {
    buf b = { 0 };
    bput(&b, "{\"category\":", 12);
    bcstr(&b, category);
    bput(&b, ",\"output\":", 10);
    bstr(&b, s, n);
    bput(&b, "}", 1);
    event("output", b.p);
    free(b.p);
}

/* ── requests: a queue the reader thread fills ────────────────────── */

typedef struct node { json *msg; struct node *next; } node;
static node *q_head, *q_tail;
static pthread_mutex_t q_mu = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t q_cv = PTHREAD_COND_INITIALIZER;
static int q_eof;

static MiniState *S;            /* set once launched */
/* Shared with the reader thread. */
static int running;    /* the script runs (not stopped) */
static int user_pause; /* a `pause` request waits for its stop */
#define LOAD(x) __atomic_load_n(&(x), __ATOMIC_SEQ_CST)
#define STORE(x, v) __atomic_store_n(&(x), (v), __ATOMIC_SEQ_CST)

static void q_push(json *m) {
    node *n = calloc(1, sizeof *n);
    n->msg = m;
    pthread_mutex_lock(&q_mu);
    if (q_tail) q_tail->next = n; else q_head = n;
    q_tail = n;
    pthread_cond_signal(&q_cv);
    pthread_mutex_unlock(&q_mu);
}

/* The next request; NULL at the end of the input. With `wait` false,
 * NULL if none is queued. */
static json *q_pop(int wait) {
    pthread_mutex_lock(&q_mu);
    while (!q_head && !q_eof && wait) pthread_cond_wait(&q_cv, &q_mu);
    json *m = NULL;
    if (q_head) {
        node *n = q_head;
        q_head = n->next;
        if (!q_head) q_tail = NULL;
        m = n->msg;
        free(n);
    }
    pthread_mutex_unlock(&q_mu);
    return m;
}

static int read_full(int fd, char *p, size_t n) {
    while (n) {
        ssize_t k = read(fd, p, n);
        if (k <= 0) return 0;
        p += k;
        n -= (size_t)k;
    }
    return 1;
}

/* One framed message from stdin, or NULL at its end. */
static json *read_msg(void) {
    char line[256];
    size_t len = 0, have = 0;
    for (;;) {
        /* A header line. */
        size_t i = 0;
        for (;;) {
            char c;
            if (read(0, &c, 1) != 1) return NULL;
            if (c == '\n') break;
            if (i + 1 < sizeof line) line[i++] = c;
        }
        if (i && line[i - 1] == '\r') i--;
        line[i] = 0;
        if (i == 0) {
            if (have) break;
            continue;
        }
        if (!strncasecmp(line, "Content-Length:", 15)) { len = strtoul(line + 15, NULL, 10); have = 1; }
    }
    char *body = malloc(len + 1);
    if (!read_full(0, body, len)) { free(body); return NULL; }
    body[len] = 0;
    if (trace) fprintf(trace, "-> %s\n", body), fflush(trace);
    const char *p = body;
    json *j = jparse(&p);
    free(body);
    return j;
}

static void *reader(void *arg) {
    (void)arg;
    json *m;
    while ((m = read_msg())) {
        const char *cmd = jstr(m, "command", "");
        if (!strcmp(cmd, "threads")) {
            /* Answered here: it does not need the script stopped. */
            respond(m, 1, NULL, "{\"threads\":[{\"id\":1,\"name\":\"main\"}]}");
            jfree(m);
            continue;
        }
        if (!strcmp(cmd, "pause")) STORE(user_pause, 1);
        q_push(m);
        /* The script must stop for the request to be handled. */
        if (LOAD(running)) mini_debug_pause(S);
    }
    pthread_mutex_lock(&q_mu);
    q_eof = 1;
    pthread_cond_broadcast(&q_cv);
    pthread_mutex_unlock(&q_mu);
    if (LOAD(running)) mini_debug_pause(S);
    return NULL;
}

/* ── the script's output ──────────────────────────────────────────── */

static int out_pipe[2] = { -1, -1 };
static pthread_mutex_t out_mu = PTHREAD_MUTEX_INITIALIZER;

/* Send what the pipe holds as output events (with out_mu held). */
static void drain_locked(void) {
    char tmp[4096];
    ssize_t n;
    while ((n = read(out_pipe[0], tmp, sizeof tmp)) > 0) output("stdout", tmp, (size_t)n);
}

/* Everything the script printed so far goes out before what follows. */
static void drain(void) {
    fflush(stdout);
    pthread_mutex_lock(&out_mu);
    drain_locked();
    pthread_mutex_unlock(&out_mu);
}

static void *pump(void *arg) {
    (void)arg;
    for (;;) {
        struct pollfd p = { out_pipe[0], POLLIN, 0 };
        if (poll(&p, 1, -1) < 0 && errno != EINTR) return NULL;
        if (p.revents & (POLLERR | POLLNVAL)) return NULL;
        pthread_mutex_lock(&out_mu);
        drain_locked();
        pthread_mutex_unlock(&out_mu);
        if ((p.revents & POLLHUP) && !(p.revents & POLLIN)) return NULL;
    }
}

/* ── handling requests ────────────────────────────────────────────── */

static char *program;       /* launch's `program` */
static int stop_on_entry;
static int break_on_error = 1;
static int configured;
static int terminated;

/* Breakpoints by source path: the ids set for it last. */
typedef struct { char *path; uint32_t *ids; size_t n; } source_bps;
static source_bps *sources;
static size_t nsources;

static source_bps *source_for(const char *path) {
    for (size_t i = 0; i < nsources; i++)
        if (!strcmp(sources[i].path, path)) return &sources[i];
    sources = realloc(sources, (nsources + 1) * sizeof *sources);
    sources[nsources] = (source_bps){ strdup(path), NULL, 0 };
    return &sources[nsources++];
}

static loimi_str lstr(const char *s) { return (loimi_str){ s, strlen(s) }; }

static const char *base_name(const char *p) {
    const char *b = strrchr(p, '/');
    return b ? b + 1 : p;
}

static void set_breakpoints(json *req) {
    json *args = jget(req, "arguments");
    const char *path = jstr(jget(args, "source"), "path", "");
    source_bps *sb = source_for(path);
    for (size_t i = 0; i < sb->n; i++) mini_debug_delete(S, sb->ids[i]);
    json *list = jget(args, "breakpoints");
    size_t n = list && list->t == J_ARR ? list->n : 0;
    sb->ids = realloc(sb->ids, (n ? n : 1) * sizeof *sb->ids);
    sb->n = 0;
    buf b = { 0 };
    respond_start(&b, req, 1, NULL);
    bput(&b, ",\"body\":{\"breakpoints\":[", 24);
    for (size_t i = 0; i < n; i++) {
        json *bp = list->items[i];
        uint32_t want = (uint32_t)jnum(bp, "line", 0);
        uint32_t line = mini_debug_code_line(S, lstr(path), want);
        if (i) bput(&b, ",", 1);
        if (!line) {
            bf(&b, "{\"verified\":false,\"line\":%u,\"message\":\"no code at or after this line\"}", want);
            continue;
        }
        uint32_t id = mini_debug_set_break(S, lstr(path), line);
        if (!mini_debug_condition(S, id, lstr(jstr(bp, "condition", "")), lstr(jstr(bp, "hitCondition", "")))) {
            mini_debug_delete(S, id);
            bf(&b, "{\"verified\":false,\"line\":%u,\"message\":\"bad hit condition (N, == N, >= N, > N, <= N, < N, %% N)\"}", line);
            continue;
        }
        sb->ids[sb->n++] = id;
        bf(&b, "{\"id\":%u,\"verified\":true,\"line\":%u}", id, line);
    }
    bput(&b, "]}", 2);
    send_msg(&b);
}

static void stack_trace(json *req) {
    json *args = jget(req, "arguments");
    uint32_t start = (uint32_t)jnum(args, "startFrame", 0);
    uint32_t levels = (uint32_t)jnum(args, "levels", 0);
    uint32_t depth = mini_debug_depth(S);
    buf b = { 0 };
    respond_start(&b, req, 1, NULL);
    bput(&b, ",\"body\":{\"stackFrames\":[", 24);
    for (uint32_t i = start, k = 0; i < depth && (!levels || k < levels); i++, k++) {
        loimi_str func, file;
        uint32_t line;
        if (!mini_debug_frame(S, i, &func, &file, &line)) break;
        if (k) bput(&b, ",", 1);
        bf(&b, "{\"id\":%u,\"name\":", i + 1);
        blstr(&b, func);
        bput(&b, ",\"source\":{\"name\":", 18);
        char path[4096];
        snprintf(path, sizeof path, "%.*s", (int)file.len, file.ptr);
        bcstr(&b, base_name(path));
        bput(&b, ",\"path\":", 8);
        blstr(&b, file);
        bf(&b, "},\"line\":%u,\"column\":1}", line);
    }
    bf(&b, "],\"totalFrames\":%u}", depth);
    send_msg(&b);
}

static void scopes(json *req) {
    uint32_t level = (uint32_t)jnum(jget(req, "arguments"), "frameId", 1) - 1;
    uint32_t locals = mini_debug_scope(S, level, MiniScope_Locals);
    uint32_t globals = mini_debug_scope(S, level, MiniScope_Globals);
    if (!locals) { respond(req, 0, "no such frame", NULL); return; }
    char body[256];
    snprintf(body, sizeof body,
             "{\"scopes\":[{\"name\":\"Locals\",\"presentationHint\":\"locals\",\"variablesReference\":%u,\"expensive\":false},"
             "{\"name\":\"Globals\",\"variablesReference\":%u,\"expensive\":false}]}",
             locals, globals);
    respond(req, 1, NULL, body);
}

static void add_variable(void *ud, loimi_str name, loimi_str value, uint32_t child) {
    buf *b = ud;
    if (b->p[b->len - 1] != '[') bput(b, ",", 1);
    bput(b, "{\"name\":", 8);
    blstr(b, name);
    bput(b, ",\"value\":", 9);
    blstr(b, value);
    bf(b, ",\"variablesReference\":%u}", child);
}

static void variables(json *req) {
    json *args = jget(req, "arguments");
    buf b = { 0 };
    respond_start(&b, req, 1, NULL);
    bput(&b, ",\"body\":{\"variables\":[", 22);
    mini_debug_variables(S, (uint32_t)jnum(args, "variablesReference", 0), (uint32_t)jnum(args, "start", 0),
                         (uint32_t)jnum(args, "count", 0), add_variable, &b);
    bput(&b, "]}", 2);
    send_msg(&b);
}

static void evaluate(json *req) {
    json *args = jget(req, "arguments");
    uint32_t level = (uint32_t)jnum(args, "frameId", 1) - 1;
    loimi_str out;
    uint32_t child;
    MiniStatus st = mini_debug_eval(S, level, lstr(jstr(args, "expression", "")), &out, &child);
    drain(); /* what a call printed */
    if (st != MiniStatus_Ok) {
        char msg[1024];
        snprintf(msg, sizeof msg, "%.*s", (int)out.len, out.ptr);
        respond(req, 0, msg, NULL);
        return;
    }
    buf b = { 0 };
    respond_start(&b, req, 1, NULL);
    bput(&b, ",\"body\":{\"result\":", 18);
    blstr(&b, out);
    bf(&b, ",\"variablesReference\":%u}", child);
    send_msg(&b);
}

static void end_session(json *req) {
    drain();
    respond(req, 1, NULL, NULL);
    if (!terminated) event("terminated", NULL);
    fflush(stdout);
    _exit(0);
}

static MiniResume on_stop(MiniState *s, void *ud, MiniStop reason, uint32_t bp);

static int launch(json *req) {
    json *args = jget(req, "arguments");
    const char *path = jstr(args, "program", NULL);
    if (!path) { respond(req, 0, "launch: no `program`", NULL); return 0; }
    FILE *f = fopen(path, "rb");
    if (!f) { respond(req, 0, "launch: cannot read the program", NULL); return 0; }
    buf src = { 0 };
    char tmp[4096];
    size_t n;
    while ((n = fread(tmp, 1, sizeof tmp, f)) > 0) bput(&src, tmp, n);
    fclose(f);
    program = strdup(path);
    stop_on_entry = (int)jnum(args, "stopOnEntry", 0);
    MiniState *s = mini_new();
    mini_debug_attach(s, on_stop, NULL);
    if (mini_debug_load(s, lstr(program), (loimi_str){ src.p ? src.p : "", src.len }) != MiniStatus_Ok) {
        loimi_str e = mini_error(s);
        char msg[1024];
        snprintf(msg, sizeof msg, "%.*s", (int)e.len, e.ptr);
        respond(req, 0, msg, NULL);
        free(src.p);
        return 0;
    }
    free(src.p);
    S = s;
    respond(req, 1, NULL, NULL);
    event("initialized", NULL);
    return 1;
}

/* Handle one request; at a stop, a resume request returns its action. */
static int handle(json *req, int stopped, MiniResume *resume) {
    const char *cmd = jstr(req, "command", "");
    if (!strcmp(cmd, "initialize")) {
        respond(req, 1, NULL,
                "{\"supportsConfigurationDoneRequest\":true,\"supportsConditionalBreakpoints\":true,"
                "\"supportsHitConditionalBreakpoints\":true,\"supportsEvaluateForHovers\":true,"
                "\"supportsTerminateRequest\":true,"
                "\"exceptionBreakpointFilters\":[{\"filter\":\"error\",\"label\":\"Uncaught errors\",\"default\":true}]}");
    } else if (!strcmp(cmd, "launch")) {
        if (S) respond(req, 0, "already launched", NULL);
        else if (!launch(req)) { event("terminated", NULL); terminated = 1; }
    } else if (!strcmp(cmd, "disconnect") || !strcmp(cmd, "terminate")) {
        end_session(req);
    } else if (!S) {
        respond(req, 0, "not launched", NULL);
    } else if (!strcmp(cmd, "setBreakpoints")) {
        set_breakpoints(req);
    } else if (!strcmp(cmd, "setExceptionBreakpoints")) {
        json *f = jget(jget(req, "arguments"), "filters");
        break_on_error = 0;
        for (size_t i = 0; f && f->t == J_ARR && i < f->n; i++)
            if (f->items[i]->t == J_STR && !strcmp(f->items[i]->str, "error")) break_on_error = 1;
        mini_debug_break_on_error(S, break_on_error);
        respond(req, 1, NULL, NULL);
    } else if (!strcmp(cmd, "configurationDone")) {
        respond(req, 1, NULL, NULL);
        configured = 1;
    } else if (!strcmp(cmd, "pause")) {
        /* At a stop already: nothing to do. */
        if (stopped) STORE(user_pause, 0);
        respond(req, 1, NULL, NULL);
    } else if (!stopped) {
        respond(req, 0, "the program is not stopped", NULL);
    } else if (!strcmp(cmd, "stackTrace")) {
        stack_trace(req);
    } else if (!strcmp(cmd, "scopes")) {
        scopes(req);
    } else if (!strcmp(cmd, "variables")) {
        variables(req);
    } else if (!strcmp(cmd, "evaluate")) {
        evaluate(req);
    } else if (!strcmp(cmd, "continue")) {
        respond(req, 1, NULL, "{\"allThreadsContinued\":true}");
        *resume = MiniResume_Continue;
        return 1;
    } else if (!strcmp(cmd, "next")) {
        respond(req, 1, NULL, NULL);
        *resume = MiniResume_Next;
        return 1;
    } else if (!strcmp(cmd, "stepIn")) {
        respond(req, 1, NULL, NULL);
        *resume = MiniResume_Step;
        return 1;
    } else if (!strcmp(cmd, "stepOut")) {
        respond(req, 1, NULL, NULL);
        *resume = MiniResume_Finish;
        return 1;
    } else {
        respond(req, 0, "unsupported request", NULL);
    }
    return 0;
}

static MiniResume on_stop(MiniState *s, void *ud, MiniStop reason, uint32_t bp) {
    (void)s;
    (void)ud;
    STORE(running, 0);
    drain();
    MiniResume r = MiniResume_Keep;
    if (reason == MiniStop_Pause && !LOAD(user_pause)) {
        /* Stopped for the adapter: handle what is queued, go on as before. */
        json *m;
        while ((m = q_pop(0))) {
            int done = handle(m, 0, &r);
            jfree(m);
            if (done) break;
        }
        STORE(running, 1);
        return MiniResume_Keep;
    }
    STORE(user_pause, 0);
    static const char *names[] = { "entry", "breakpoint", "step", "pause", "exception" };
    buf b = { 0 };
    bf(&b, "{\"reason\":\"%s\",\"threadId\":1,\"allThreadsStopped\":true", names[reason]);
    loimi_str msg = mini_debug_message(s);
    if (msg.len) {
        if (reason == MiniStop_Error) bf(&b, ",\"description\":\"Uncaught error\"");
        bf(&b, ",\"text\":");
        blstr(&b, msg);
    }
    if (reason == MiniStop_Breakpoint) bf(&b, ",\"hitBreakpointIds\":[%u]", bp);
    bput(&b, "}", 1);
    event("stopped", b.p);
    free(b.p);
    for (;;) {
        json *m = q_pop(1);
        if (!m) _exit(0); /* the client went away */
        int done = handle(m, 1, &r);
        jfree(m);
        if (done) break;
    }
    STORE(running, 1);
    return r;
}

int main(void) {
    if (getenv("MINI_DAP_LOG")) trace = fopen(getenv("MINI_DAP_LOG"), "w");
    /* The client's channel is the original stdout; the script's stdout
     * (fd 1, which Mini's print writes through stdio) becomes a pipe. */
    dap_out = dup(1);
    if (dap_out < 0 || pipe(out_pipe) < 0) return 1;
    fcntl(out_pipe[0], F_SETFL, O_NONBLOCK);
    dup2(out_pipe[1], 1);
    close(out_pipe[1]);
    pthread_t rt, pt;
    pthread_create(&rt, NULL, reader, NULL);
    pthread_create(&pt, NULL, pump, NULL);
    MiniResume ignored;
    /* Until configurationDone: initialize, launch, breakpoints. */
    while (!configured) {
        json *m = q_pop(1);
        if (!m) return 0;
        handle(m, 0, &ignored);
        jfree(m);
    }
    mini_debug_break_on_error(S, break_on_error);
    if (stop_on_entry) mini_debug_stop_on_entry(S);
    STORE(running, 1);
    MiniStatus st = mini_call(S, 0);
    STORE(running, 0);
    drain();
    int code = 0;
    if (st != MiniStatus_Ok) {
        loimi_str e = mini_error(S);
        buf b = { 0 };
        bf(&b, "error: %.*s\n", (int)e.len, e.ptr);
        output("stderr", b.p, b.len);
        free(b.p);
        code = 1;
    }
    char body[64];
    snprintf(body, sizeof body, "{\"exitCode\":%d}", code);
    event("exited", body);
    event("terminated", NULL);
    terminated = 1;
    /* Until the client disconnects. */
    for (;;) {
        json *m = q_pop(1);
        if (!m) return code;
        handle(m, 0, &ignored);
        jfree(m);
    }
}
