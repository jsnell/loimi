/* Run-time half of `loimi build --jit-stencils` (a copy-and-patch JIT
 * prototype; see docs/design/copy-and-patch.md). x86-64 Linux only.
 *
 * The compiler emits, per op, a stencil: machine code with holes for the
 * operand fields, the instruction word and the continuations (tables
 * `loimi_jit_*`, from src/elf.rs). The interpreter tells this runtime
 * about every verified code object (`loimi_jit_register`). With the JIT
 * on, the dispatch table is pointed at the counting copies of the
 * handlers that jump (`loimi_jit_ctable`), which count calls and
 * backward jumps per target (`loimi_jit_hotcount`) and call
 * `loimi_jit_hot` when a counter runs out; with it off, the interpreter
 * is the default build's. Compiling a code object copies
 * the stencil of each instruction into executable memory, patches it, and
 * then replaces each instruction word that has compiled code with the
 * enter opcode, whose payload indexes `loimi_jit_entries`: the
 * interpreter's dispatch then lands in compiled code wherever it reaches
 * such an instruction. Compiled code exits to the interpreter by
 * dispatching through its table (ops without a stencil, cold paths,
 * calls into code that is not compiled). Rewrites of compiled code
 * (`become`, `Vm.optimize`, `Vm.deopt`) go through this runtime, which
 * compiles the code object again at once or points the rewritten
 * instructions at other code (`rewrite`); still running old code follows
 * through 5-byte prefixes in front of rewritable instructions.
 *
 * Environment: LOIMI_JIT=1 turns the JIT on; LOIMI_JIT_HOT=n sets the
 * hot-counter threshold; LOIMI_JIT_STATS=1 prints statistics at exit;
 * LOIMI_JIT_RECOMPILE=n stops recompiles after n compiles (testing). */
#define _GNU_SOURCE
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
#include <time.h>

struct jit_reloc { uint32_t off; uint16_t type; uint16_t kind; int64_t addend; int64_t arg; };
/* A stencil: its hot part (`hot` bytes, the fast path), then its cold
 * part (blocks LLVM thinks are rarely run, `-fsplit-machine-functions`),
 * which the JIT places after all the hot parts of a code object. */
struct jit_stencil { uint32_t code, len, reloc, nreloc; int32_t trim; uint32_t present, hot; };
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
extern const struct jit_reloc loimi_jit_drelocs[];
extern const int32_t loimi_jit_ndrelocs;
extern const int64_t loimi_jit_rodata_len;
extern const int32_t loimi_jit_ntls;
extern void loimi_jit_tls(int64_t *out);
extern const struct jit_opinfo loimi_jit_opinfo[];
extern const uint32_t loimi_jit_nops, loimi_jit_enter_op, loimi_jit_op_bits;
extern const char *const loimi_jit_opnames[];
extern void **const loimi_jit_table;
extern void *const loimi_jit_ctable[];
/* Stencil variants of ops with a const operand, per tag of the constant:
 * used when the constant's bits satisfy (bits & mask) == value (or
 * bits < value if ult). */
struct jit_variant { uint32_t op, stencil; int64_t field; uint64_t mask, value; uint32_t ult; };
extern const struct jit_variant loimi_jit_variants[];
extern const int32_t loimi_jit_nvariants;

#define HOT_SLOTS 4096
void **loimi_jit_entries;
uint16_t loimi_jit_hotcount[HOT_SLOTS];

enum { K_SYM, K_WORD, K_FIELD, K_NEXT, K_JUMP, K_TLS, K_RODATA, K_HELPER, K_SELF, K_CONST };
enum { R_64 = 1, R_PC32 = 2, R_PLT32 = 4, R_32 = 10, R_32S = 11, R_TPOFF32 = 23 };
enum { S_NONE, S_COMPILED, S_FAILED };
#define MAX_COMPILES 32

/* Per word of a compiled code object (fl): an instruction start has a
 * home (where jumps to it go); F_PREFIX: the home is a 5-byte prefix in
 * front of the code, which `retarget` can make a jump; F_ENTRY: the word
 * is an enter word (w holds the real word); F_NOPATCH: another
 * instruction reads the word, so it must keep its opcode; F_INTERP: the
 * word went back and forth too often, and the instruction is left to the
 * interpreter (its home jumps to the exit stencil). */
enum { F_PREFIX = 1, F_ENTRY = 2, F_NOPATCH = 4, F_INTERP = 8 };
/* Retargets of one instruction before it is left to the interpreter. */
#define MAX_RETARGETS 16
/* Code made after the compile for instruction i holding word w. */
struct alt {
    uint64_t i;
    uint32_t w;
    uint8_t *at;
};

struct rec {
    uint32_t *code;
    uint64_t n;
    /* The constant pool of `const` operands (patched into the code), or
     * NULL. Verified code's pool does not change. */
    const uint64_t *pool;
    int state, compiles, rewriting;
    /* While compiled, per word: */
    uint8_t **home, **body; /* the instruction's home, and the code it runs now */
    uint32_t *w, *cw;       /* its real word, and the word compiled at first */
    int32_t *ent;           /* its entry index, or -1 */
    uint8_t *fl, *nre; /* flags; retargets so far */
    struct alt *alts;
    uint32_t nalts, capalts;
    /* Per word, kept across compiles: another instruction read it while
     * compiled (F_NOPATCH once). It never becomes an enter word again: a
     * super that splits itself (`become` in a part, in the interpreter)
     * still decodes its later parts from their words after the split. */
    uint8_t *read;
};

static int enabled, stats, debug, inited, max_recompile = MAX_COMPILES;
static FILE *mapf;
static uint16_t threshold = 50;
static struct rec **recs;
static size_t nrecs, caprecs;
static size_t nentries, capentries;
static ptrdiff_t wdelta;
#define WR(p) ((p) + wdelta)
static uint8_t *region, *region_end, *bump, *rodata, *helpers, *exit_code;
static int64_t *tlsoff;
static struct {
    uint64_t compiles, failures, invalidations, given_up, insns, exits, bytes, patched, ns, max_ns;
    uint64_t retargets, copies, reused, fallbacks, recompiles, interp;
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
            "jit: %llu compiles (%llu failed, %llu invalidations, %llu given up), %llu instructions (%llu exit to the interpreter), "
            "%llu bytes, %llu entries; compile time %.3f ms total, %.1f us per function (max %.1f), %.0f ns per instruction\n",
            (unsigned long long)st.compiles, (unsigned long long)st.failures, (unsigned long long)st.invalidations,
            (unsigned long long)st.given_up,
            (unsigned long long)st.insns, (unsigned long long)st.exits, (unsigned long long)st.bytes,
            (unsigned long long)st.patched, st.ns / 1e6, st.compiles ? st.ns / 1e3 / st.compiles : 0.0, st.max_ns / 1e3,
            st.insns ? (double)st.ns / st.insns : 0.0);
    fprintf(stderr, "jit: rewrites of compiled code: %llu recompiles, %llu retargets (%llu to existing code, %llu new copies, %llu left to the interpreter), %llu dropped the code\n",
            (unsigned long long)st.recompiles, (unsigned long long)st.retargets, (unsigned long long)st.reused,
            (unsigned long long)st.copies, (unsigned long long)st.interp, (unsigned long long)st.fallbacks);
}

__attribute__((constructor)) static void jit_init(void) {
    const char *e = getenv("LOIMI_JIT");
    enabled = e && *e && strcmp(e, "0") != 0;
    const char *h = getenv("LOIMI_JIT_HOT");
    if (h && atoi(h) > 0 && atoi(h) < 65535) threshold = (uint16_t)atoi(h);
    /* LOIMI_JIT_RECOMPILE=n: compiles of a code object after which a
     * rewrite no longer recompiles it (for testing the other paths). */
    const char *rc = getenv("LOIMI_JIT_RECOMPILE");
    if (rc && *rc) max_recompile = atoi(rc);
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
    case R_64: memcpy(WR(P), &v, 8); return 1;
    case R_PC32:
    case R_PLT32:
        v -= (int64_t)(intptr_t)P;
        if (!fits32s(v)) return 0;
        memcpy(WR(P), &(int32_t){(int32_t)v}, 4);
        return 1;
    case R_32:
        if (v < 0 || v > (int64_t)UINT32_MAX) return 0;
        memcpy(WR(P), &(uint32_t){(uint32_t)v}, 4);
        return 1;
    case R_32S:
    case R_TPOFF32:
        if (!fits32s(v)) return 0;
        memcpy(WR(P), &(int32_t){(int32_t)v}, 4);
        return 1;
    }
    return 0;
}

/* Executable memory near the program: stencils reach its functions and
 * data with 32-bit displacements (x86-64 small code model). The same
 * memory is mapped twice: executable (and not writable) where the code
 * runs, and writable (not executable) elsewhere, where the runtime writes
 * it (WR). Re-patching an instruction (`retarget`) then needs no system
 * call. */
static int init_region(void) {
    if (region) return 1;
    size_t size = 256u << 20;
    int fd = memfd_create("loimi-jit", MFD_CLOEXEC);
    if (fd < 0 || ftruncate(fd, (off_t)size) != 0) {
        if (fd >= 0) close(fd);
        enabled = 0;
        return 0;
    }
    void *p = mmap((void *)0x40000000, size, PROT_READ | PROT_EXEC, MAP_SHARED | MAP_FIXED_NOREPLACE, fd, 0);
    if (p == MAP_FAILED) p = mmap((void *)0x40000000, size, PROT_READ | PROT_EXEC, MAP_SHARED, fd, 0);
    void *wp = p == MAP_FAILED ? MAP_FAILED : mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    close(fd);
    if (wp == MAP_FAILED) {
        if (p != MAP_FAILED) munmap(p, size);
        enabled = 0;
        return 0;
    }
    wdelta = (uint8_t *)wp - (uint8_t *)p;
    region = p;
    region_end = region + size;
    bump = region;
    /* Read-only data the stencils use (float constants and the like). */
    size_t ro = (size_t)loimi_jit_rodata_len;
    if (ro) {
        memcpy(WR(region), loimi_jit_rodata, ro);
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
        memcpy(WR(helpers), loimi_jit_hcode, (size_t)loimi_jit_hcode_len);
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
                munmap(WR(region), size);
                region = NULL;
                enabled = 0;
                return 0;
            }
        }
        bump += hl;
    }
    /* The helpers' jump tables (in the read-only data), then protect it. */
    for (int32_t k = 0; k < loimi_jit_ndrelocs; k++) {
        const struct jit_reloc *rl = &loimi_jit_drelocs[k];
        int64_t S = 0;
        switch (rl->kind) {
        case K_SYM: S = (int64_t)(intptr_t)loimi_jit_syms[rl->arg]; break;
        case K_RODATA: S = (int64_t)(intptr_t)(rodata + rl->arg); break;
        case K_HELPER: S = (int64_t)(intptr_t)(helpers + rl->arg); break;
        }
        patch(rodata + rl->off, rl, S);
    }
    /* One copy of the exit stencil (dispatch *pc in the interpreter), for
     * continuations that do not land on an instruction. */
    const struct jit_stencil *xs = &loimi_jit_stencils[loimi_jit_nops];
    exit_code = bump;
    memcpy(WR(exit_code), loimi_jit_code + xs->code, xs->len);
    for (uint32_t k = 0; k < xs->nreloc; k++) {
        const struct jit_reloc *rl = &loimi_jit_relocs[xs->reloc + k];
        int64_t S = 0;
        switch (rl->kind) {
        case K_SYM: S = (int64_t)(intptr_t)loimi_jit_syms[rl->arg]; break;
        case K_TLS: S = tlsoff[rl->arg]; break;
        case K_RODATA: S = (int64_t)(intptr_t)(rodata + rl->arg); break;
        case K_HELPER: S = (int64_t)(intptr_t)(helpers + rl->arg); break;
        }
        patch(exit_code + rl->off, rl, S);
    }
    bump += 4096;
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

static void free_compiled(struct rec *r) {
    free(r->home);
    free(r->body);
    free(r->w);
    free(r->cw);
    free(r->ent);
    free(r->fl);
    free(r->nre);
    free(r->alts);
    r->home = r->body = NULL;
    r->w = r->cw = NULL;
    r->ent = NULL;
    r->fl = r->nre = NULL;
    r->alts = NULL;
    r->nalts = r->capalts = 0;
}

static void free_rec(struct rec *r) {
    free_compiled(r);
    free(r->read);
    free(r);
}

/* A new verified code object. Records overlapping it are of objects the
 * collector freed (their memory is being reused), so they go. */
void loimi_jit_register(uint32_t *code, uint64_t n, const void *pool) {
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
    r->pool = pool;
    recs[a] = r;
    nrecs++;
}

#define OPMASK ((1u << loimi_jit_op_bits) - 1)

/* The stencil for an instruction word: its op's, else (a super whose
 * stencil is missing) its first part's, else the exit stencil. */
static uint32_t stencil_for(uint32_t w) {
    const struct jit_opinfo *oi = &loimi_jit_opinfo[w & OPMASK];
    if (oi->base != 0xffff && loimi_jit_stencils[oi->base].present) return oi->base;
    if (oi->part != 0xffff && loimi_jit_stencils[oi->part].present) return oi->part;
    return loimi_jit_nops;
}

/* The stencil for word w of code object r: a variant of stencil_for(w)
 * for the tag of its constant, if there is one. */
static int64_t field(uint32_t w, int64_t arg);
static uint32_t stencil_in(const struct rec *r, uint32_t w) {
    uint32_t s = stencil_for(w);
    if (!r->pool || s != (w & OPMASK)) return s;
    for (int32_t j = 0; j < loimi_jit_nvariants; j++) {
        const struct jit_variant *v = &loimi_jit_variants[j];
        if (v->op != s || !loimi_jit_stencils[v->stencil].present) continue;
        uint64_t c = r->pool[field(w, v->field)];
        if (v->ult ? c < v->value : (c & v->mask) == v->value) return v->stencil;
    }
    return s;
}

/* The real word at index j: enter words stand for the word they replaced. */
static uint32_t word_at(const struct rec *r, uint64_t j) { return (r->fl[j] & F_ENTRY) ? r->w[j] : r->code[j]; }

static int64_t field(uint32_t w, int64_t arg) {
    unsigned shift = (arg >> 8) & 255, bits = (arg >> 16) & 255, sgn = (arg >> 24) & 1;
    uint64_t u = (w >> shift) & ((bits >= 32) ? 0xffffffffu : ((1u << bits) - 1));
    if (!sgn) return (int64_t)u;
    return (int64_t)(u ^ (1ull << (bits - 1))) - (int64_t)(1ull << (bits - 1));
}

/* Copy stencil s for the instruction at pc, its hot part (len bytes of
 * it: the final jump may be trimmed) to `at` and its cold part to `cold`,
 * and patch it. Continuations go to the instructions' homes (r->home). */
static int emit_copy(const struct rec *r, uint64_t pc, uint32_t w0, uint32_t s, uint32_t len, uint8_t *at, uint8_t *cold) {
    const struct jit_stencil *sp = &loimi_jit_stencils[s];
    uint64_t n = r->n;
    memcpy(WR(at), loimi_jit_code + sp->code, len);
    memcpy(WR(cold), loimi_jit_code + sp->code + sp->hot, sp->len - sp->hot);
    for (uint32_t k = 0; k < sp->nreloc; k++) {
        const struct jit_reloc *rl = &loimi_jit_relocs[sp->reloc + k];
        if (rl->off < sp->hot && rl->off + 4 > len) continue; /* the trimmed jump */
        uint8_t *P = rl->off < sp->hot ? at + rl->off : cold + (rl->off - sp->hot);
        int64_t S, t;
        switch (rl->kind) {
        case K_SYM: S = (int64_t)(intptr_t)loimi_jit_syms[rl->arg]; break;
        case K_WORD: S = rl->arg == 0 ? w0 : word_at(r, pc + rl->arg); break;
        case K_FIELD: {
            unsigned o = rl->arg & 255;
            int64_t f = field(o == 0 ? w0 : word_at(r, pc + o), rl->arg);
            unsigned bits = (rl->arg >> 16) & 255;
            S = ((rl->arg >> 24) & 1) ? f + (1ll << (bits - 1)) : f * 8;
            break;
        }
        case K_NEXT:
            t = (int64_t)pc + rl->arg;
            /* Not an instruction: the jump is never taken (verified
             * code jumps to instructions only), but must go somewhere. */
            S = (int64_t)(intptr_t)(t < 0 || (uint64_t)t >= n || !r->home[t] ? exit_code : r->home[t]);
            break;
        case K_JUMP: {
            unsigned o = rl->arg & 255;
            t = (int64_t)pc + o + field(o == 0 ? w0 : word_at(r, pc + o), rl->arg);
            /* A signed operand that is not a label (the stencil
             * compares the jump's target with pc + it): the same. */
            S = (int64_t)(intptr_t)(t < 0 || (uint64_t)t >= n || !r->home[t] ? exit_code : r->home[t]);
            break;
        }
        case K_CONST: {
            unsigned o = rl->arg & 255;
            if (!r->pool) return 0;
            S = (int64_t)r->pool[field(o == 0 ? w0 : word_at(r, pc + o), rl->arg)];
            break;
        }
        case K_TLS: S = tlsoff[rl->arg]; break;
        case K_RODATA: S = (int64_t)(intptr_t)(rodata + rl->arg); break;
        case K_HELPER: S = (int64_t)(intptr_t)(helpers + rl->arg); break;
        case K_SELF: S = (int64_t)(intptr_t)(rl->arg < sp->hot ? at + rl->arg : cold + (rl->arg - sp->hot)); break;
        default: return 0;
        }
        if (!patch(P, rl, S)) return 0;
    }
    return 1;
}

/* A stencil's name: its op's (variants: with a `*`). */
static const char *stencil_name(uint32_t s, int *variant) {
    *variant = s > loimi_jit_nops;
    if (s == loimi_jit_nops) return "exit";
    return loimi_jit_opnames[s < loimi_jit_nops ? s : loimi_jit_variants[s - loimi_jit_nops - 1].op];
}

static void map_line(uint8_t *a, uint32_t len, uint32_t s) {
    int v;
    const char *nm = stencil_name(s, &v);
    fprintf(mapf, "%lx %lx %s%s\n", (unsigned long)(uintptr_t)a, (unsigned long)(uintptr_t)(a + len), nm, v ? "*" : "");
}

static uint32_t new_entry(void *native) {
    if (nentries == capentries) {
        capentries = capentries ? capentries * 2 : 4096;
        loimi_jit_entries = realloc(loimi_jit_entries, capentries * sizeof(void *));
    }
    loimi_jit_entries[nentries] = native;
    return (uint32_t)nentries++;
}

/* Make the words that have compiled code enter words, and the others real
 * words: the interpreter enters compiled code wherever it dispatches to
 * an instruction that has a stencil, except at words other instructions
 * read (a `then` operand word, a super's later parts, whose head may
 * still run in the interpreter, or was one: see rec.read). Called after
 * every change of words. */
static void sync_entries(struct rec *r) {
    uint64_t n = r->n;
    if (!r->read) r->read = calloc(n, 1);
    for (uint64_t pc = 0; pc < n; pc++) r->fl[pc] &= ~F_NOPATCH;
    for (uint64_t pc = 0; pc < n; pc++) {
        if (!r->home[pc]) continue;
        const struct jit_opinfo *oi = &loimi_jit_opinfo[word_at(r, pc) & OPMASK];
        if ((oi->flags & 1) && pc + oi->size < n) r->read[pc + oi->size] = 1;
        if (oi->flags & 2)
            for (uint64_t k = oi->size; k < oi->span && pc + k < n; k++) r->read[pc + k] = 1;
    }
    for (uint64_t pc = 0; pc < n; pc++)
        if (r->read && r->read[pc]) r->fl[pc] |= F_NOPATCH;
    for (uint64_t pc = 0; pc < n; pc++) {
        if (!r->home[pc]) continue;
        uint32_t w = word_at(r, pc);
        int want = !(r->fl[pc] & (F_NOPATCH | F_INTERP)) && stencil_for(w) != loimi_jit_nops;
        if (want && r->ent[pc] < 0) {
            if (nentries >= (1ull << (32 - loimi_jit_op_bits))) want = 0;
            else r->ent[pc] = (int32_t)new_entry(r->body[pc]);
        }
        if (want) {
            loimi_jit_entries[r->ent[pc]] = r->body[pc];
            if (!(r->fl[pc] & F_ENTRY)) {
                r->w[pc] = w;
                r->code[pc] = loimi_jit_enter_op | (uint32_t)r->ent[pc] << loimi_jit_op_bits;
                r->fl[pc] |= F_ENTRY;
            }
        } else if (r->fl[pc] & F_ENTRY) {
            r->code[pc] = r->w[pc];
            r->fl[pc] &= ~F_ENTRY;
        }
    }
}

/* Put the real words back and forget the compiled code (still running
 * copies of it stay valid: they implement the old members, and members
 * are complete). It is compiled again once it is hot again. */
static void drop(struct rec *r) {
    if (debug) fprintf(stderr, "jit: invalidated %p\n", (void *)r->code);
    for (uint64_t i = 0; i < r->n; i++)
        if (r->fl[i] & F_ENTRY) r->code[i] = r->w[i];
    free_compiled(r);
    r->rewriting = 0;
    r->state = r->compiles >= MAX_COMPILES ? S_FAILED : S_NONE;
    if (r->state == S_FAILED) st.given_up++;
    st.invalidations++;
    /* Compile again after a few more calls, instead of a full threshold. */
    uint16_t *c = &loimi_jit_hotcount[((uintptr_t)r->code >> 2) & (HOT_SLOTS - 1)];
    uint16_t soon = threshold / 8 + 1;
    if (*c > soon) *c = soon;
}

static const uint8_t nop5[5] = {0x0f, 0x1f, 0x44, 0x00, 0x00};

static void jmp5(uint8_t *at, uint8_t *to) {
    uint8_t buf[5];
    if (to == at + 5) {
        memcpy(buf, nop5, 5);
    } else {
        int32_t rel = (int32_t)(to - (at + 5));
        buf[0] = 0xe9;
        memcpy(buf + 1, &rel, 4);
    }
    memcpy(WR(at), buf, 5);
}

/* Code for instruction i holding word w that exists already: the copy
 * made at compile time, or one made later. */
static uint8_t *code_for(const struct rec *r, uint64_t i, uint32_t w) {
    if (w == r->cw[i]) return r->home[i] + ((r->fl[i] & F_PREFIX) ? 5 : 0);
    for (uint32_t k = 0; k < r->nalts; k++)
        if (r->alts[k].i == i && r->alts[k].w == w) return r->alts[k].at;
    return NULL;
}

/* A copy of the stencil for word w at instruction i, out of line (never
 * trimmed), or NULL (a different size, out of memory or budget). */
static uint8_t *make_copy(struct rec *r, uint64_t i, uint32_t w) {
    const struct jit_opinfo *ni = &loimi_jit_opinfo[w & OPMASK], *oi = &loimi_jit_opinfo[r->cw[i] & OPMASK];
    if (ni->base == 0xffff || ni->size != oi->size || r->nalts >= 16 + r->n) return NULL;
    uint32_t s = stencil_in(r, w);
    const struct jit_stencil *sp = &loimi_jit_stencils[s];
    uint32_t len = sp->len;
    uint64_t sz = (len + 15) & ~(uint64_t)15;
    if (bump + sz > region_end || !emit_copy(r, i, w, s, sp->hot, bump, bump + sp->hot)) return NULL;
    uint8_t *at = bump;
    bump += sz;
    st.bytes += sz;
    if (r->nalts == r->capalts) {
        r->capalts = r->capalts ? r->capalts * 2 : 8;
        r->alts = realloc(r->alts, r->capalts * sizeof *r->alts);
    }
    r->alts[r->nalts++] = (struct alt){i, w, at};
    st.copies++;
    if (mapf) {
        map_line(at, len, s);
        fflush(mapf);
    }
    return at;
}

/* Point instruction i's home at `body`. The home's 5-byte prefix is a
 * nop in front of the copy made at compile time, or a jump to other code.
 * Every way into the instruction's code goes through its home (direct
 * jumps, falling through from the previous copy) or its entry, so old code
 * that is still running reaches the new code too, and nothing else of
 * the old copy changes. */
static void retarget(struct rec *r, uint64_t i, uint8_t *body) {
    if (body != r->body[i]) {
        jmp5(r->home[i], body);
        r->body[i] = body;
    }
    st.retargets++;
    if (debug)
        fprintf(stderr, "jit: %p+%llu: %s -> %s\n", (void *)r->code, (unsigned long long)i,
                loimi_jit_opnames[r->cw[i] & OPMASK], loimi_jit_opnames[r->w[i] & OPMASK]);
}

static int compile(struct rec *r, const int32_t *oent);

/* Compile r again from its current words, while its old code may still
 * be running: the old code's prefixed homes jump to the new code's, so
 * old code moves over at its next rewritable instruction. Entry indices
 * are kept, so enter words do not change. */
static int recompile(struct rec *r) {
    uint64_t n = r->n;
    uint8_t **oh = r->home, *ofl = r->fl;
    int32_t *oent = r->ent;
    for (uint64_t i = 0; i < n; i++)
        if (ofl[i] & F_ENTRY) r->code[i] = r->w[i];
    r->home = NULL;
    r->fl = NULL;
    r->ent = NULL;
    free_compiled(r);
    r->compiles++;
    int ok = compile(r, oent);
    if (ok) {
        st.recompiles++;
        for (uint64_t i = 0; i < n; i++)
            if ((ofl[i] & F_PREFIX) && r->home[i]) jmp5(oh[i], r->home[i]);
    } else {
        r->state = S_FAILED;
    }
    free(oh);
    free(ofl);
    free(oent);
    return ok;
}

/* Instruction i's word changed to r->w[i]. Code that exists for it is
 * reused (a site that goes back and forth between members); otherwise
 * the code object is compiled again at once (while it has been compiled
 * fewer than max_recompile times), or else given an out-of-line copy for
 * that instruction. Returns 0 if none of that works: drop the code. With
 * `defer`, only the cheap cases are done, and 2 means "recompile". */
static int rewrite(struct rec *r, uint64_t i, int defer) {
    if (!r->home[i]) return 0;
    if (!(r->fl[i] & F_PREFIX)) {
        if (r->compiles >= max_recompile) return 0;
        if (defer) return 2;
        return recompile(r);
    }
    uint8_t *b = code_for(r, i, r->w[i]);
    if (b && ++r->nre[i] > MAX_RETARGETS) {
        /* Each retarget writes code the processor may be running (a
         * pipeline flush): a site that keeps going back and forth runs
         * in the interpreter instead. */
        r->fl[i] |= F_INTERP;
        st.interp++;
        b = exit_code;
    } else if (b) {
        st.reused++;
    } else if (r->compiles < max_recompile) {
        if (defer) return 2;
        return recompile(r);
    } else if (!(b = make_copy(r, i, r->w[i]))) {
        return 0;
    }
    retarget(r, i, b);
    return 1;
}

/* `become` (in the interpreter's counting copies and in stencils): give
 * the word at p opcode opc, and update its compiled code (`rewrite`). */
void loimi_jit_become(uint32_t *p, uint32_t opc) {
    struct rec *r = nrecs ? lookup(p) : NULL;
    if (!r || r->state != S_COMPILED || r->rewriting) {
        *p = (*p & ~OPMASK) | opc;
        return;
    }
    uint64_t i = (uint64_t)(p - r->code);
    if (r->fl[i] & F_INTERP) {
        r->w[i] = *p = (*p & ~OPMASK) | opc;
        return;
    }
    uint32_t old = word_at(r, i), nw = (old & ~OPMASK) | opc;
    if (nw == old) return;
    r->w[i] = nw;
    if (!(r->fl[i] & F_ENTRY)) *p = nw;
    int k = r->home[i] ? rewrite(r, i, 1) : 0;
    if (k == 2) {
        recompile(r);
        return;
    }
    if (!k) {
        st.fallbacks++;
        drop(r);
        return;
    }
    /* Members of a family usually have the same shape: then only the
     * entry changes. */
    const struct jit_opinfo *a = &loimi_jit_opinfo[old & OPMASK], *b = &loimi_jit_opinfo[nw & OPMASK];
    if (!(r->fl[i] & F_INTERP) && (a->flags & 3) == (b->flags & 3) && a->span == b->span &&
        (stencil_for(old) == loimi_jit_nops) == (stencil_for(nw) == loimi_jit_nops)) {
        if (r->fl[i] & F_ENTRY) loimi_jit_entries[r->ent[i]] = r->body[i];
    } else {
        sync_entries(r);
    }
}

/* `Vm.optimize` and `Vm.deopt` rewrite words of a code object between
 * these two calls: the enter words go back first, and afterwards the
 * instructions whose words changed are updated. */
void loimi_jit_invalidate(uint32_t *code) {
    struct rec *r = nrecs ? lookup(code) : NULL;
    if (!r || r->state != S_COMPILED) return;
    for (uint64_t i = 0; i < r->n; i++)
        if (r->fl[i] & F_ENTRY) {
            r->code[i] = r->w[i];
            r->fl[i] &= ~F_ENTRY;
        }
    r->rewriting = 1;
}

void loimi_jit_rewritten(uint32_t *code) {
    struct rec *r = nrecs ? lookup(code) : NULL;
    if (!r || r->state != S_COMPILED || !r->rewriting) return;
    r->rewriting = 0;
    int again = 0;
    for (uint64_t i = 0; i < r->n; i++)
        if (r->home[i] && r->code[i] != r->w[i]) {
            r->w[i] = r->code[i];
            if (r->fl[i] & F_INTERP) continue;
            again = again || !(r->fl[i] & F_PREFIX) || !code_for(r, i, r->w[i]);
        }
    if (again && r->compiles < max_recompile) {
        recompile(r);
        return;
    }
    for (uint64_t i = 0; i < r->n; i++) {
        if (!r->home[i] || (r->fl[i] & F_INTERP)) continue;
        uint8_t *b = code_for(r, i, r->w[i]);
        if (b == r->body[i]) continue;
        if (!rewrite(r, i, 0)) {
            st.fallbacks++;
            drop(r);
            return;
        }
    }
    sync_entries(r);
}

/* Compile code object r from its words (none of them enter words). With
 * oent (a recompile), instructions keep their entry indices. */
static int compile(struct rec *r, const int32_t *oent) {
    uint64_t t0 = now_ns();
    const uint32_t nops = loimi_jit_nops;
    uint64_t n = r->n;
    uint32_t *code = r->code;
    uint32_t *sts = malloc(n * sizeof *sts);
    uint32_t *lens = malloc(n * sizeof *lens);
    int64_t *map = malloc(n * sizeof *map), *cmap = malloc(n * sizeof *cmap);
    int ok = 0;
    uint64_t ninsn = 0, nexit = 0;
    free_compiled(r);
    r->home = calloc(n, sizeof *r->home);
    r->body = calloc(n, sizeof *r->body);
    r->w = malloc(n * sizeof *r->w);
    r->cw = malloc(n * sizeof *r->cw);
    r->ent = malloc(n * sizeof *r->ent);
    r->fl = calloc(n, 1);
    r->nre = calloc(n, 1);
    for (uint64_t i = 0; i < n; i++) {
        r->ent[i] = -1;
        r->w[i] = r->cw[i] = code[i];
        map[i] = -1;
    }
    /* Layout: one stencil per instruction (a super's first part at its
     * head word; the other parts are instructions of their own), behind a
     * 5-byte nop for instructions whose word may be rewritten. The cold
     * parts follow all the hot ones. */
    int64_t off = 0, coff = 0;
    for (uint64_t pc = 0; pc < n;) {
        uint32_t op = code[pc] & OPMASK;
        const struct jit_opinfo *oi = &loimi_jit_opinfo[op];
        if (oi->base == 0xffff || oi->size == 0 || pc + oi->size > n) {
            if (debug) fprintf(stderr, "jit: %p: no op at word %llu (%08x)\n", (void *)code, (unsigned long long)pc, code[pc]);
            goto out;
        }
        uint32_t s = stencil_in(r, code[pc]);
        if (s == nops) nexit++;
        const struct jit_stencil *sp = &loimi_jit_stencils[s];
        uint32_t len = sp->hot;
        if (sp->trim >= 0 && (uint64_t)sp->trim == oi->size && pc + oi->size < n) len -= 5;
        cmap[pc] = coff;
        coff += sp->len - sp->hot;
        if (oi->flags & 4) {
            r->fl[pc] |= F_PREFIX;
            off += 5;
        }
        map[pc] = off;
        sts[pc] = s;
        lens[pc] = len;
        off += len;
        pc += oi->size;
        ninsn++;
    }
    off = (off + 15) & ~(int64_t)15;
    int64_t hot_end = off;
    off = (off + coff + 15) & ~(int64_t)15;
    if (bump + off > region_end) goto out;
    uint8_t *dst = bump;
    for (uint64_t pc = 0; pc < n; pc++)
        if (map[pc] >= 0) {
            r->body[pc] = dst + map[pc];
            r->home[pc] = r->body[pc] - ((r->fl[pc] & F_PREFIX) ? 5 : 0);
        }
    for (uint64_t pc = 0; pc < n; pc++) {
        if (map[pc] < 0) continue;
        if (r->fl[pc] & F_PREFIX) memcpy(WR(r->home[pc]), nop5, 5);
        if (!emit_copy(r, pc, code[pc], sts[pc], lens[pc], r->body[pc], dst + hot_end + cmap[pc])) goto out;
    }
    bump += off;
    if (oent)
        for (uint64_t pc = 0; pc < n; pc++)
            if (r->home[pc]) r->ent[pc] = oent[pc];
    sync_entries(r);
    if (mapf) {
        for (uint64_t pc = 0; pc < n; pc++)
            if (map[pc] >= 0) map_line(r->body[pc], lens[pc], sts[pc]);
        fflush(mapf);
    }
    if (debug) {
        fprintf(stderr, "jit: compiled %p (%llu words) at %p\n", (void *)code, (unsigned long long)n, (void *)dst);
        for (uint64_t pc = 0; pc < n; pc++) {
            if (map[pc] < 0) continue;
            uint32_t w = r->w[pc];
            int v;
            const char *nm = stencil_name(sts[pc], &v);
            fprintf(stderr, "  %4llu %08x %-12s %s%s +%lld%s%s\n", (unsigned long long)pc, w, loimi_jit_opnames[w & OPMASK],
                    nm, v ? "*" : "", (long long)map[pc],
                    (r->fl[pc] & F_ENTRY) ? " entry" : "", (r->fl[pc] & F_PREFIX) ? " prefix" : "");
        }
    }
    for (uint64_t pc = 0; pc < n; pc++) st.patched += (r->fl[pc] & F_ENTRY) != 0;
    st.bytes += off;
    st.insns += ninsn;
    st.exits += nexit;
    ok = 1;
out:
    free(map);
    free(cmap);
    free(sts);
    free(lens);
    if (!ok) free_compiled(r);
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
    r->state = compile(r, NULL) ? S_COMPILED : S_FAILED;
}
