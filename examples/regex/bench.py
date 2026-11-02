#!/usr/bin/env python3
"""The benchmarks of `rxhost bench`, timed with Python's `re` (on bytes).
Same text generator and patterns as host.c; best of 3."""
import re
import sys
import time

WORDS = [b"the", b"quick", b"brown", b"fox", b"jumps", b"over", b"lazy", b"dog", b"regex", b"loimi",
         b"bytecode", b"machine", b"Sherlock", b"Holmes", b"Watson", b"1887", b"42", b"x9", b"foo_bar", b"end"]
M = (1 << 64) - 1


def gen_text(n):
    out = bytearray()
    s = 12345
    while len(out) < n:
        s = (s * 6364136223846793005 + 1442695040888963407) & M
        r = s >> 33
        w = WORDS[r % 20]
        if len(out) + len(w) + 1 > n:
            break
        out += w
        out.append(10 if (r >> 8) % 13 == 0 else 32)
    out += b" " * (n - len(out))
    return bytes(out)


BENCHES = [
    ("literal", rb"Sherlock Holmes", 0),
    ("literal-alt", rb"Sherlock|Watson|Holmes|1887", 0),
    ("class", rb"[a-z]+ing|\d{3,}", 0),
    ("words", rb"\w+", 0),
    ("capture", rb"(\w+)\s+(\w+)", 0),
    ("alt-heavy", rb"(?:fox|dog|cat|bird|lazy)\s(?:over|jumps|end)", 0),
    ("dot-star", rb"x9.*end", 0),
    ("pathological", rb"(a*)*b", 1),
    ("patho-alt", rb"(a|aa)+$", 2),
]

n = int(sys.argv[1]) if len(sys.argv) > 1 else 4 << 20
k = int(sys.argv[2]) if len(sys.argv) > 2 else 22
text = gen_text(n)
path = b"a" * k
path2 = b"a" * 28 + b"b"
print("%-14s %10s %10s" % ("bench", "matches", "python-re"))
for name, pat, which in BENCHES:
    t = path if which == 1 else path2 if which == 2 else text
    r = re.compile(pat)
    best = 1e9
    for _ in range(3):
        t0 = time.perf_counter()
        c = sum(1 for _ in r.finditer(t))
        best = min(best, time.perf_counter() - t0)
    print("%-14s %10d %9.2fms" % (name, c, best * 1e3))
