/* Calls every function of tests/embed/api.lm through the generated
 * header. Expected output: host.expected. */
#include <stdio.h>
#include "api.h"

static int32_t inc(int32_t x) { return x + 1; }

int main(void) {
    Counter *c = counter_new();
    printf("step %lld\n", (long long)counter_step(c, 0, 150));
    printf("step %lld\n", (long long)counter_step(c, 1, 20));
    printf("or_null %lld %lld\n", (long long)counter_or_null(c), (long long)counter_or_null(NULL));
    printf("is_big %d %d\n", is_big(c, 100), is_big(c, 200));
    printf("negate %d %d\n", negate(true), negate(false));
    counter_free(c);
    printf("widths %lld\n", (long long)widths(-1, 255, -300, 65535, -70000, 4000000000u, -5, 6));
    printf("floats %g\n", floats(1.5f, 4.0));
    int32_t sq[5];
    fill(sq, 5);
    printf("fill %d %d %d %d %d\n", sq[0], sq[1], sq[2], sq[3], sq[4]);
    char *word = "xyz";
    printf("first_byte %c\n", first_byte((void **)&word));
    loimi_str s = longer(LOIMI_STR("short"), LOIMI_STR("longer one"));
    printf("longer %.*s\n", (int)s.len, s.ptr);
    printf("str_len %llu\n", (unsigned long long)str_len((loimi_str){ "a\0b", 3 }));
    printf("apply_twice %d\n", apply_twice(inc, 40));
    printf("arg_count %llu\n", (unsigned long long)arg_count());
    printf("protected %d %d\n", protected_depth(7, false), protected_depth(7, true));
    printf("protected again %d\n", protected_depth(3, false));
    printf("consts %u %u %d %llu %d %s %d\n", API_FLAG_A, API_FLAG_B, API_MIN, (unsigned long long)API_BIG, API_ON, API_NAME,
           API_SUB);
    const char *bytes = "\x01\x02";
    const api_point *pt = point_new(10, 20);
    printf("point_sum %lld\n", (long long)point_sum(pt, bytes, 2));
    return 0;
}
