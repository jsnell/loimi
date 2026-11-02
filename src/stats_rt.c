// `--vm-stats` runtime: longer op sequences. The generated code counts
// ops and adjacent pairs itself; for sequences of 3 and 4 ops it calls
// `loimi_stats_seq` on every op, and the report calls
// `loimi_stats_seq_report` after the pairs.
//
// Each vm has one `loimi_seq`, zero-initialized: the last three ops (the
// window restarts when an op does not directly follow the previous one
// in the code) and a hash table of the sequences of 2 to 4 ops ending at
// each op, with their counts.
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

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

void loimi_stats_seq(loimi_seq *s, int64_t adjacent, int64_t op) {
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

static int by_saving(const void *a, const void *b) {
    const loimi_seq_entry *x = *(loimi_seq_entry *const *)a, *y = *(loimi_seq_entry *const *)b;
    uint64_t sx = x->count * (x->len - 1), sy = y->count * (y->len - 1);
    return sx < sy ? 1 : sx > sy ? -1 : 0;
}

// The ops of an entry, first op first.
static int seq_ops(const loimi_seq_entry *e, uint32_t *out) {
    for (uint32_t j = 0; j < e->len; j++) out[j] = (e->key >> (16 * (e->len - 1 - j))) & 0xffff;
    return (int)e->len;
}

// Whether `optimize` could not fuse both in some sequence: a proper
// suffix of one is a prefix of the other, or one contains the other (as
// the `super_overlap` lint).
static int seq_overlap(const loimi_seq_entry *x, const loimi_seq_entry *y) {
    uint32_t a[4], b[4];
    int na = seq_ops(x, a), nb = seq_ops(y, b);
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
        uint32_t t[4];
        for (int j = 0; j < 4; j++) { t[j] = a[j]; a[j] = b[j]; b[j] = t[j]; }
        int tn = na; na = nb; nb = tn;
    }
    return 0;
}

// Candidates are sequences of 3 or 4 plain ops (no supers) that run at
// least 1% of the time and are *dominant*: at least half the runs of
// their shorter prefix and of their shorter suffix continue into them.
// A super for one saves len - 1 dispatches per run, where pairs over the
// same ops save at most len / 2. They are taken by saving, skipping any
// that overlaps one already taken (`optimize` could fuse only one of
// them where they meet; see the `super_overlap` lint).
void loimi_stats_seq_report(loimi_seq *s, const char **names, const uint8_t *plain, uint64_t total) {
    if (!s->tab || total == 0) return;
    uint64_t cap = 256, n = 0;
    loimi_seq_entry **cand = malloc(sizeof(*cand) * cap);
    if (!cand) return;
    for (uint64_t i = 0; i < SEQ_CAP; i++) {
        loimi_seq_entry *e = &s->tab[i];
        if (e->len < 3 || e->count * 100 < total) continue;
        int ok = 1;
        for (uint32_t j = 0; j < e->len; j++) ok &= plain[(e->key >> (16 * j)) & 0xffff];
        if (!ok) continue;
        uint64_t mask = (1ull << (16 * (e->len - 1))) - 1;
        uint64_t pre = seq_count(s, e->key >> 16, e->len - 1), suf = seq_count(s, e->key & mask, e->len - 1);
        if (e->count * 2 < pre || e->count * 2 < suf) continue;
        if (n == cap) break;
        cand[n++] = e;
    }
    qsort(cand, n, sizeof(*cand), by_saving);
    uint64_t picked = 0;
    for (uint64_t k = 0; k < n; k++) {
        int clash = 0;
        for (uint64_t j = 0; j < picked && !clash; j++) clash = seq_overlap(cand[j], cand[k]);
        if (!clash) cand[picked++] = cand[k];
    }
    if (picked) {
        fprintf(stderr, "\nfrequent longer sequences (at least 1%% of executed ops, and most runs of their parts), as superinstructions:\n");
    }
    for (uint64_t k = 0; k < picked; k++) {
        loimi_seq_entry *e = cand[k];
        uint32_t ops[4];
        int len = seq_ops(e, ops);
        fprintf(stderr, "    super ");
        for (int j = 0; j < len; j++) fprintf(stderr, "%s%s", names[ops[j]], j + 1 < len ? "_" : "");
        fprintf(stderr, " =");
        for (int j = 0; j < len; j++) fprintf(stderr, " %s;", names[ops[j]]);
        fprintf(stderr, "    // %llu, %.1f%%, saves %llu dispatches\n", (unsigned long long)e->count, e->count * 100.0 / total,
                (unsigned long long)(e->count * (e->len - 1)));
    }
    free(cand);
}
