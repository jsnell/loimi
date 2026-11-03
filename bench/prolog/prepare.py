#!/usr/bin/env python3
"""Make standalone benchmark programs from SWI-Prolog's bench suite.

Usage: prepare.py SRC_PROGRAMS_DIR DST_DIR

Copies the programs listed in PROGRAMS from the pinned SWI-Prolog bench
tarball (see MANIFEST) into DST_DIR/NAME.pl, applying only the mechanical
edits in EDITS below (also listed in README.md). Every edit is checked to
apply exactly as often as expected, so a changed upstream file fails loudly
instead of being silently mis-edited.
"""
import re
import sys
from pathlib import Path

# The suite's programs that run on plain ISO-ish Prolog. Left out: fib,
# pingpong, moded_path (tabling), queens_clpfd (library(clpfd)), det
# (SWI-only single-sided unification `=>`).
PROGRAMS = """
boyer browse chat_parser crypt derive divide10 eval fast_mu flatten log10
meta_qsort mu nand nreverse ops8 perfect poly_10 prover qsort queens_8
query reducer sendmore serialise sieve simple_analyzer tak times10 unify
zebra
""".split()

# name -> list of (regex, replacement, expected number of matches).
MODE = (r"(?m)^:- mode\([^\n]*\)\.\n", "", 1)
EDITS = {
    # `:- mode(...)` is a DEC-10 declaration, not ISO; it has no effect
    # on the program's meaning.
    "eval": [MODE],
    "log10": [MODE],
    "mu": [MODE],
    "nand": [MODE],
}


def main():
    src, dst = Path(sys.argv[1]), Path(sys.argv[2])
    dst.mkdir(parents=True, exist_ok=True)
    for name in PROGRAMS:
        text = (src / f"{name}.pl").read_text()
        for pat, rep, count in EDITS.get(name, []):
            text, n = re.subn(pat, rep, text)
            if n != count:
                sys.exit(f"prepare.py: {name}: {pat!r} matched {n}x, expected {count}")
        header = (f"% {name}.pl from https://github.com/SWI-Prolog/bench "
                  f"programs/{name}.pl\n% (commit in bench/prolog/MANIFEST); "
                  f"edits: {'see bench/prolog/README.md' if name in EDITS else 'none'}.\n")
        (dst / f"{name}.pl").write_text(header + text)
    print(f"prepare.py: {len(PROGRAMS)} programs in {dst}")


if __name__ == "__main__":
    main()
