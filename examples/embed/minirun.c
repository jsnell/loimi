/* The smallest Mini host: run a script file through the embedding API.
 * Used to measure what library builds cost against the standalone
 * executable (docs/lowering.md §4).
 *
 *   loimi build examples/mini/embed.lm --release --shared -o libmini.so --header mini.h
 *   clang-19 -O2 -I. examples/embed/minirun.c -L. -lmini -Wl,-rpath,. -o minirun
 *   ./minirun bench/mini/programs/fib.lua
 */
#include <stdio.h>
#include <stdlib.h>

#include "mini.h"

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: minirun <script>\n");
        return 2;
    }
    FILE *f = fopen(argv[1], "rb");
    if (!f) {
        perror(argv[1]);
        return 1;
    }
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *src = malloc(n > 0 ? n : 1);
    if (fread(src, 1, n, f) != (size_t)n) {
        perror(argv[1]);
        return 1;
    }
    fclose(f);
    MiniState *S = mini_new();
    MiniStatus st = mini_dostring(S, src, n);
    fflush(stdout);
    if (st != MiniStatus_Ok) {
        loimi_str e = mini_error(S);
        fprintf(stderr, "error: %.*s\n", (int)e.len, e.ptr);
    }
    mini_free(S);
    free(src);
    return st == MiniStatus_Ok ? 0 : 1;
}
