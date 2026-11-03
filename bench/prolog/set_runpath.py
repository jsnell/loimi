#!/usr/bin/env python3
"""Rewrite the RUNPATH (or RPATH) of 64-bit little-endian ELF files in place.

Usage: set_runpath.py NEW_RUNPATH FILE...

SWI-Prolog's CMake build installs swipl and libswipl.so with RUNPATH set
to the absolute library directory of the install, so an install that is
moved (e.g. built in a git worktree and moved to the main checkout) no
longer starts. install.sh calls this after `make install` to make it
`$ORIGIN`-relative. The host has neither patchelf nor chrpath, so this
overwrites the string in .dynstr; the new value must not be longer than
the old one (the rest is filled with NULs). Files without a RUNPATH or
RPATH are left alone.
"""
import struct, sys

DT_NULL, DT_STRTAB, DT_RPATH, DT_RUNPATH = 0, 5, 15, 29
PT_LOAD, PT_DYNAMIC = 1, 2


def set_runpath(path, new):
    with open(path, "r+b") as f:
        b = bytearray(f.read())
        if b[:4] != b"\x7fELF" or b[4] != 2 or b[5] != 1:
            sys.exit(f"{path}: not a 64-bit little-endian ELF file")
        phoff, = struct.unpack_from("<Q", b, 0x20)
        phentsize, phnum = struct.unpack_from("<HH", b, 0x36)
        loads, dyn = [], None
        for i in range(phnum):
            p_type, _, p_offset, p_vaddr, _, p_filesz, _, _ = struct.unpack_from(
                "<IIQQQQQQ", b, phoff + i * phentsize)
            if p_type == PT_LOAD:
                loads.append((p_vaddr, p_offset, p_filesz))
            elif p_type == PT_DYNAMIC:
                dyn = (p_offset, p_filesz)
        if dyn is None:
            sys.exit(f"{path}: no dynamic section")

        def off(vaddr):
            for va, o, sz in loads:
                if va <= vaddr < va + sz:
                    return o + vaddr - va
            sys.exit(f"{path}: address {vaddr:#x} not in a loaded segment")

        strtab, rp = None, []
        for i in range(dyn[1] // 16):
            tag, val = struct.unpack_from("<qQ", b, dyn[0] + 16 * i)
            if tag == DT_NULL:
                break
            if tag == DT_STRTAB:
                strtab = val
            elif tag in (DT_RPATH, DT_RUNPATH):
                rp.append(val)
        if not rp:
            print(f"{path}: no RUNPATH")
            return
        for v in rp:
            at = off(strtab) + v
            end = b.index(0, at)
            old = bytes(b[at:end])
            enc = new.encode()
            if len(enc) > len(old):
                sys.exit(f"{path}: new RUNPATH longer than the old one ({old.decode()})")
            b[at:end] = enc + b"\0" * (len(old) - len(enc))
            print(f"{path}: {old.decode()} -> {new}")
        f.seek(0)
        f.write(b)


if __name__ == "__main__":
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    for p in sys.argv[2:]:
        set_runpath(p, sys.argv[1])
