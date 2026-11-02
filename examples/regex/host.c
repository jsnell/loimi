/* C host for the Loimi regex library (lib.lm).
 *
 *   loimi build examples/regex/lib.lm --release --lib -o librx.a --header rx.h
 *   clang-19 -O2 -I. examples/regex/host.c librx.a -lm -o rxhost
 *
 *   rxhost demo                 a few matches, an error, a disassembly
 *   rxhost test CASES           run test cases (see tests/gen.py), print results
 *   rxhost bench [pcre]         benchmarks (with -DHAVE_PCRE2 also PCRE2)
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "rx.h"

#ifdef HAVE_PCRE2
#define PCRE2_CODE_UNIT_WIDTH 8
#include <pcre2.h>
#endif

/* rx_exec flags (lib.lm cannot export constants to the header). */
#define RX_SEARCH 1u
#define RX_PIKE 2u

static void print_match(Regex *re, const char *s, uint32_t flags) {
    int64_t spans[2 * 10];
    int rc = rx_exec(re, (void *)s, strlen(s), 0, flags, spans, 10);
    if (rc < 0) {
        loimi_str e = rx_error(re);
        printf("  error: %.*s\n", (int)e.len, e.ptr);
        return;
    }
    if (rc == 0) {
        printf("  %-14s no match\n", s);
        return;
    }
    printf("  %-14s", s);
    uint32_t n = rx_groups(re) + 1;
    for (uint32_t g = 0; g < n && g < 10; g++) {
        if (spans[2 * g] < 0)
            printf(" [%u]=None", g);
        else
            printf(" [%u]='%.*s'", g, (int)(spans[2 * g + 1] - spans[2 * g]), s + spans[2 * g]);
    }
    printf("\n");
}

static int demo(void) {
    const char *pat = "(\\w+)@(\\w+)\\.com";
    Regex *re = rx_compile((void *)pat, strlen(pat));
    printf("pattern %s: %u groups\n", pat, rx_groups(re));
    print_match(re, "mail bob@example.com now", RX_SEARCH);
    print_match(re, "mail bob@example.com now", RX_SEARCH | RX_PIKE);
    print_match(re, "nobody here", RX_SEARCH);
    loimi_str d = rx_disasm(re, 0);
    printf("%.*s", (int)d.len, d.ptr);
    rx_free(re);

    const char *bad[] = { "a(b", "*a", "[z-a]", "a{3,2}", "\\q" };
    for (int i = 0; i < 5; i++) {
        re = rx_compile((void *)bad[i], strlen(bad[i]));
        loimi_str e = rx_error(re);
        printf("compile %-8s ok=%d error='%.*s' at %lld\n", bad[i], rx_ok(re), (int)e.len, e.ptr,
               (long long)rx_error_offset(re));
        rx_free(re);
    }
    return 0;
}

/* ---- test mode: lines of "HEXPATTERN HEXSUBJECT" ---- */

static size_t unhex(const char *h, char *out) {
    size_t n = 0;
    if (h[0] == '-' && h[1] == 0) return 0; /* empty string */
    for (; h[0] && h[1]; h += 2) {
        unsigned v;
        sscanf(h, "%2x", &v);
        out[n++] = (char)v;
    }
    return n;
}

static void put_result(Regex *re, const char *s, size_t n, uint32_t flags) {
    int64_t spans[2 * 100];
    if (!rx_ok(re)) {
        printf("ERR");
        return;
    }
    int rc = rx_exec(re, (void *)s, n, 0, flags, spans, 100);
    if (rc < 0) {
        printf("LIMIT");
        return;
    }
    if (rc == 0) {
        printf("NOMATCH");
        return;
    }
    uint32_t g = rx_groups(re) + 1;
    for (uint32_t i = 0; i < g && i < 100; i++)
        printf("%s%lld,%lld", i ? " " : "", (long long)spans[2 * i], (long long)spans[2 * i + 1]);
}

static int test(const char *path) {
    FILE *f = fopen(path, "r");
    if (!f) {
        perror(path);
        return 2;
    }
    static char line[1 << 16], hp[1 << 15], hs[1 << 15], pat[1 << 14], subj[1 << 14];
    while (fgets(line, sizeof line, f)) {
        char mode[8];
        if (sscanf(line, "%7s %32767s %32767s", mode, hp, hs) != 3) continue;
        size_t pn = unhex(hp, pat), sn = unhex(hs, subj);
        Regex *re = rx_compile(pat, pn);
        uint32_t fl = mode[0] == 's' ? RX_SEARCH : 0;
        put_result(re, subj, sn, fl);
        printf("\t");
        put_result(re, subj, sn, fl | RX_PIKE);
        printf("\n");
        rx_free(re);
    }
    fclose(f);
    return 0;
}

/* ---- benchmarks ---- */

static double now(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec * 1e-9;
}

/* The text generator; bench.py has the same one. */
static char *gen_text(size_t n) {
    static const char *words[] = { "the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog",
                                   "regex", "loimi", "bytecode", "machine", "Sherlock", "Holmes",
                                   "Watson", "1887", "42", "x9", "foo_bar", "end" };
    char *t = malloc(n + 32);
    size_t i = 0;
    uint64_t s = 12345;
    while (i < n) {
        s = s * 6364136223846793005ull + 1442695040888963407ull;
        uint32_t r = (uint32_t)(s >> 33);
        const char *w = words[r % 20];
        size_t wl = strlen(w);
        if (i + wl + 1 > n) break;
        memcpy(t + i, w, wl);
        i += wl;
        t[i++] = (r >> 8) % 13 == 0 ? '\n' : ' ';
    }
    while (i < n) t[i++] = ' ';
    t[n] = 0;
    return t;
}

typedef struct { const char *name, *pat; int text; } Bench;

static const Bench benches[] = {
    { "literal", "Sherlock Holmes", 0 },
    { "literal-alt", "Sherlock|Watson|Holmes|1887", 0 },
    { "class", "[a-z]+ing|\\d{3,}", 0 },
    { "words", "\\w+", 0 },
    { "capture", "(\\w+)\\s+(\\w+)", 0 },
    { "alt-heavy", "(?:fox|dog|cat|bird|lazy)\\s(?:over|jumps|end)", 0 },
    { "dot-star", "x9.*end$", 0 },
    { "pathological", "(a*)*b", 1 },
};

/* Count non-overlapping matches, as len(re.findall(...)) does. */
static long count_rx(Regex *re, const char *t, size_t n, uint32_t flags) {
    long c = 0;
    size_t at = 0;
    int64_t sp[2];
    while (at <= n) {
        int rc = rx_exec(re, (void *)t, n, at, flags | RX_SEARCH, sp, 1);
        if (rc < 0) return -1;
        if (rc == 0) break;
        c++;
        at = sp[1] > sp[0] ? (size_t)sp[1] : (size_t)sp[1] + 1;
    }
    return c;
}

#ifdef HAVE_PCRE2
static long count_pcre(pcre2_code *code, pcre2_match_data *md, const char *t, size_t n, int jit) {
    long c = 0;
    size_t at = 0;
    while (at <= n) {
        int rc = jit ? pcre2_jit_match(code, (PCRE2_SPTR)t, n, at, 0, md, NULL)
                     : pcre2_match(code, (PCRE2_SPTR)t, n, at, 0, md, NULL);
        if (rc < 0) break;
        PCRE2_SIZE *ov = pcre2_get_ovector_pointer(md);
        c++;
        at = ov[1] > ov[0] ? ov[1] : ov[1] + 1;
    }
    return c;
}
#endif

static int bench(int argc, char **argv) {
    size_t n = 4 << 20;
    int reps = 3;
    if (argc > 2) n = (size_t)atol(argv[2]);
    char *text = gen_text(n);
    /* The pathological case: (a*)*b against a^k, exponential in k for a
     * backtracker (Python too). */
    char path[32];
    memset(path, 'a', 24);
    path[24] = 0;
    printf("%-14s %10s %8s %10s %10s %10s %10s\n", "bench", "matches", "", "rx-bt", "rx-pike", "pcre2", "pcre2-jit");
    for (size_t b = 0; b < sizeof benches / sizeof benches[0]; b++) {
        const Bench *B = &benches[b];
        const char *t = B->text ? path : text;
        size_t tn = strlen(t);
        Regex *re = rx_compile((void *)B->pat, strlen(B->pat));
        double best[2] = { 1e9, 1e9 };
        long cnt[2] = { 0, 0 };
        for (int e = 0; e < 2; e++) {
            for (int r = 0; r < reps; r++) {
                double t0 = now();
                cnt[e] = count_rx(re, t, tn, e ? RX_PIKE : 0);
                double dt = now() - t0;
                if (dt < best[e]) best[e] = dt;
            }
        }
        double pbest[2] = { -1, -1 };
#ifdef HAVE_PCRE2
        int err;
        PCRE2_SIZE eo;
        pcre2_code *pc = pcre2_compile((PCRE2_SPTR)B->pat, PCRE2_ZERO_TERMINATED, 0, &err, &eo, NULL);
        pcre2_match_data *md = pcre2_match_data_create_from_pattern(pc, NULL);
        for (int jit = 0; jit < 2; jit++) {
            if (jit) pcre2_jit_compile(pc, PCRE2_JIT_COMPLETE);
            pbest[jit] = 1e9;
            for (int r = 0; r < reps; r++) {
                double t0 = now();
                long c = count_pcre(pc, md, t, tn, jit);
                double dt = now() - t0;
                if (dt < pbest[jit]) pbest[jit] = dt;
                if (c != cnt[0]) printf("  (pcre2 count %ld differs)\n", c);
            }
        }
        pcre2_match_data_free(md);
        pcre2_code_free(pc);
#endif
        printf("%-14s %10ld %8s %9.2fms %9.2fms %9.2fms %9.2fms%s\n", B->name, cnt[0], "", best[0] * 1e3,
               best[1] * 1e3, pbest[0] * 1e3, pbest[1] * 1e3, cnt[0] != cnt[1] ? "  COUNT MISMATCH" : "");
        rx_free(re);
    }
    free(text);
    return 0;
}

int main(int argc, char **argv) {
    if (argc >= 2 && !strcmp(argv[1], "demo")) return demo();
    if (argc >= 3 && !strcmp(argv[1], "test")) return test(argv[2]);
    if (argc >= 2 && !strcmp(argv[1], "bench")) return bench(argc, argv);
    fprintf(stderr, "usage: rxhost demo | test CASES | bench [textsize]\n");
    return 2;
}
