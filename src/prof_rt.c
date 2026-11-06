/* Run-time half of `loimi build --vm-prof`: a sampling profiler linked
 * into the program (docs/language.md section 7.3, "Sampling: --vm-prof").
 *
 * A timer sends SIGPROF; each sample records the interrupted instruction
 * address, the vm's last `pc` (stored by every op's handler, read through
 * the getter in `struct prof_vm`) and the return addresses on the stack
 * (the program is built with unwind tables). At exit the samples are
 * named, in the process, and printed as three tables sorted by count:
 *
 *  - self: where the samples landed: an op's handler, a cold block (the
 *    names `--vm-stats` prints), a function of the program (the runtime:
 *    the collector, `std`), JIT code (by stencil), or a shared library's
 *    function with the program function that called it
 *    ("libc.so.6:memset <- std.gc.Heap.collect");
 *  - by vm location, with what it calls: the innermost op or cold block
 *    on the stack, so a collection is charged to the op that allocated;
 *  - by guest pc: the vm's `pc` at the op's entry, with that location.
 *
 * Code locations: the compiler records marks (address, key) in section
 * `loimi_prof_marks`: functions by their address (tail-call handlers,
 * outlined cold blocks, and in goto and switch modes the function that
 * holds all handlers), and inline cold blocks by an empty inline asm at
 * their first instruction and at their ends (where the op's code
 * resumes). A sample belongs to the last mark at or before it within the
 * same function (from the ELF symbol table). In goto and switch modes
 * each op stores its number on entry, and that op owns the sample unless
 * the mark is one of its cold blocks: LLVM merges and copies blocks
 * between the handlers of one function. The vms' names are in
 * section `loimi_prof_vms`. Under `--jit-stencils` the JIT calls
 * `loimi_prof_jit` for each instruction it compiles.
 *
 * Environment: LOIMI_PROF=0 turns sampling off; LOIMI_PROF_HZ=n sets the
 * rate (default 1000); LOIMI_PROF_CLOCK=cpu samples the process's CPU
 * time instead of wall-clock time (a CPU-time timer fires at most once
 * per kernel tick, often 250 Hz; the wall-clock timer is a
 * high-resolution timer, and a busy interpreter's wall-clock time is its
 * CPU time; time blocked in a system call shows as the libc function
 * that made it); LOIMI_PROF_OUT=file writes the
 * report there instead of stderr; LOIMI_PROF_TOP=n prints n rows per
 * table (default 30, 0: all); LOIMI_PROF_DEPTH=n keeps n return
 * addresses per sample (default 24, 0: no stack walk); LOIMI_PROF_DEBUG=1
 * lists the marks and the stacks of samples outside vm code on stderr. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <elf.h>
#include <errno.h>
#include <execinfo.h>
#include <fcntl.h>
#include <link.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <ucontext.h>
#include <unistd.h>

struct prof_vm {
    uint64_t tag; /* key bits above the kind and index: build tag, vm */
    const char *name;
    uint64_t nops, ncold;
    const char *const *ops;
    const char *const *colds;
    void *(*pc_get)(void);
    uint64_t nvms;
    uint32_t (*op_get)(void); /* goto and switch modes: the op running */
    const int32_t *owners;      /* per cold block: its op, or -1 */
    uint64_t index;             /* of this vm in its program */
};
struct prof_mark { uintptr_t addr; uint64_t key; };

extern struct prof_vm __start_loimi_prof_vms[] __attribute__((weak, visibility("hidden")));
extern struct prof_vm __stop_loimi_prof_vms[] __attribute__((weak, visibility("hidden")));
extern struct prof_mark __start_loimi_prof_marks[] __attribute__((weak, visibility("hidden")));
extern struct prof_mark __stop_loimi_prof_marks[] __attribute__((weak, visibility("hidden")));

#define KIND_OP 0
#define KIND_COLD 1
#define KIND_RESUME 3
#define KEY_VM(k) ((k) & ~(uint64_t)0xffffff)
#define KEY_KIND(k) (((k) >> 20) & 0xf)
#define KEY_IDX(k) ((k) & 0xfffff)

/* One profiler per process: several `--vm-prof` libraries in one host
 * share this flag (weak), and the first to start samples for all. */
__attribute__((weak)) int loimi_prof_running_1;

/* --- Sampling -------------------------------------------------------- */

/* Samples: [n | op << 32, ip, pc, n return addresses], in chunks mapped as needed
 * (mmap is async-signal-safe). */
#define CHUNK_WORDS (1u << 17)
#define MAX_CHUNKS 512
#define MAX_DEPTH 64
static uint64_t *chunks[MAX_CHUNKS];
static uint32_t chunk_used[MAX_CHUNKS];
static int nchunks;
static uint64_t nsamples, ndropped;
static int busy;
static int depth = 24;
static void *(*pc_get)(void);
static uint32_t (*op_get)(void);
static timer_t timer;
static int timer_ok;
static long hz = 1000;
static int wall;
static struct timespec t_start;
static int mine; /* this copy started the timer */

static uint64_t *reserve(uint32_t words) {
    if (nchunks == 0 || chunk_used[nchunks - 1] + words > CHUNK_WORDS) {
        if (nchunks == MAX_CHUNKS) return NULL;
        void *m = mmap(NULL, CHUNK_WORDS * 8, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (m == MAP_FAILED) return NULL;
        chunks[nchunks++] = m;
    }
    uint64_t *r = chunks[nchunks - 1] + chunk_used[nchunks - 1];
    chunk_used[nchunks - 1] += words;
    return r;
}

static void on_sample(int sig, siginfo_t *si, void *ctx) {
    (void)sig;
    (void)si;
    int saved = errno;
    ucontext_t *uc = ctx;
#if defined(__x86_64__)
    uintptr_t ip = (uintptr_t)uc->uc_mcontext.gregs[REG_RIP];
#elif defined(__aarch64__)
    uintptr_t ip = (uintptr_t)uc->uc_mcontext.pc;
#else
    uintptr_t ip = 0;
#endif
    void *pc = pc_get ? pc_get() : NULL;
    uint32_t op = op_get ? op_get() : 0;
    void *bt[MAX_DEPTH + 8];
    int n = depth ? backtrace(bt, depth + 8) : 0, k = -1;
    /* The frames above the handler and the signal trampoline. */
    for (int i = 0; i < n && i < 8; i++)
        if ((uintptr_t)bt[i] == ip) {
            k = i + 1;
            break;
        }
    int nf = k < 0 ? 0 : n - k;
    if (nf > depth) nf = depth;
    /* Two threads may take a sample at once: the second drops its own. */
    if (__atomic_exchange_n(&busy, 1, __ATOMIC_ACQUIRE)) {
        __atomic_fetch_add(&ndropped, 1, __ATOMIC_RELAXED);
        errno = saved;
        return;
    }
    uint64_t *r = reserve(3 + nf);
    if (r) {
        r[0] = (uint64_t)nf | (uint64_t)op << 32;
        r[1] = ip;
        r[2] = (uintptr_t)pc;
        for (int i = 0; i < nf; i++) r[3 + i] = (uintptr_t)bt[k + i];
        nsamples++;
    } else {
        __atomic_fetch_add(&ndropped, 1, __ATOMIC_RELAXED);
    }
    __atomic_store_n(&busy, 0, __ATOMIC_RELEASE);
    errno = saved;
}

static long env_long(const char *name, long dflt) {
    const char *s = getenv(name);
    if (!s || !*s) return dflt;
    char *end;
    long v = strtol(s, &end, 10);
    return *end ? dflt : v;
}

__attribute__((constructor)) static void prof_start(void) {
    const char *on = getenv("LOIMI_PROF");
    if (on && strcmp(on, "0") == 0) return;
    if (loimi_prof_running_1) return;
    loimi_prof_running_1 = 1;
    mine = 1;
    for (struct prof_vm *v = __start_loimi_prof_vms; v && v < __stop_loimi_prof_vms; v++)
        if (!pc_get) {
            pc_get = v->pc_get;
            op_get = v->op_get;
        }
    hz = env_long("LOIMI_PROF_HZ", 1000);
    if (hz < 1) hz = 1;
    if (hz > 100000) hz = 100000;
    depth = (int)env_long("LOIMI_PROF_DEPTH", 24);
    if (depth < 0) depth = 0;
    if (depth > MAX_DEPTH) depth = MAX_DEPTH;
    const char *c = getenv("LOIMI_PROF_CLOCK");
    wall = !(c && strcmp(c, "cpu") == 0);
    /* The unwinder loads and allocates on its first use: not in a signal
     * handler. */
    void *warm[4];
    if (depth) backtrace(warm, 4);
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = on_sample;
    sa.sa_flags = SA_SIGINFO | SA_RESTART;
    sigemptyset(&sa.sa_mask);
    sigaction(SIGPROF, &sa, NULL);
    struct sigevent se;
    memset(&se, 0, sizeof se);
    se.sigev_notify = SIGEV_SIGNAL;
    se.sigev_signo = SIGPROF;
    if (timer_create(wall ? CLOCK_MONOTONIC : CLOCK_PROCESS_CPUTIME_ID, &se, &timer) != 0) return;
    timer_ok = 1;
    long ns = 1000000000L / hz;
    struct itimerspec its = {{ns / 1000000000L, ns % 1000000000L}, {ns / 1000000000L, ns % 1000000000L}};
    clock_gettime(wall ? CLOCK_MONOTONIC : CLOCK_PROCESS_CPUTIME_ID, &t_start);
    timer_settime(timer, 0, &its, NULL);
}

/* --- JIT code (`--jit-stencils`) -------------------------------------- */

struct jrange { uintptr_t lo, hi; const char *name; int variant; uintptr_t pc; uint64_t seq; };
static struct jrange *jr;
static size_t njr, capjr;

/* Called by jit_rt.c for each compiled instruction: its code, its op's
 * name (variant: a `*`) and its word. */
void loimi_prof_jit(const void *lo, const void *hi, const char *name, int variant, const void *pc) {
    if (njr == capjr) {
        size_t c = capjr ? capjr * 2 : 1024;
        struct jrange *n = realloc(jr, c * sizeof *n);
        if (!n) return;
        jr = n;
        capjr = c;
    }
    jr[njr] = (struct jrange){(uintptr_t)lo, (uintptr_t)hi, name, variant, (uintptr_t)pc, njr};
    njr++;
}

static int cmp_jr(const void *a, const void *b) {
    const struct jrange *x = a, *y = b;
    if (x->lo != y->lo) return x->lo < y->lo ? -1 : 1;
    return x->seq < y->seq ? -1 : x->seq > y->seq;
}

/* The latest range holding a. */
static const struct jrange *find_jit(uintptr_t a) {
    size_t lo = 0, hi = njr;
    while (lo < hi) {
        size_t m = (lo + hi) / 2;
        if (jr[m].lo <= a) lo = m + 1;
        else hi = m;
    }
    const struct jrange *best = NULL;
    for (size_t i = lo, seen = 0; i > 0 && seen < 64; i--, seen++)
        if (jr[i - 1].lo <= a && a < jr[i - 1].hi && (!best || jr[i - 1].seq > best->seq)) best = &jr[i - 1];
    return best;
}

/* --- Symbols ---------------------------------------------------------- */

struct sym { uintptr_t lo, hi; const char *name; };
struct obj {
    char path[512];
    const char *base;
    uintptr_t bias;
    uintptr_t seg_lo[8], seg_hi[8];
    int nseg, loaded, prog;
    struct sym *syms;
    size_t nsyms;
};
#define MAX_OBJS 256
static struct obj objs[MAX_OBJS];
static int nobjs;

static int add_obj(struct dl_phdr_info *info, size_t size, void *data) {
    (void)size;
    (void)data;
    if (nobjs == MAX_OBJS) return 0;
    struct obj *o = &objs[nobjs];
    memset(o, 0, sizeof *o);
    o->bias = info->dlpi_addr;
    for (int i = 0; i < info->dlpi_phnum && o->nseg < 8; i++) {
        const ElfW(Phdr) *ph = &info->dlpi_phdr[i];
        if (ph->p_type == PT_LOAD && (ph->p_flags & PF_X)) {
            o->seg_lo[o->nseg] = info->dlpi_addr + ph->p_vaddr;
            o->seg_hi[o->nseg] = info->dlpi_addr + ph->p_vaddr + ph->p_memsz;
            o->nseg++;
        }
    }
    if (!o->nseg) return 0;
    const char *name = info->dlpi_name;
    if (!name || !*name) {
        if (nobjs == 0) {
            ssize_t n = readlink("/proc/self/exe", o->path, sizeof o->path - 1);
            o->path[n > 0 ? n : 0] = 0;
        }
    } else {
        snprintf(o->path, sizeof o->path, "%s", name);
    }
    const char *sl = strrchr(o->path, '/');
    o->base = sl ? sl + 1 : (o->path[0] ? o->path : "?");
    nobjs++;
    return 0;
}

static int cmp_sym(const void *a, const void *b) {
    const struct sym *x = a, *y = b;
    if (x->lo != y->lo) return x->lo < y->lo ? -1 : 1;
    /* Of aliases, the one with a size first. */
    return (y->hi > y->lo) - (x->hi > x->lo);
}

/* The object's function symbols: `.symtab`, else `.dynsym`. */
static void load_syms(struct obj *o) {
    o->loaded = 1;
    int fd = o->path[0] ? open(o->path, O_RDONLY | O_CLOEXEC) : -1;
    if (fd < 0) return;
    struct stat st;
    if (fstat(fd, &st) != 0 || st.st_size < (off_t)sizeof(ElfW(Ehdr))) {
        close(fd);
        return;
    }
    const uint8_t *m = mmap(NULL, st.st_size, PROT_READ, MAP_PRIVATE, fd, 0);
    close(fd);
    if (m == MAP_FAILED) return;
    const ElfW(Ehdr) *eh = (const void *)m;
    if (memcmp(eh->e_ident, ELFMAG, SELFMAG) != 0 || eh->e_shoff == 0 ||
        eh->e_shoff + (uint64_t)eh->e_shnum * sizeof(ElfW(Shdr)) > (uint64_t)st.st_size)
        return;
    const ElfW(Shdr) *sh = (const void *)(m + eh->e_shoff);
    const ElfW(Shdr) *tab = NULL;
    for (int want = SHT_SYMTAB; !tab; want = SHT_DYNSYM) {
        for (int i = 0; i < eh->e_shnum; i++)
            if (sh[i].sh_type == (uint32_t)want) tab = &sh[i];
        if (want == SHT_DYNSYM) break;
    }
    if (!tab || tab->sh_link >= eh->e_shnum) return;
    const ElfW(Shdr) *strs = &sh[tab->sh_link];
    if (tab->sh_offset + tab->sh_size > (uint64_t)st.st_size || strs->sh_offset + strs->sh_size > (uint64_t)st.st_size) return;
    const ElfW(Sym) *s = (const void *)(m + tab->sh_offset);
    size_t n = tab->sh_size / sizeof *s;
    o->syms = malloc(n * sizeof *o->syms);
    if (!o->syms) return;
    for (size_t i = 0; i < n; i++) {
        int t = ELF64_ST_TYPE(s[i].st_info);
        if ((t != STT_FUNC && t != STT_GNU_IFUNC) || s[i].st_shndx == SHN_UNDEF || s[i].st_value == 0) continue;
        if (s[i].st_name >= strs->sh_size) continue;
        uintptr_t lo = o->bias + s[i].st_value;
        o->syms[o->nsyms++] = (struct sym){lo, lo + s[i].st_size, (const char *)(m + strs->sh_offset + s[i].st_name)};
    }
    qsort(o->syms, o->nsyms, sizeof *o->syms, cmp_sym);
}

static struct obj *find_obj(uintptr_t a) {
    for (int i = 0; i < nobjs; i++)
        for (int j = 0; j < objs[i].nseg; j++)
            if (objs[i].seg_lo[j] <= a && a < objs[i].seg_hi[j]) return &objs[i];
    return NULL;
}

static const struct sym *find_sym(struct obj *o, uintptr_t a) {
    if (!o->loaded) load_syms(o);
    size_t lo = 0, hi = o->nsyms;
    while (lo < hi) {
        size_t m = (lo + hi) / 2;
        if (o->syms[m].lo <= a) lo = m + 1;
        else hi = m;
    }
    if (lo == 0) return NULL;
    const struct sym *s = &o->syms[lo - 1];
    /* A symbol with a size holds what it covers; one without (assembly)
     * reaches to the next. */
    while (s > o->syms && s->hi > s->lo && a >= s->hi && (s - 1)->lo == s->lo) s--;
    if (s->hi > s->lo && a >= s->hi) return NULL;
    return s;
}

/* --- Marks ------------------------------------------------------------ */

struct mark { uintptr_t addr; uint64_t key; const struct prof_vm *vm; };
static struct mark *marks;
static size_t nmarks;

static int cmp_mark(const void *a, const void *b) {
    const struct mark *x = a, *y = b;
    return x->addr < y->addr ? -1 : x->addr > y->addr;
}

static void load_marks(void) {
    size_t n = __start_loimi_prof_marks ? (size_t)(__stop_loimi_prof_marks - __start_loimi_prof_marks) : 0;
    marks = malloc((n ? n : 1) * sizeof *marks);
    if (!marks) return;
    for (size_t i = 0; i < n; i++) {
        const struct prof_mark *m = &__start_loimi_prof_marks[i];
        const struct prof_vm *vm = NULL;
        for (struct prof_vm *v = __start_loimi_prof_vms; v && v < __stop_loimi_prof_vms; v++)
            if (v->tag == KEY_VM(m->key)) vm = v;
        if (!vm || !m->addr) continue;
        uint64_t k = KEY_KIND(m->key), x = KEY_IDX(m->key);
        if ((k == KIND_OP && x >= vm->nops) || (k == KIND_COLD && x >= vm->ncold) || (k != KIND_OP && k != KIND_COLD && k != KIND_RESUME)) continue;
        marks[nmarks++] = (struct mark){m->addr, m->key, vm};
    }
    qsort(marks, nmarks, sizeof *marks, cmp_mark);
}

static const struct mark *find_mark(uintptr_t a) {
    size_t lo = 0, hi = nmarks;
    while (lo < hi) {
        size_t m = (lo + hi) / 2;
        if (marks[m].addr <= a) lo = m + 1;
        else hi = m;
    }
    return lo ? &marks[lo - 1] : NULL;
}

/* --- Locations -------------------------------------------------------- */

enum { L_NONE, L_OP, L_COLD, L_JIT, L_FN, L_LIB };
struct loc {
    int kind;
    const struct prof_vm *vm;
    uint64_t idx;
    const char *name;
    const struct obj *obj;
    uintptr_t off;
    const struct jrange *jit;
};

/* Where address a is, with `cur` the op running when the sample was
 * taken (goto and switch modes: vm index << 20 | op, plus 1; else 0). */
static struct loc locate(uintptr_t a, uint32_t cur) {
    struct loc l;
    memset(&l, 0, sizeof l);
    const struct jrange *j = njr ? find_jit(a) : NULL;
    if (j) {
        l.kind = L_JIT;
        l.jit = j;
        return l;
    }
    struct obj *o = find_obj(a);
    if (!o) return l;
    const struct sym *s = find_sym(o, a);
    if (o->prog) {
        const struct mark *m = find_mark(a);
        /* After the end of a cold block: the op (or dispatch) whose mark
         * is before the cold block's, or the function's own mark. */
        while (m && KEY_KIND(m->key) == KIND_RESUME && m > marks && (!s || m->addr >= s->lo)) {
            const struct mark *q = m - 1;
            while (q > marks && (KEY_KIND(q->key) == KIND_RESUME || KEY_KIND(q->key) == KIND_COLD) && !(s && q->addr == s->lo)) q--;
            m = q;
        }
        if (m && KEY_KIND(m->key) != KIND_RESUME && (!s || m->addr >= s->lo)) {
            uint64_t k = KEY_KIND(m->key);
            l.kind = k == KIND_OP ? L_OP : L_COLD;
            l.vm = m->vm;
            l.idx = KEY_IDX(m->key);
            /* One function for all handlers: the op that runs owns the
             * code, unless it is in one of that op's cold blocks (or a vm
             * function's, inlined into it). */
            if (cur && m->vm->op_get && ((cur - 1) >> 20) == m->vm->index) {
                uint64_t op = (cur - 1) & 0xfffff;
                if (op < m->vm->nops && !(k == KIND_COLD && (m->vm->owners[l.idx] < 0 || (uint64_t)m->vm->owners[l.idx] == op))) {
                    l.kind = L_OP;
                    l.idx = op;
                }
            }
            return l;
        }
        /* Before the first handler of a function that holds them all:
         * the switch, which the running op dispatches through. */
        const struct mark *last = s && s->hi > s->lo ? find_mark(s->hi - 1) : NULL;
        if (cur && last && last->addr >= s->lo && last->vm->op_get && ((cur - 1) >> 20) == last->vm->index &&
            ((cur - 1) & 0xfffff) < last->vm->nops) {
            l.kind = L_OP;
            l.vm = last->vm;
            l.idx = (cur - 1) & 0xfffff;
            return l;
        }
    }
    l.kind = o->prog ? L_FN : L_LIB;
    l.obj = o;
    l.name = s ? s->name : NULL;
    l.off = a - o->bias;
    return l;
}

static int in_vm(const struct loc *l) { return l->kind >= L_OP && l->kind <= L_JIT; }

static void loc_name(const struct loc *l, char *buf, size_t n) {
    const char *pre = "", *dot = "";
    if (l->vm && l->vm->nvms > 1) {
        pre = l->vm->name;
        dot = ".";
    }
    switch (l->kind) {
    case L_OP: snprintf(buf, n, "%s%s%s", pre, dot, l->vm->ops[l->idx]); break;
    case L_COLD: snprintf(buf, n, "%s%s%s", pre, dot, l->vm->colds[l->idx]); break;
    case L_JIT: snprintf(buf, n, "%s%s (jit)", l->jit->name, l->jit->variant ? "*" : ""); break;
    case L_FN:
        if (l->name) snprintf(buf, n, "%s", l->name);
        else snprintf(buf, n, "%s+0x%lx", l->obj->base, (unsigned long)l->off);
        break;
    case L_LIB:
        if (l->name) snprintf(buf, n, "%s:%s", l->obj->base, l->name);
        else snprintf(buf, n, "%s+0x%lx", l->obj->base, (unsigned long)l->off);
        break;
    default: snprintf(buf, n, "?"); break;
    }
}

/* --- Counting by name --------------------------------------------------- */

struct entry { char *name; uint64_t n; };
struct table { struct entry *e; size_t cap, used; uint64_t total; };

static uint64_t hash_str(const char *s) {
    uint64_t h = 0xcbf29ce484222325ULL;
    for (; *s; s++) h = (h ^ (uint8_t)*s) * 0x100000001b3ULL;
    return h;
}

static void count(struct table *t, const char *name) {
    t->total++;
    if (2 * (t->used + 1) > t->cap) {
        size_t c = t->cap ? t->cap * 2 : 256;
        struct entry *e = calloc(c, sizeof *e);
        if (!e) return;
        for (size_t i = 0; i < t->cap; i++)
            if (t->e[i].name) {
                size_t j = hash_str(t->e[i].name) & (c - 1);
                while (e[j].name) j = (j + 1) & (c - 1);
                e[j] = t->e[i];
            }
        free(t->e);
        t->e = e;
        t->cap = c;
    }
    size_t j = hash_str(name) & (t->cap - 1);
    while (t->e[j].name && strcmp(t->e[j].name, name) != 0) j = (j + 1) & (t->cap - 1);
    if (!t->e[j].name) {
        t->e[j].name = strdup(name);
        t->used++;
    }
    t->e[j].n++;
}

static int cmp_entry(const void *a, const void *b) {
    const struct entry *x = a, *y = b;
    if (x->n != y->n) return x->n > y->n ? -1 : 1;
    return strcmp(x->name, y->name);
}

static void print_table(FILE *f, struct table *t, const char *title, const char *head, uint64_t all, long top) {
    size_t n = 0;
    for (size_t i = 0; i < t->cap; i++)
        if (t->e[i].name) t->e[n++] = t->e[i];
    qsort(t->e, n, sizeof *t->e, cmp_entry);
    fprintf(f, "\n%s\n%10s  %6s  %6s  %s\n", title, "samples", "%", "cum%", head);
    uint64_t cum = 0;
    size_t shown = top > 0 && (size_t)top < n ? (size_t)top : n;
    for (size_t i = 0; i < shown; i++) {
        cum += t->e[i].n;
        fprintf(f, "%10llu  %5.1f%%  %5.1f%%  %s\n", (unsigned long long)t->e[i].n, 100.0 * t->e[i].n / all, 100.0 * cum / all, t->e[i].name);
    }
    if (shown < n) {
        uint64_t rest = 0;
        for (size_t i = shown; i < n; i++) rest += t->e[i].n;
        fprintf(f, "%10llu  %5.1f%%          (%zu more)\n", (unsigned long long)rest, 100.0 * rest / all, n - shown);
    }
}

/* --- The report ----------------------------------------------------------- */

__attribute__((destructor)) static void prof_report(void) {
    if (!mine) return;
    if (timer_ok) timer_delete(timer);
    struct timespec t_end;
    clock_gettime(wall ? CLOCK_MONOTONIC : CLOCK_PROCESS_CPUTIME_ID, &t_end);
    sigset_t ss;
    sigemptyset(&ss);
    sigaddset(&ss, SIGPROF);
    sigprocmask(SIG_BLOCK, &ss, NULL);
    double secs = (t_end.tv_sec - t_start.tv_sec) + (t_end.tv_nsec - t_start.tv_nsec) / 1e9;

    dl_iterate_phdr(add_obj, NULL);
    struct obj *self = find_obj((uintptr_t)&prof_report);
    for (int i = 0; i < nobjs; i++) objs[i].prog = &objs[i] == self;
    load_marks();
    if (njr) qsort(jr, njr, sizeof *jr, cmp_jr);

    struct table self_t = {0}, vm_t = {0}, pc_t = {0};
    char a[600], b[600], line[1300];
    if (getenv("LOIMI_PROF_DEBUG"))
        for (size_t i = 0; i < nmarks; i++) {
            struct loc l = locate(marks[i].addr, 0);
            loc_name(&l, a, sizeof a);
            fprintf(stderr, "mark %lx %s\n", (unsigned long)marks[i].addr, a);
        }
    for (int c = 0; c < nchunks; c++) {
        uint64_t *r = chunks[c], *end = chunks[c] + chunk_used[c];
        while (r < end) {
            uint64_t nf = r[0] & 0xffffffff;
            uint32_t cur = (uint32_t)(r[0] >> 32);
            uintptr_t ip = r[1], pc = r[2];
            const uint64_t *fr = r + 3;
            r += 3 + nf;
            struct loc l0 = locate(ip, cur);
            loc_name(&l0, a, sizeof a);
            /* Library code: name the program function that called it. */
            if (l0.kind == L_LIB || l0.kind == L_NONE) {
                for (uint64_t i = 0; i < nf; i++) {
                    struct loc l = locate(fr[i] - 1, cur);
                    if (l.kind != L_LIB && l.kind != L_NONE) {
                        loc_name(&l, b, sizeof b);
                        snprintf(line, sizeof line, "%s <- %s", a, b);
                        snprintf(a, sizeof a, "%s", line);
                        break;
                    }
                }
            }
            count(&self_t, a);
            /* The innermost vm location on the stack. */
            struct loc lv = l0;
            for (uint64_t i = 0; !in_vm(&lv) && i < nf; i++) lv = locate(fr[i] - 1, cur);
            if (!in_vm(&lv)) {
                if (getenv("LOIMI_PROF_DEBUG")) {
                    fprintf(stderr, "outside: %s", a);
                    for (uint64_t i = 0; i < nf; i++) {
                        struct loc l = locate(fr[i] - 1, cur);
                        loc_name(&l, b, sizeof b);
                        fprintf(stderr, " <- %s", b);
                    }
                    fprintf(stderr, "\n");
                }
                count(&vm_t, "(outside vm code)");
                continue;
            }
            loc_name(&lv, b, sizeof b);
            count(&vm_t, b);
            if (lv.kind == L_JIT) pc = lv.jit->pc;
            if (!pc) continue;
            snprintf(line, sizeof line, "0x%012lx  %s", (unsigned long)pc, b);
            count(&pc_t, line);
        }
    }

    const char *out = getenv("LOIMI_PROF_OUT");
    FILE *f = out && *out ? fopen(out, "w") : NULL;
    if (!f) f = stderr;
    long top = env_long("LOIMI_PROF_TOP", 30);
    fprintf(f, "\n== vm prof: %llu samples, %ld Hz of %s time, %.3f s ==\n", (unsigned long long)nsamples, hz,
            wall ? "wall-clock" : "CPU", secs);
    if (ndropped) fprintf(f, "(%llu samples dropped: out of buffer, or taken on two threads at once)\n", (unsigned long long)ndropped);
    if (nsamples) {
        print_table(f, &self_t, "self (where the samples landed):", "location", nsamples, top);
        print_table(f, &vm_t, "by vm location, with what it calls (the innermost op or cold block on the stack):", "location",
                    nsamples, top);
        if (pc_t.total)
            print_table(f, &pc_t, "by guest pc (the vm's pc when the op started):", "pc              location", nsamples, top);
    }
    if (f != stderr) fclose(f);
    else fflush(f);
}
