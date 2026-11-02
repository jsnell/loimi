/* Two `--vm-stats` libraries in one host: examples/regex/lib.lm and
 * tests/embed/panic.lm, linked whole-archive. Built by the
 * `vm_stats_library` test in golden.rs. */
#include <stdio.h>
#include "rx.h"
#include "panic.h"

static int64_t cb(RCtx *c, int64_t a) { return rvm_call(c, 2, a); }

int main(void) {
    rx_regex *r = rx_compile("a+b", 3);
    int64_t spans[2];
    printf("rx %d\n", rx_exec(r, "xaab", 4, 0, RX_SEARCH, spans, 1));
    rx_free(r);
    World *w = world_new();
    RCtx *c = rvm_new(w, cb);
    printf("rvm %lld\n", (long long)rvm_run(c, 0, 1));
    rvm_free(c);
    world_free(w);
    return 0;
}
