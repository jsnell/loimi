// `--vm-stats` runtime: longer op sequences. The generated code counts
// ops and adjacent pairs itself; for sequences of 3 and 4 ops it calls
// `loimi_stats_seq` on every op, and the report ends with
// `loimi_stats_suggest`: the superinstructions to add, pairs and longer.
//
// Each vm has one `loimi_seq`, zero-initialized: the last three ops (the
// window restarts when an op does not directly follow the previous one
// in the code) and a hash table of the sequences of 2 to 4 ops ending at
// each op, with their counts.
//
// The compiler renames the two entry points with a hash of this file
// (`vmgen::stats_rt_sym`), and they are weak, so that several
// `--vm-stats` libraries link into one host.
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    uint64_t key;  // ops, 16 bits each, the last in the low bits
    uint64_t count;
    uint32_t len;  // 0: empty slot
} loimi_seq_entry;

typedef struct {
    uint64_t win[3];
    int64_t len;
    loimi_seq_entry *tab;
    uint64_t used;
} loimi_seq;

#define SEQ_CAP (1u << 18)

static uint64_t seq_hash(uint64_t key, uint32_t len) {
    uint64_t h = (key ^ ((uint64_t)len << 61)) * 0x9e3779b97f4a7c15ull;
    return h >> 46;  // 18 bits
}

static loimi_seq_entry *seq_find(loimi_seq *s, uint64_t key, uint32_t len, int insert) {
    uint64_t i = seq_hash(key, len);
    for (;;) {
        loimi_seq_entry *e = &s->tab[i];
        if (e->len == len && e->key == key) return e;
        if (e->len == 0) {
            // Leave room: past 3/4 full, new sequences are not counted.
            if (!insert || s->used * 4 >= (uint64_t)SEQ_CAP * 3) return 0;
            e->len = len;
            e->key = key;
            s->used++;
            return e;
        }
        i = (i + 1) & (SEQ_CAP - 1);
    }
}

__attribute__((weak, visibility("hidden"))) void loimi_stats_seq(loimi_seq *s, int64_t adjacent, int64_t op) {
    if (!s->tab) {
        s->tab = calloc(SEQ_CAP, sizeof(loimi_seq_entry));
        if (!s->tab) return;
    }
    if (!adjacent || op >= 65536) s->len = 0;
    uint64_t key = (uint64_t)op;
    for (int64_t l = 1; l <= s->len; l++) {
        key |= s->win[s->len - l] << (16 * l);
        loimi_seq_entry *e = seq_find(s, key, (uint32_t)l + 1, 1);
        if (e) e->count++;
    }
    if (op >= 65536) return;
    if (s->len == 3) {
        s->win[0] = s->win[1];
        s->win[1] = s->win[2];
        s->len = 2;
    }
    s->win[s->len++] = (uint64_t)op;
}

static uint64_t seq_count(loimi_seq *s, uint64_t key, uint32_t len) {
    loimi_seq_entry *e = seq_find(s, key, len, 0);
    return e ? e->count : 0;
}

// A suggested (or existing) super: its ops, first op first.
typedef struct {
    uint32_t ops[8];
    int len;
    uint64_t count;
    uint64_t saving;  // dispatches saved: count * (len - 1)
    int64_t clash;    // -1: suggested; else what it overlaps (see below)
} loimi_cand;

// Whether `optimize` could not fuse both in some sequence: a proper
// suffix of one is a prefix of the other, or one contains the other (as
// the `super_overlap` lint).
static int cand_overlap(const loimi_cand *x, const loimi_cand *y) {
    const uint32_t *a = x->ops, *b = y->ops;
    int na = x->len, nb = y->len;
    for (int pass = 0; pass < 2; pass++) {
        for (int k = 1; k <= na && k <= nb; k++) {  // a's last k == b's first k
            int eq = 1;
            for (int j = 0; j < k; j++) eq &= a[na - k + j] == b[j];
            if (eq) return 1;
        }
        for (int off = 0; off + nb <= na; off++) {  // a contains b
            int eq = 1;
            for (int j = 0; j < nb; j++) eq &= a[off + j] == b[j];
            if (eq) return 1;
        }
        const uint32_t *t = a;
        a = b;
        b = t;
        int tn = na;
        na = nb;
        nb = tn;
    }
    return 0;
}

static int by_saving(const void *a, const void *b) {
    const loimi_cand *x = a, *y = b;
    return x->saving < y->saving ? 1 : x->saving > y->saving ? -1 : 0;
}

static void print_name(const loimi_cand *c, const char **names) {
    for (int j = 0; j < c->len; j++) fprintf(stderr, "%s%s", names[c->ops[j]], j + 1 < c->len ? "_" : "");
}

static void print_super(const loimi_cand *c, const char **names, uint64_t total, const char *lead) {
    fprintf(stderr, "%ssuper ", lead);
    print_name(c, names);
    fprintf(stderr, " =");
    for (int j = 0; j < c->len; j++) fprintf(stderr, " %s;", names[c->ops[j]]);
    fprintf(stderr, "    // %llu, %.1f%%, saves %llu dispatches", (unsigned long long)c->count, c->count * 100.0 / total,
            (unsigned long long)c->saving);
}

// An accumulator variant (`language.md` 7.4): a name ending in `__`
// and letters A and R, at least one A. Its length without them in *base.
static int acc_variant(const char *name, size_t *base) {
    size_t n = strlen(name), i = n, a = 0;
    while (i > 0 && (name[i - 1] == 'A' || name[i - 1] == 'R')) a += name[--i] == 'A';
    if (a == 0 || i == n || i < 3 || name[i - 1] != '_' || name[i - 2] != '_') return 0;
    if (base) *base = i - 2;
    return 1;
}

// For a suggestion over accumulator variants: the existing super over the
// same ops without the variants (which the variants split up), if any.
static int64_t split_super(const loimi_cand *c, const char **names, const loimi_cand *exist, uint64_t nexist) {
    int any = 0;
    for (int j = 0; j < c->len; j++) any |= acc_variant(names[c->ops[j]], 0);
    if (!any) return -2;
    for (uint64_t k = 0; k < nexist; k++) {
        if (exist[k].len != c->len) continue;
        int eq = 1;
        for (int j = 0; j < c->len && eq; j++) {
            const char *a = names[c->ops[j]], *b = names[exist[k].ops[j]];
            size_t na = strlen(a);
            acc_variant(a, &na);
            eq = strlen(b) == na && strncmp(a, b, na) == 0;
        }
        if (eq) return (int64_t)k;
    }
    return -1;
}

// The suggestions that end the report, as one list that can be pasted
// whole. Candidates are adjacent pairs of plain ops (no supers) that run
// at least 1% of the time, and sequences of 3 or 4 that do and are
// *dominant*: at least half the runs of their shorter prefix and of
// their shorter suffix continue into them (a super for one saves
// len - 1 dispatches per run, where pairs over the same ops save at most
// len / 2). They are taken by dispatches saved, skipping any that
// overlaps one already taken or a super the vm already has: `optimize`
// could fuse only one of them where they meet, and the `super_overlap`
// lint would warn. The skipped ones follow, commented out, each with
// what it overlaps: alternatives, not additions.
//
// `pairs[a * nops + b]` counts op a directly followed by op b; `supers`
// lists the vm's supers as (number of parts, part opcodes...).
__attribute__((weak, visibility("hidden"))) void loimi_stats_suggest(loimi_seq *s, const uint64_t *pairs, uint64_t nops,
                                                                     const char **names, const uint8_t *plain,
                                                                     uint64_t total, const uint32_t *supers,
                                                                     uint64_t nsuper_words) {
    if (total == 0) return;
    uint64_t cap = 1024, n = 0, nexist = 0;
    loimi_cand *cand = calloc(cap, sizeof(*cand));
    loimi_cand *exist = calloc(nsuper_words + 1, sizeof(*exist));
    if (!cand || !exist) return;
    for (uint64_t w = 0; w < nsuper_words; w += 1 + supers[w]) {
        loimi_cand *e = &exist[nexist++];
        e->len = supers[w] < 8 ? (int)supers[w] : 8;
        for (int j = 0; j < e->len; j++) e->ops[j] = supers[w + 1 + j];
    }
    for (uint64_t a = 0; a < nops; a++) {
        for (uint64_t b = 0; b < nops && n < cap; b++) {
            uint64_t c = pairs[a * nops + b];
            if (c == 0 || c * 100 < total || !plain[a] || !plain[b]) continue;
            cand[n++] = (loimi_cand){ .ops = { (uint32_t)a, (uint32_t)b }, .len = 2, .count = c, .saving = c };
        }
    }
    for (uint64_t i = 0; s->tab && i < SEQ_CAP && n < cap; i++) {
        loimi_seq_entry *e = &s->tab[i];
        if (e->len < 3 || e->count * 100 < total) continue;
        int ok = 1;
        for (uint32_t j = 0; j < e->len; j++) ok &= plain[(e->key >> (16 * j)) & 0xffff];
        if (!ok) continue;
        uint64_t mask = (1ull << (16 * (e->len - 1))) - 1;
        uint64_t pre = seq_count(s, e->key >> 16, e->len - 1), suf = seq_count(s, e->key & mask, e->len - 1);
        if (e->count * 2 < pre || e->count * 2 < suf) continue;
        loimi_cand *c = &cand[n++];
        c->len = (int)e->len;
        for (int j = 0; j < c->len; j++) c->ops[j] = (e->key >> (16 * (c->len - 1 - j))) & 0xffff;
        c->count = e->count;
        c->saving = e->count * (e->len - 1);
    }
    qsort(cand, n, sizeof(*cand), by_saving);
    // `clash`: -1 if suggested, k < n if it overlaps suggestion k, and
    // n + j if it overlaps existing super j.
    uint64_t ntaken = 0;
    for (uint64_t k = 0; k < n; k++) {
        cand[k].clash = -1;
        for (uint64_t j = 0; j < nexist && cand[k].clash < 0; j++)
            if (cand_overlap(&exist[j], &cand[k])) cand[k].clash = (int64_t)(n + j);
        for (uint64_t j = 0; j < k && cand[k].clash < 0; j++)
            if (cand[j].clash < 0 && cand_overlap(&cand[j], &cand[k])) cand[k].clash = (int64_t)j;
        ntaken += cand[k].clash < 0;
    }
    if (ntaken) {
        fprintf(stderr, "\nsuggested superinstructions (each at least 1%% of executed ops; none overlaps another or an\n"
                        "existing super, so they can all be added together):\n");
        int over_acc = 0;
        for (uint64_t k = 0; k < n; k++) {
            if (cand[k].clash >= 0) continue;
            print_super(&cand[k], names, total, "    ");
            int64_t sp = split_super(&cand[k], names, exist, nexist);
            over_acc |= sp >= -1;
            if (sp >= 0) {
                fprintf(stderr, "; the variants split ");
                print_name(&exist[sp], names);
            }
            fprintf(stderr, "\n");
        }
        if (over_acc)
            fprintf(stderr, "(supers over accumulator variants: a variant breaks up the supers over its op, and gains\n"
                            "little where it does; declare these `#[forward]`, docs/language.md section 7.4)\n");
    }
    if (ntaken < n) {
        fprintf(stderr, "\nalso frequent, but each overlaps one of the above or an existing super (an alternative to\n"
                        "it, not an addition):\n");
        for (uint64_t k = 0; k < n; k++) {
            if (cand[k].clash < 0) continue;
            print_super(&cand[k], names, total, "//  ");
            int existing = cand[k].clash >= (int64_t)n;
            fprintf(stderr, "; overlaps %s", existing ? "existing " : "");
            print_name(existing ? &exist[cand[k].clash - n] : &cand[cand[k].clash], names);
            fprintf(stderr, "\n");
        }
    }
    free(cand);
    free(exist);
}
