#!/usr/bin/env python3
"""Generate regex test cases with Python's `re` (on bytes) as the oracle.

    gen.py OUT_CASES OUT_EXPECTED [--random N] [--seed S]

Each case line is `MODE HEXPATTERN HEXSUBJECT` (MODE `search` or `match`,
`-` for an empty string); the expected line is the result: `ERR` (the
pattern does not compile), `NOMATCH`, or the spans of group 0..n as
`start,end` (-1,-1 for a group that did not take part).

Hand-written cases come from cases.txt (one `MODE<TAB>PATTERN<TAB>SUBJECT`
per line, with Python escapes in SUBJECT). Random cases: patterns over a
small alphabet built from the supported syntax, subjects over `abc\\n`,
from a fixed seed, sizes capped.
"""
import random
import re
import sys
import warnings

warnings.simplefilter("ignore")


def hx(b):
    return b.hex() if b else "-"


def expect(mode, pat, subj):
    try:
        r = re.compile(pat)
    except (re.error, OverflowError, RecursionError):
        return "ERR"
    m = r.search(subj) if mode == "search" else r.match(subj)
    if not m:
        return "NOMATCH"
    return " ".join("%d,%d" % m.span(g) for g in range(r.groups + 1))


ATOMS = [b"a", b"b", b"c", b".", b"[ab]", b"[^a]", b"[a-c]", b"\\w", b"\\d", b"\\s", b"\\n", b"^", b"$", b"x"]
QUANTS = [b"", b"", b"", b"*", b"+", b"?", b"*?", b"+?", b"??", b"{2}", b"{1,3}", b"{0,2}", b"{2,}", b"{1,2}?"]


def rand_pat(rng, depth=0):
    n = rng.randint(1, 4)
    parts = []
    for _ in range(n):
        if depth < 3 and rng.random() < 0.3:
            inner = rand_pat(rng, depth + 1)
            if rng.random() < 0.3:
                inner = inner + b"|" + rand_pat(rng, depth + 1)
            atom = (b"(?:" if rng.random() < 0.25 else b"(") + inner + b")"
        else:
            atom = rng.choice(ATOMS)
        q = rng.choice(QUANTS) if atom not in (b"^", b"$") else b""
        parts.append(atom + q)
    p = b"".join(parts)
    if depth == 0 and rng.random() < 0.2:
        p += b"|" + rand_pat(rng, 1)
    return p


def rand_subj(rng):
    n = rng.randint(0, 12)
    return bytes(rng.choice(b"aaabbc\n1 ") for _ in range(n))


def unescape(s):
    return s.encode("latin-1").decode("unicode_escape").encode("latin-1")


def main():
    out_cases, out_exp = sys.argv[1], sys.argv[2]
    nrand = 2000
    seed = 1
    args = sys.argv[3:]
    if "--random" in args:
        nrand = int(args[args.index("--random") + 1])
    if "--seed" in args:
        seed = int(args[args.index("--seed") + 1])
    cases = []
    here = __file__.rsplit("/", 1)[0]
    with open(here + "/cases.txt", encoding="latin-1") as f:
        for line in f:
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            mode, pat, subj = (line.split("\t") + [""])[:3]
            cases.append((mode, pat.encode("latin-1"), unescape(subj)))
    rng = random.Random(seed)
    for _ in range(nrand):
        cases.append((rng.choice(["search", "search", "match"]), rand_pat(rng), rand_subj(rng)))
    with open(out_cases, "w") as c, open(out_exp, "w") as e:
        for mode, pat, subj in cases:
            c.write("%s %s %s\n" % (mode, hx(pat), hx(subj)))
            e.write(expect(mode, pat, subj) + "\n")


main()
