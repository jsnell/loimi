/* Run-time half of `loimi build --jit-stencils` (a copy-and-patch JIT
 * prototype; see docs/design/copy-and-patch.md). x86-64 Linux only.
 *
 * The compiler emits, per op, a stencil: machine code with holes for the
 * operand fields, the instruction word and the continuations (tables
 * `loimi_jit_*`, from src/elf.rs). The interpreter tells this runtime
 * about every verified code object (`loimi_jit_register`), counts calls,
 * returns and backward jumps per target (`loimi_jit_hotcount`), and calls
 * `loimi_jit_hot` when a counter runs out. Compiling a code object copies
 * the stencil of each instruction into executable memory, patches it, and
 * then replaces each instruction word that has compiled code with the
 * enter opcode, whose payload indexes `loimi_jit_entries`: the
 * interpreter's dispatch then lands in compiled code wherever it reaches
 * such an instruction. Compiled code exits to the interpreter by
 * dispatching through its table (ops without a stencil, cold paths,
 * calls into code that is not compiled). Rewrites of the code (`become`,
 * `Vm.optimize`) first restore the original words, and the code object
 * is compiled again once it is hot again.
 *
 * Environment: LOIMI_JIT=1 turns the JIT on; LOIMI_JIT_HOT=n sets the
 * hot-counter threshold; LOIMI_JIT_STATS=1 prints statistics at exit. */
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>

struct jit_reloc { uint32_t off; uint16_t type; uint16_t kind; int64_t addend; int64_t arg; };
struct jit_stencil { uint32_t code, len, reloc, nreloc; int32_t trim; uint32_t present; };
struct jit_opinfo { uint16_t size, base, flags, span, part; };

extern const uint8_t loimi_jit_code[];
extern const struct jit_reloc loimi_jit_relocs[];
extern const struct jit_stencil loimi_jit_stencils[];
extern void *const loimi_jit_syms[];
extern const uint8_t loimi_jit_rodata[];
extern const uint8_t loimi_jit_hcode[];
extern const int64_t loimi_jit_hcode_len;
extern const struct jit_reloc loimi_jit_hrelocs[];
extern const int32_t loimi_jit_nhrelocs;
extern const int64_t loimi_jit_rodata_len;
extern const int32_t loimi_jit_ntls;
extern void loimi_jit_tls(int64_t *out);
extern const struct jit_opinfo loimi_jit_opinfo[];
extern const uint32_t loimi_jit_nops, loimi_jit_enter_op, loimi_jit_op_bits;
extern const char *const loimi_jit_opnames[];
extern void **const loimi_jit_table;
extern void *const loimi_jit_ctable[];

#define HOT_SLOTS 4096
void **loimi_jit_entries;
uint16_t loimi_jit_hotcount[HOT_SLOTS];

enum { K_SYM, K_WORD, K_FIELD, K_NEXT, K_JUMP, K_TLS, K_RODATA, K_HELPER };
enum { R_64 = 1, R_PC32 = 2, R_PLT32 = 4, R_32 = 10, R_32S = 11, R_TPOFF32 = 23 };
enum { S_NONE, S_COMPILED, S_FAILED };
#define MAX_COMPILES 8

struct rec {
    uint32_t *code;
    uint64_t n;
    int state, compiles;
    uint32_t npatched;
    uint32_t *ppc, *orig;
};

static int enabled, stats, debug, inited;
static FILE *mapf;
static uint16_t threshold = 50;
static struct rec **recs;
static size_t nrecs, caprecs;
static size_t nentries, capentries;
static uint8_t *region, *region_end, *bump, *rodata, *helpers;
static int64_t *tlsoff;
static struct {
    uint64_t compiles, failures, invalidations, insns, exits, bytes, patched, ns, max_ns;
} st;

static uint64_t now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000u + ts.tv_nsec;
}

static void report(void) {
    if (!st.compiles && !st.failures) {
        fprintf(stderr, "jit: nothing compiled\n");
        return;
    }
    fprintf(stderr,
            "jit: %llu compiles (%llu failed, %llu invalidations), %llu instructions (%llu exit to the interpreter), "
            "%llu bytes, %llu entries; compile time %.3f ms total, %.1f us per function (max %.1f), %.0f ns per instruction\n",
            (unsigned long long)st.compiles, (unsigned long long)st.failures, (unsigned long long)st.invalidations,
            (unsigned long long)st.insns, (unsigned long long)st.exits, (unsigned long long)st.bytes,
            (unsigned long long)st.patched, st.ns / 1e6, st.compiles ? st.ns / 1e3 / st.compiles : 0.0, st.max_ns / 1e3,
            st.insns ? (double)st.ns / st.insns : 0.0);
}

__attribute__((constructor)) static void jit_init(void) {
    const char *e = getenv("LOIMI_JIT");
    enabled = e && *e && strcmp(e, "0") != 0;
    const char *h = getenv("LOIMI_JIT_HOT");
    if (h && atoi(h) > 0 && atoi(h) < 65535) threshold = (uint16_t)atoi(h);
    /* With the JIT on, the interpreter runs the counting copies of the
     * handlers that have them (calls, returns, label jumps, `become`).
     * With it off, the table stays the default build's. */
    if (enabled) {
        /* The table is read-only data, as in a default build (LLVM
         * treats its loads as invariant, and a writable table cost a few
         * percent with the JIT off): unprotect it for the update. */
        uintptr_t a = (uintptr_t)loimi_jit_table & ~(uintptr_t)4095;
        uintptr_t e = ((uintptr_t)(loimi_jit_table + loimi_jit_nops) + 4095) & ~(uintptr_t)4095;
        if (mprotect((void *)a, e - a, PROT_READ | PROT_WRITE) == 0) {
            for (uint32_t i = 0; i < loimi_jit_nops; i++)
                if (loimi_jit_ctable[i]) loimi_jit_table[i] = loimi_jit_ctable[i];
            mprotect((void *)a, e - a, PROT_READ);
        } else {
            enabled = 0;
        }
    }
    for (int i = 0; i < HOT_SLOTS; i++) loimi_jit_hotcount[i] = threshold;
    const char *s = getenv("LOIMI_JIT_STATS");
    stats = enabled && s && *s && strcmp(s, "0") != 0;
    if (stats) atexit(report);
    const char *d = getenv("LOIMI_JIT_DEBUG");
    debug = enabled && d && *d && strcmp(d, "0") != 0;
    /* LOIMI_JIT_MAP=file: "start end op" per compiled instruction, for
     * attributing profiler samples to stencils. */
    const char *m = getenv("LOIMI_JIT_MAP");
    if (enabled && m && *m) mapf = fopen(m, "w");
    inited = 1;
}

static int fits32s(int64_t v) { return v >= INT32_MIN && v <= INT32_MAX; }

/* Write relocation `rl` at P, with S the target's address or value. */
static int patch(uint8_t *P, const struct jit_reloc *rl, int64_t S) {
    int64_t v = S + rl->addend;
    switch (rl->type) {
    case R_64: memcpy(P, &v, 8); return 1;
    case R_PC32:
    case R_PLT32:
        v -= (int64_t)(intptr_t)P;
        if (!fits32s(v)) return 0;
        memcpy(P, &(int32_t){(int32_t)v}, 4);
        return 1;
    case R_32:
        if (v < 0 || v > (int64_t)UINT32_MAX) return 0;
        memcpy(P, &(uint32_t){(uint32_t)v}, 4);
        return 1;
    case R_32S:
    case R_TPOFF32:
        if (!fits32s(v)) return 0;
        memcpy(P, &(int32_t){(int32_t)v}, 4);
        return 1;
    }
    return 0;
}

/* Executable memory near the program: stencils reach its functions and
 * data with 32-bit displacements (x86-64 small code model). */
static int init_region(void) {
    if (region) return 1;
    size_t size = 256u << 20;
    void *p = mmap((void *)0x40000000, size, PROT_READ | PROT_EXEC, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
    if (p == MAP_FAILED) p = mmap((void *)0x40000000, size, PROT_READ | PROT_EXEC, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) {
        enabled = 0;
        return 0;
    }
    region = p;
    region_end = region + size;
    bump = region;
    /* Read-only data the stencils use (float constants and the like). */
    size_t ro = (size_t)loimi_jit_rodata_len;
    if (ro) {
        mprotect(region, (ro + 4095) & ~(size_t)4095, PROT_READ | PROT_WRITE);
        memcpy(region, loimi_jit_rodata, ro);
        mprotect(region, (ro + 4095) & ~(size_t)4095, PROT_READ);
        bump = region + ((ro + 4095) & ~(size_t)4095);
    }
    rodata = region;
    tlsoff = calloc(loimi_jit_ntls + 1, sizeof *tlsoff);
    loimi_jit_tls(tlsoff);
    /* The helpers: functions of the interpreter that stencils call, which
     * have their own copy (so the interpreter's are compiled as in a
     * default build). Copied and relocated once. */
    size_t hl = ((size_t)loimi_jit_hcode_len + 4095) & ~(size_t)4095;
    helpers = bump;
    if (hl) {
        mprotect(helpers, hl, PROT_READ | PROT_WRITE);
        memcpy(helpers, loimi_jit_hcode, (size_t)loimi_jit_hcode_len);
        for (int32_t k = 0; k < loimi_jit_nhrelocs; k++) {
            const struct jit_reloc *rl = &loimi_jit_hrelocs[k];
            int64_t S;
            switch (rl->kind) {
            case K_SYM: S = (int64_t)(intptr_t)loimi_jit_syms[rl->arg]; break;
            case K_TLS: S = tlsoff[rl->arg]; break;
            case K_RODATA: S = (int64_t)(intptr_t)(rodata + rl->arg); break;
            case K_HELPER: S = (int64_t)(intptr_t)(helpers + rl->arg); break;
            default: S = 0; break;
            }
            if (!patch(helpers + rl->off, rl, S)) {
                /* Cannot happen near the program; give up on the JIT. */
                munmap(region, size);
                region = NULL;
                enabled = 0;
                return 0;
            }
        }
        mprotect(helpers, hl, PROT_READ | PROT_EXEC);
        bump += hl;
    }
    return 1;
}

static struct rec *lookup(const uint32_t *pc) {
    size_t lo = 0, hi = nrecs;
    while (lo < hi) {
        size_t mid = (lo + hi) / 2;
        if (recs[mid]->code <= pc) lo = mid + 1;
        else hi = mid;
    }
    if (lo == 0) return NULL;
    struct rec *r = recs[lo - 1];
    return pc < r->code + r->n ? r : NULL;
}

static void free_rec(struct rec *r) {
    free(r->ppc);
    free(r->orig);
    free(r);
}

/* A new verified code object. Records overlapping it are of objects the
 * collector freed (their memory is being reused), so they go. */
void loimi_jit_register(uint32_t *code, uint64_t n) {
    if (!enabled || n == 0) return;
    size_t lo = 0, hi = nrecs;
    while (lo < hi) {
        size_t mid = (lo + hi) / 2;
        if (recs[mid]->code < code) lo = mid + 1;
        else hi = mid;
    }
    size_t a = lo, b = lo;
    while (a > 0 && recs[a - 1]->code + recs[a - 1]->n > code) a--;
    while (b < nrecs && recs[b]->code < code + n) b++;
    for (size_t i = a; i < b; i++) free_rec(recs[i]);
    memmove(recs + a, recs + b, (nrecs - b) * sizeof *recs);
    nrecs -= b - a;
    if (nrecs == caprecs) {
        caprecs = caprecs ? caprecs * 2 : 256;
        recs = realloc(recs, caprecs * sizeof *recs);
    }
    memmove(recs + a + 1, recs + a, (nrecs - a) * sizeof *recs);
    struct rec *r = calloc(1, sizeof *r);
    r->code = code;
    r->n = n;
    recs[a] = r;
    nrecs++;
}

/* Put the original words back; the compiled code is left unreachable. */
static void unpatch(struct rec *r) {
    if (debug) fprintf(stderr, "jit: invalidated %p\n", (void *)r->code);
    for (uint32_t i = 0; i < r->npatched; i++) r->code[r->ppc[i]] = r->orig[i];
    r->npatched = 0;
    r->state = r->compiles >= MAX_COMPILES ? S_FAILED : S_NONE;
    st.invalidations++;
}

void loimi_jit_become(uint32_t *pc) {
    struct rec *r = nrecs ? lookup(pc) : NULL;
    if (r && r->state == S_COMPILED) unpatch(r);
}

void loimi_jit_invalidate(uint32_t *code) { loimi_jit_become(code); }

static int64_t field(uint32_t w, int64_t arg) {
    unsigned shift = (arg >> 8) & 255, bits = (arg >> 16) & 255, sgn = (arg >> 24) & 1;
    uint64_t u = (w >> shift) & ((bits >= 32) ? 0xffffffffu : ((1u << bits) - 1));
    if (!sgn) return (int64_t)u;
    return (int64_t)(u ^ (1ull << (bits - 1))) - (int64_t)(1ull << (bits - 1));
}

static uint32_t new_entry(void *native) {
    if (nentries == capentries) {
        capentries = capentries ? capentries * 2 : 4096;
        loimi_jit_entries = realloc(loimi_jit_entries, capentries * sizeof(void *));
    }
    loimi_jit_entries[nentries] = native;
    return (uint32_t)nentries++;
}

static int compile(struct rec *r) {
    uint64_t t0 = now_ns();
    const uint32_t opmask = (1u << loimi_jit_op_bits) - 1, nops = loimi_jit_nops;
    uint64_t n = r->n;
    uint32_t *code = r->code;
    int64_t *map = malloc((n + 1) * sizeof *map);
    uint8_t *nopatch = calloc(n + 1, 1);
    uint32_t *sts = malloc(n * sizeof *sts);
    uint32_t *lens = malloc(n * sizeof *lens);
    int ok = 0;
    uint64_t ninsn = 0, nexit = 0;
    for (uint64_t i = 0; i <= n; i++) map[i] = -1;
    /* Layout: one stencil per instruction (a super's first part at its
     * head word; the other parts are instructions of their own). */
    int64_t off = 0;
    for (uint64_t pc = 0; pc < n;) {
        uint32_t op = code[pc] & opmask;
        const struct jit_opinfo *oi = &loimi_jit_opinfo[op];
        if (oi->base == 0xffff || oi->size == 0 || pc + oi->size > n) goto out;
        uint32_t s = loimi_jit_stencils[oi->base].present ? oi->base
                     : (oi->part != 0xffff && loimi_jit_stencils[oi->part].present) ? oi->part : nops;
        if (s == nops) nexit++;
        if ((oi->flags & 1) && pc + oi->size < n) nopatch[pc + oi->size] = 1;
        if (oi->flags & 2)
            for (uint64_t k = oi->size; k < oi->span && pc + k < n; k++) nopatch[pc + k] = 1;
        const struct jit_stencil *sp = &loimi_jit_stencils[s];
        uint32_t len = sp->len;
        if (sp->trim >= 0 && (uint64_t)sp->trim == oi->size && pc + oi->size < n) len -= 5;
        map[pc] = off;
        sts[pc] = s;
        lens[pc] = len;
        off += len;
        pc += oi->size;
        ninsn++;
    }
    off = (off + 15) & ~(int64_t)15;
    if (bump + off > region_end) goto out;
    uint8_t *dst = bump;
    uintptr_t pa = (uintptr_t)dst & ~(uintptr_t)4095, pe = ((uintptr_t)dst + off + 4095) & ~(uintptr_t)4095;
    if (mprotect((void *)pa, pe - pa, PROT_READ | PROT_WRITE)) goto out;
    for (uint64_t pc = 0; pc < n; pc++) {
        if (map[pc] < 0) continue;
        const struct jit_stencil *sp = &loimi_jit_stencils[sts[pc]];
        uint8_t *at = dst + map[pc];
        memcpy(at, loimi_jit_code + sp->code, lens[pc]);
        for (uint32_t k = 0; k < sp->nreloc; k++) {
            const struct jit_reloc *rl = &loimi_jit_relocs[sp->reloc + k];
            if (rl->off + 4 > lens[pc]) continue; /* the trimmed jump */
            uint8_t *P = at + rl->off;
            int64_t S, t;
            switch (rl->kind) {
            case K_SYM: S = (int64_t)(intptr_t)loimi_jit_syms[rl->arg]; break;
            case K_WORD: S = code[pc + rl->arg]; break;
            case K_FIELD: {
                int64_t f = field(code[pc + (rl->arg & 255)], rl->arg);
                unsigned bits = (rl->arg >> 16) & 255;
                S = ((rl->arg >> 24) & 1) ? f + (1ll << (bits - 1)) : f * 8;
                break;
            }
            case K_NEXT:
                t = (int64_t)pc + rl->arg;
                if (t < 0 || (uint64_t)t >= n || map[t] < 0) goto fail;
                S = (int64_t)(intptr_t)(dst + map[t]);
                break;
            case K_JUMP:
                t = (int64_t)pc + (rl->arg & 255) + field(code[pc + (rl->arg & 255)], rl->arg);
                if (t < 0 || (uint64_t)t >= n || map[t] < 0) goto fail;
                S = (int64_t)(intptr_t)(dst + map[t]);
                break;
            case K_TLS: S = tlsoff[rl->arg]; break;
            case K_RODATA: S = (int64_t)(intptr_t)(rodata + rl->arg); break;
            case K_HELPER: S = (int64_t)(intptr_t)(helpers + rl->arg); break;
            default: goto fail;
            }
            if (!patch(P, rl, S)) goto fail;
        }
    }
    mprotect((void *)pa, pe - pa, PROT_READ | PROT_EXEC);
    bump += off;
    /* Enter compiled code wherever the interpreter dispatches to an
     * instruction that has a stencil, except words other instructions
     * read (a `then` operand word, a super's later parts). */
    r->ppc = realloc(r->ppc, ninsn * sizeof *r->ppc);
    r->orig = realloc(r->orig, ninsn * sizeof *r->orig);
    r->npatched = 0;
    for (uint64_t pc = 0; pc < n; pc++) {
        if (map[pc] < 0 || sts[pc] == nops || nopatch[pc]) continue;
        if (nentries >= (1ull << (32 - loimi_jit_op_bits))) break;
        uint32_t e = new_entry(dst + map[pc]);
        r->ppc[r->npatched] = (uint32_t)pc;
        r->orig[r->npatched++] = code[pc];
        code[pc] = loimi_jit_enter_op | e << loimi_jit_op_bits;
    }
    if (mapf) {
        for (uint64_t pc = 0; pc < n; pc++)
            if (map[pc] >= 0)
                fprintf(mapf, "%lx %lx %s\n", (unsigned long)(uintptr_t)(dst + map[pc]),
                        (unsigned long)(uintptr_t)(dst + map[pc] + lens[pc]), sts[pc] == nops ? "exit" : loimi_jit_opnames[sts[pc]]);
        fflush(mapf);
    }
    if (debug) {
        fprintf(stderr, "jit: compiled %p (%llu words) at %p\n", (void *)code, (unsigned long long)n, (void *)dst);
        for (uint64_t pc = 0, k = 0; pc < n; pc++) {
            if (map[pc] < 0) continue;
            while (k < r->npatched && r->ppc[k] < pc) k++;
            int patched = k < r->npatched && r->ppc[k] == pc;
            uint32_t w = patched ? r->orig[k] : code[pc];
            fprintf(stderr, "  %4llu %08x %-12s %s +%lld%s\n", (unsigned long long)pc, w, loimi_jit_opnames[w & opmask],
                    sts[pc] == nops ? "exit" : loimi_jit_opnames[sts[pc]], (long long)map[pc], patched ? " entry" : "");
        }
    }
    st.patched += r->npatched;
    st.bytes += off;
    st.insns += ninsn;
    st.exits += nexit;
    ok = 1;
    goto out;
fail:
    mprotect((void *)pa, pe - pa, PROT_READ | PROT_EXEC);
out:
    free(map);
    free(nopatch);
    free(sts);
    free(lens);
    uint64_t dt = now_ns() - t0;
    if (ok) {
        st.compiles++;
        st.ns += dt;
        if (dt > st.max_ns) st.max_ns = dt;
    } else {
        st.failures++;
    }
    return ok;
}

void loimi_jit_hot(uint32_t *pc) {
    loimi_jit_hotcount[((uintptr_t)pc >> 2) & (HOT_SLOTS - 1)] = threshold;
    if (!enabled || !init_region()) return;
    struct rec *r = lookup(pc);
    if (!r || r->state != S_NONE) {
        /* Not ours, or given up on: count longer before asking again. */
        loimi_jit_hotcount[((uintptr_t)pc >> 2) & (HOT_SLOTS - 1)] = 65535;
        return;
    }
    r->compiles++;
    r->state = compile(r) ? S_COMPILED : S_FAILED;
}
