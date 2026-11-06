#!/usr/bin/env python3
"""Trace-tier experiment: stitch a loop function from a vm's JIT stencils.

Inputs
  --stencils  the stencil module (LOIMI_KEEP_STENCILS=dir of a --jit-stencils build)
  --main      the program IR of the same build (--keep-ll), for the names it defines
  --dump      the recording (LOIMI_TRACE_DUMP=file): code words, ops, patched constants
  --mode      region: every instruction of the loop, all internal edges, block counters
              trace:  one path through the loop (from --profile), off-trace code exits
Output: IR to append to the program (LOIMI_TRACE_IR=file).

The generated function has a handler's signature (ctx, pc, regs, k, fp, insn).
Per step (instruction) it copies the stencil body with
  - operand fields, instruction words and constants as literals;
  - `next`/label continuations to the next step as branches, others as exits
    through the dispatch table;
  - `regs` replaced by a local copy of the register window (an alloca SROA
    promotes): copied in at entry, written back before every exit and every
    call that may see the register file, reloaded after such a call.
"""
import argparse, json, re, sys
from collections import defaultdict

ap = argparse.ArgumentParser()
ap.add_argument("--stencils", required=True)
ap.add_argument("--main", required=True)
ap.add_argument("--dump", required=True)
ap.add_argument("--mode", choices=["region", "trace"], default="region")
ap.add_argument("--profile", help="LOIMI_TRACE_PROFILE output of a region build (trace mode)")
ap.add_argument("--map", help="block map json of the region build (trace mode)")
ap.add_argument("--name", default="loop")
ap.add_argument("--header", type=int, help="loop header pc (default: the dump's hot pc)")
ap.add_argument("--code", type=int, default=-1, help="which compile of the dump (default: last with a hot pc)")
ap.add_argument("--counters", action="store_true", help="count every block (region mode: on)")
ap.add_argument("--guards", choices=["none", "reexec"], default="reexec",
                help="trace mode: untaken branches of side-effect-free steps exit and re-execute the instruction")
ap.add_argument("--no-promote", action="store_true", help="keep regs in memory (no local window)")
ap.add_argument("--blacklist", type=int, default=1000, help="drop the trace after N guard failures (0: never)")
ap.add_argument("--peel", action="store_true", help="trace mode: emit the trace twice, preamble and loop")
ap.add_argument("--nocount", action="store_true", help="region mode without block counters")
ap.add_argument("--no-inline-cold", dest="inline_cold", action="store_false", help="leave outlined cold blocks as exits")
ap.add_argument("--out", required=True)
ap.add_argument("--mapout", help="write the block map (json)")
a = ap.parse_args()

# ---------------------------------------------------------------- recording
codes = []
for line in open(a.dump):
    f = line.split()
    if f[0] == "code":
        codes.append({"n": int(f[2]), "hash": int(f[3], 16), "hot": int(f[5]), "w": {}, "op": {}, "k": defaultdict(list)})
    elif f[0] == "w":
        c = codes[-1]
        c["w"][int(f[1])] = int(f[2], 16)
        if f[3] != "-":
            c["op"][int(f[1])] = f[3]
    elif f[0] == "k":
        codes[-1]["k"][(int(f[1]), f[2])].append(int(f[3], 16))
code = [c for c in codes if c["hot"] >= 0][a.code] if a.code != -1 else [c for c in codes if c["hot"] >= 0][-1]
H = a.header if a.header is not None else code["hot"]
W = code["w"]

# ---------------------------------------------------------------- stencil module
TYPES = set()
fn_lines = {}
fn_head = {}
decls = {}
cur = None
for line in open(a.stencils):
    line = line.rstrip("\n")
    if cur is not None:
        if line == "}":
            fn_lines[cur] = body
            cur = None
        else:
            body.append(line)
        continue
    if line.startswith("define "):
        m = re.search(r'@("[^"]+"|[-\w$.]+)\(', line)
        cur = m.group(1)
        fn_head[cur] = line
        body = []
    elif line.startswith("%") and " = type " in line:
        TYPES.add(line.split(" = type ")[0][1:])
    elif line.startswith("declare "):
        m = re.search(r'@("[^"]+"|[-\w$.]+)\(', line)
        decls[m.group(1)] = line
main_defs = set()
main_decls = set()
for line in open(a.main):
    if line.startswith("define ") or line.startswith("declare "):
        m = re.search(r'@("[^"]+"|[-\w$.]+)\(', line)
        (main_defs if line.startswith("define") else main_decls).add(m.group(1))
    elif line.startswith("@"):
        m = re.match(r'@("[^"]+"|[-\w$.]+) =', line)
        if m:
            main_defs.add(m.group(1))


def field(w, shift, bits, signed):
    u = (w >> shift) & ((1 << bits) - 1)
    if signed and u >> (bits - 1):
        u -= 1 << bits
    return u


def stencil_of(pc):
    return '"__jit.op.%s"' % code["op"][pc]


CALL_RE = re.compile(r'call [^@%]*?(?:@("[^"]+"|[-\w$.]+)|(%[-\w$.]+))\(')
safe_memo = {}


def safe_fn(name, depth=0):
    """A callee that cannot see the register file or run the collector:
    a body with no stores outside its own allocas and calls only to such
    functions (or LLVM intrinsics)."""
    if name.startswith('llvm.') or name.startswith('"llvm.'):
        return True
    if name in safe_memo:
        return safe_memo[name]
    safe_memo[name] = False
    body = fn_lines.get(name)
    if body is None or depth > 8:
        return False
    allocas = set(m.group(1) for l in body for m in [re.match(r'\s*(%[-\w$.]+) = alloca', l)] if m)
    ok = True
    for l in body:
        s = l.strip()
        m = re.match(r'store [^,]+, ptr (%[-\w$.]+)', s)
        if m and m.group(1) not in allocas:
            ok = False
        if s.startswith("store") and not m:
            ok = False
        if "atomicrmw" in s or "cmpxchg" in s or " fence " in s:
            ok = False
        cm = CALL_RE.search(s)
        if cm and "asm" not in s:
            callee = cm.group(1)
            if callee is None or not safe_fn(callee, depth + 1):
                ok = False
    safe_memo[name] = ok
    return ok


# ---------------------------------------------------------------- continuations
CONT_RE = re.compile(r'musttail call preserve_nonecc void @"__jit\.(n|j)\.([0-9.]+)"\((.*)\)')


def conts(pc):
    """Targets of the stencil's continuations (pc offsets)."""
    out = []
    for l in fn_lines[stencil_of(pc)]:
        m = CONT_RE.search(l)
        if m:
            out.append(cont_target(pc, m.group(1), m.group(2)))
    return out


def cont_target(pc, kind, spec):
    if kind == "n":
        return pc + int(spec)
    o, s, b = map(int, spec.split("."))
    return pc + o + field(W[pc + o], s, b, True)


# the loop: from H, every instruction up to the last that jumps back to H
heads = sorted(code["op"])
back = [pc for pc in heads if pc >= H and H in conts(pc)]
if not back:
    sys.exit("no back edge to %d" % H)
B = max(back)
region = []
seen = set()
work = [H]
while work:
    pc = work.pop()
    if pc in seen or pc not in code["op"] or not (H <= pc <= B):
        continue
    seen.add(pc)
    work.extend(conts(pc))
# continuations through the table (inlined cold blocks) are not static:
# the region is every instruction of the loop's range
seen |= set(pc for pc in code["op"] if H <= pc <= B)
region = sorted(seen)

prof = None
if a.mode == "trace":
    bmap = json.load(open(a.map))
    counts = {}
    for line in open(a.profile):
        f = line.split()
        if f[0] != "trace":
            counts[int(f[0])] = int(f[1])
    prof = {}  # (pc, label) -> count
    for k, idx in bmap["blocks"].items():
        pc, lab = k.split(":", 1)
        prof[(int(pc), lab)] = counts.get(idx, 0)
    # the hottest path from H back to H, by the counts of the blocks
    # holding each internal continuation
    edges = defaultdict(int)
    for e in bmap["edges"]:
        edges[(e[0], e[2])] += counts.get(e[1], 0)
    steps = [H]
    while True:
        cands = [(n, t) for (f, t), n in edges.items() if f == steps[-1]]
        if not cands:
            sys.exit("trace: no successor of %d" % steps[-1])
        n, t = max(cands)
        if t == H:
            break
        if t in steps:
            sys.exit("trace: inner cycle at %d" % t)
        steps.append(t)
    succ = {steps[i]: steps[(i + 1) % len(steps)] for i in range(len(steps))}
else:
    steps = region
    succ = None

# register window: the largest unsigned 8-bit field of the steps' words, + 4 (forbase)
NR = 0
for pc in steps:
    for l in fn_lines[stencil_of(pc)]:
        for m in re.finditer(r'@"__jit\.f\.(\d+)\.(\d+)\.(\d+)\.u"', l):
            o, s, b = map(int, m.groups())
            NR = max(NR, field(W[pc + o], s, b, False) + 4)
print("steps:", [(pc, code["op"][pc]) for pc in steps], "window:", NR, file=sys.stderr)

# ---------------------------------------------------------------- emission
out = []
allocas = []
heads_in = defaultdict(list)  # step pc -> [(block, ctx, k, fp)]
blockmap = {}
edges_out = []
used_globals = set()
ctr = [0]


def fresh(p):
    ctr[0] += 1
    return "%%x%d.%s" % (ctr[0], p)


def writeback(emit, src="%lregs", dst="%tr.regs"):
    if a.no_promote:
        return
    for j in range(NR):
        x = fresh("wb")
        y = fresh("wv")
        z = fresh("wd")
        emit("  %s = getelementptr inbounds i64, ptr %s, i64 %d" % (x, src, j))
        emit("  %s = load i64, ptr %s" % (y, x))
        emit("  %s = getelementptr inbounds i64, ptr %s, i64 %d" % (z, dst, j))
        emit("  store i64 %s, ptr %s" % (y, z))


def reload(emit):
    writeback(emit, "%tr.regs", "%lregs")


def counter(emit, idx):
    x = fresh("cp")
    y = fresh("cv")
    z = fresh("cn")
    emit('  %s = getelementptr inbounds i64, ptr @"loimi.trace.counts.%s", i64 %d' % (x, a.name, idx))
    emit("  %s = load i64, ptr %s" % (y, x))
    emit("  %s = add i64 %s, 1" % (z, y))
    emit("  store i64 %s, ptr %s" % (z, x))


def dispatch(emit, ctx, pcv, k, fp):
    w = fresh("w")
    o = fresh("o")
    oz = fresh("oz")
    g = fresh("g")
    h = fresh("h")
    emit("  %s = load i32, ptr %s" % (w, pcv))
    emit("  %s = and i32 %s, 255" % (o, w))
    emit("  %s = zext i32 %s to i64" % (oz, o))
    emit('  %s = getelementptr inbounds ptr, ptr @"vm.Mini.table", i64 %s' % (g, oz))
    emit("  %s = load ptr, ptr %s" % (h, g))
    emit("  musttail call preserve_nonecc void %s(ptr %s, ptr %s, ptr %%tr.regs, ptr %s, ptr %s, i32 %s)" % (h, ctx, pcv, k, fp, w))
    emit("  ret void")
    used_globals.add('"vm.Mini.table"')


def i32s(v):
    v &= 0xffffffff
    return v - (1 << 32) if v >> 31 else v


def i64s(v):
    v &= (1 << 64) - 1
    return v - (1 << 64) if v >> 63 else v


nblocks = [0]
nreexec = [0]
refused = []
side_effect_steps = []
heap_consts = []


LOCAL = re.compile(r'%("[^"]*"|[-a-zA-Z$._0-9]+)')
LABEL = re.compile(r'^([-a-zA-Z$._0-9]+):')
MT_RE = re.compile(r'^\s*musttail call preserve_nonecc void (@"[^"]+"|%[-\w$.]+)\((.*)\)\s*$')


def split_args(s):
    out, depth, cur = [], 0, ""
    for ch in s:
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
        if ch == "," and depth == 0:
            out.append(cur.strip())
            cur = ""
        else:
            cur += ch
    if cur.strip():
        out.append(cur.strip())
    return out


def expand_cold(body, depth=0):
    """Inline the step's musttail calls to outlined cold blocks (functions
    `X.coldN` with a handler's signature), once: on the trace they may be
    the hot path (Mini's native calls, sqrt)."""
    if not a.inline_cold:
        return body
    out = []
    tail = []
    n = 0
    skip = False
    for l in body:
        if skip:
            skip = False
            if l.strip() == "ret void":
                continue
        m = MT_RE.match(l)
        if m and re.search(r'\.cold\d+"$', m.group(1)) and m.group(1)[1:] in fn_lines and depth == 0:
            callee = m.group(1)[1:]
            args = [x.split(" ", 1)[1] for x in split_args(m.group(2))]
            n += 1
            C = "c%d." % n
            pmap = {"p0": args[0], "p1": args[1], "p2": args[2], "p3": args[3], "p4": args[4], "insn": args[5]}

            def ren(mm, C=C, pmap=pmap):
                nm = mm.group(1)
                if nm in TYPES:
                    return mm.group(0)
                if nm in pmap:
                    return pmap[nm]
                if nm.startswith('"'):
                    return '%"' + C + nm[1:]
                return "%" + C + nm
            out.append("  br label %%%sentry" % C)
            skip = True
            for cl in fn_lines[callee]:
                lm = LABEL.match(cl)
                if lm:
                    tail.append(C + cl)
                else:
                    tail.append(LOCAL.sub(ren, cl))
            continue
        out.append(l)
    return out + tail


def emit_step(si, pc, cp="s"):
    name = stencil_of(pc)
    body = expand_cold(fn_lines[name])
    P = "%s%d." % (cp, pc)

    def ren(m):
        nm = m.group(1)
        if nm in TYPES:
            return m.group(0)
        if nm == "p0":
            return "%" + P + "ctx"
        if nm == "p1":
            return "%" + P + "pc"
        if nm == "p2":
            return "%tr.regs" if a.no_promote else "%lregs"
        if nm == "p3":
            return "%" + P + "k"
        if nm == "p4":
            return "%" + P + "fp"
        if nm == "insn":
            return str(i32s(W[pc]))
        if nm.startswith('"'):
            return '%"' + P + nm[1:]
        return "%" + P + nm

    def holes(l):
        def fhole(m):
            o, s, b, sg = int(m.group(2)), int(m.group(3)), int(m.group(4)), m.group(5) == "s"
            f = field(W[pc + o], s, b, sg)
            return str(f + (1 << (b - 1)) if sg else f * 8)
        l = re.sub(r'= ptrtoint ptr @"__jit\.f\.((\d+)\.(\d+)\.(\d+)\.([us]))" to (i\d+)',
                   lambda m: "= add %s 0, %s" % (m.group(6), fhole(m)), l)
        l = re.sub(r'ptrtoint \(ptr @"__jit\.f\.((\d+)\.(\d+)\.(\d+)\.([us]))" to i\d+\)', fhole, l)
        l = re.sub(r'= ptrtoint ptr @"__jit\.w\.(\d+)" to (i\d+)', lambda m: "= add %s 0, %d" % (m.group(2), i32s(W[pc + int(m.group(1))])), l)
        l = re.sub(r'ptrtoint \(ptr @"__jit\.w\.(\d+)" to i\d+\)', lambda m: str(i32s(W[pc + int(m.group(1))])), l)

        def chole(m):
            key = (pc, "%s.%s.%s" % (m.group(1), m.group(2), m.group(3)))
            vs = set(code["k"].get(key, []))
            if len(vs) != 1:
                sys.exit("constant %s at %d: %s" % (key, pc, vs))
            v = vs.pop()
            if (v >> 48) >= 0xfffa and (v & 0xffffffffffff) > 0x10000:
                # a heap constant: its address differs between runs, so
                # the trace loads it from the pool (k[field])
                heap_consts.append(pc)
                o, sh, b = int(m.group(1)), int(m.group(2)), int(m.group(3))
                g = fresh("kp")
                return "@@PRE %s = getelementptr inbounds i64, ptr %%%sk, i64 %d@@= load i64, ptr %s" % (g, P, field(W[pc + o], sh, b, False), g)
            return "= add i64 0, %d" % i64s(v)
        l = re.sub(r'= call i64 asm "movabsq [^"]*", "=r,i"\(ptr @"__jit\.c\.(\d+)\.(\d+)\.(\d+)\.u"\)', chole, l)
        if "@\"__jit." in l and "musttail" not in l:
            sys.exit("unhandled hole at %d: %s" % (pc, l))
        m = re.match(r'(.*?)@@PRE (.*?)@@(.*)$', l)
        if m:
            l = "  " + m.group(2) + "\n" + m.group(1) + m.group(3)
        return l

    # blocks and their successors
    blocks = []  # [label, [lines]]
    for l in body:
        m = LABEL.match(l)
        if m:
            blocks.append([m.group(1), []])
        else:
            blocks[-1][1].append(l)
    succs = {}
    for lab, ls in blocks:
        t = ls[-1].strip() if ls else ""
        succs[lab] = re.findall(r'label %("[^"]*"|[-\w$.]+)', t) if t.startswith("br ") else []

    def on(lab):
        if prof is None:
            return True
        return prof.get((pc, lab), 0) > 0

    # register-file pointers: p2, copies of it (exact), offsets from it (derived)
    allocas_l = set()
    exact = {"%p2"}
    derived = set()
    slots = set()
    for _ in range(4):
        for lab, ls in blocks:
            for l in ls:
                s = l.strip()
                m = re.match(r'(%[-\w$.]+) = alloca', s)
                if m:
                    allocas_l.add(m.group(1))
                m = re.match(r'store ptr (%[-\w$.]+), ptr (%[-\w$.]+)', s)
                if m and m.group(1) in exact and m.group(2) in allocas_l:
                    slots.add(m.group(2))
                m = re.match(r'(%[-\w$.]+) = load ptr, ptr (%[-\w$.]+)', s)
                if m and m.group(2) in slots:
                    exact.add(m.group(1))
                m = re.match(r'(%[-\w$.]+) = getelementptr (?:inbounds )?[^,]+, ptr (%[-\w$.]+)', s)
                if m and (m.group(2) in exact or m.group(2) in derived):
                    derived.add(m.group(1))
    regsish = exact | derived

    def has_effect(ls):
        for l in ls:
            s = l.strip()
            m = re.match(r'store [^,]+, ptr (%[-\w$.]+)', s)
            if m and m.group(1) not in allocas_l and m.group(1) not in regsish:
                return True
            if "atomicrmw" in s or "cmpxchg" in s:
                return True
            cm = CALL_RE.search(s)
            if cm and "musttail" not in s and "asm" not in s and not (cm.group(1) and safe_fn(cm.group(1))):
                return True
        return False

    # effects before each on-trace block ends (forward over the on-trace CFG)
    eff_out = {}
    entry = blocks[0][0]
    changed = True
    while changed:
        changed = False
        for lab, ls in blocks:
            if not on(lab):
                continue
            preds = [p for p, ss in succs.items() if lab in ss and on(p)]
            ein = any(eff_out.get(p, False) for p in preds)
            e = ein or has_effect(ls)
            if eff_out.get(lab) != e:
                eff_out[lab] = e
                changed = True
    effect = any(eff_out.values())
    if effect:
        side_effect_steps.append(pc)
    reexec_ok = prof is not None and a.guards == "reexec"
    # guard edges: on-trace A -> off-trace B with no effect before the end of A
    guard_edges = set()
    if reexec_ok:
        for lab, ls in blocks:
            if on(lab) and not eff_out.get(lab, True):
                for b in succs[lab]:
                    if not on(b):
                        guard_edges.add((lab, b))
    # blocks still reachable
    reach = set()
    work = [entry]
    while work:
        b = work.pop()
        if b in reach:
            continue
        reach.add(b)
        work.extend(x for x in succs[b] if (b, x) not in guard_edges)
    need_snap = any(True for _ in guard_edges)

    emit = out.append
    cur = [P + "head"]

    def label(l):
        emit(l + ":")
        cur[0] = l

    def rebase_val(v):
        """A pointer into the local window, as the same slot of the real file."""
        x = fresh("ri"); y = fresh("rl"); d = fresh("rd"); g = fresh("rg"); q = fresh("rin"); r = fresh("rr")
        emit("  %s = ptrtoint ptr %s to i64" % (x, v))
        emit("  %s = ptrtoint ptr %%lregs to i64" % y)
        emit("  %s = sub i64 %s, %s" % (d, x, y))
        emit("  %s = icmp ult i64 %s, %d" % (q, d, 8 * NR))
        emit("  %s = getelementptr i8, ptr %%tr.regs, i64 %s" % (g, d))
        emit("  %s = select i1 %s, ptr %s, ptr %s" % (r, q, g, v))
        return r

    def exit_call(callee, ctx, pcv, C, k, fp, w):
        """Leave: if the regs pointer is still the window, write it back and
        pass the real file; otherwise the code switched to real memory."""
        line = "  musttail call preserve_nonecc void %s(ptr %s, ptr %s, ptr @@R@@, ptr %s, ptr %s, i32 %s)" % (callee, ctx, pcv, k, fp, w)
        if a.no_promote or C in ("%lregs", "%tr.regs"):
            writeback(emit)
            emit(line.replace("@@R@@", "%tr.regs"))
            emit("  ret void")
            return
        q = fresh("ceq")
        lw = fresh("xw")[1:]
        ln = fresh("xn")[1:]
        emit("  %s = icmp eq ptr %s, %%lregs" % (q, C))
        emit("  br i1 %s, label %%%s, label %%%s" % (q, lw, ln))
        label(lw)
        writeback(emit)
        emit(line.replace("@@R@@", "%tr.regs"))
        emit("  ret void")
        label(ln)
        emit(line.replace("@@R@@", C))
        emit("  ret void")

    def table_call(pcv):
        w = fresh("w"); o = fresh("o"); oz = fresh("oz"); g = fresh("g"); h = fresh("h")
        emit("  %s = load i32, ptr %s" % (w, pcv))
        emit("  %s = and i32 %s, 255" % (o, w))
        emit("  %s = zext i32 %s to i64" % (oz, o))
        emit('  %s = getelementptr inbounds ptr, ptr @"vm.Mini.table", i64 %s' % (g, oz))
        emit("  %s = load ptr, ptr %s" % (h, g))
        return h, w

    def goto_head(tl, C, ctx, k, fp, t):
        """Continue at step t if regs is still the window, else exit there."""
        if a.no_promote or C == "%lregs":
            emit("  br label %%%shead" % tl)
            heads_in[tl].append(("%" + cur[0], ctx, k, fp))
            return
        q = fresh("ceq")
        ln = fresh("cn")[1:]
        emit("  %s = icmp eq ptr %s, %%lregs" % (q, C))
        emit("  br i1 %s, label %%%shead, label %%%s" % (q, tl, ln))
        heads_in[tl].append(("%" + cur[0], ctx, k, fp))
        label(ln)
        pcv = fresh("xpc")
        emit("  %s = getelementptr inbounds i32, ptr %%tr.pc, i64 %d" % (pcv, t - H))
        h, w = table_call(pcv)
        emit("  musttail call preserve_nonecc void %s(ptr %s, ptr %s, ptr %s, ptr %s, ptr %s, i32 %s)" % (h, ctx, pcv, C, k, fp, w))
        emit("  ret void")

    label(P + "head")
    emit("@@PHIS " + P)
    emit("  %%%spc = getelementptr inbounds i32, ptr %%tr.pc, i64 %d" % (P, pc - H))
    if need_snap and not a.no_promote:
        writeback(emit, "%lregs", "%snap")
    emit("  br label %%%s%s" % (P, entry))
    counting = a.counters or (a.mode == "region" and not a.nocount)
    table_fns = set()
    table_geps = set()

    def targets_for_dynamic():
        if succ:
            return [succ[pc]]
        return [t for t in steps]

    def R(x):
        return holes(LOCAL.sub(ren, x))

    for lab, ls in blocks:
        if lab not in reach:
            continue
        label(P + lab)
        idx = nblocks[0]
        nblocks[0] += 1
        blockmap["%d:%s" % (pc, lab)] = idx
        if counting:
            counter(emit, idx)
        skip_ret = False
        for l in ls:
            if skip_ret:
                skip_ret = False
                if l.strip() == "ret void":
                    continue
            s = l.strip()
            if re.match(r'%[-\w$.]+ = alloca', s):
                allocas.append(LOCAL.sub(ren, l))
                continue
            m = re.match(r'(%[-\w$.]+) = getelementptr inbounds ptr, ptr @"vm.Mini.table"', s)
            if m:
                table_geps.add(m.group(1))
            m = re.match(r'(%[-\w$.]+) = load ptr, ptr (%[-\w$.]+)', s)
            if m and m.group(2) in table_geps:
                table_fns.add(m.group(1))
            m = CONT_RE.search(s)
            if m:
                t = cont_target(pc, m.group(1), m.group(2))
                args = [x.split(" ", 1)[1] for x in split_args(m.group(3))]
                ctx, _, C, k, fp = [R(x) for x in args[:5]]
                internal = (succ[pc] == t) if succ else (t in seen)
                if internal and on(lab):
                    tl = "%s%d." % ("s" if (cp == "p" and t == H) else cp, t)
                    edges_out.append((pc, idx, t))
                    goto_head(tl, C, ctx, k, fp, t)
                else:
                    pcv = fresh("xpc")
                    emit("  %s = getelementptr inbounds i32, ptr %%tr.pc, i64 %d" % (pcv, t - H))
                    h, w = table_call(pcv)
                    exit_call(h, ctx, pcv, C, k, fp, w)
                skip_ret = True
                continue
            mt = MT_RE.match(l)
            if mt:
                args = [x.split(" ", 1)[1] for x in split_args(mt.group(2))]
                ctx, pcv, C, k, fp, w = [R(x) for x in args[:6]]
                callee = LOCAL.sub(ren, mt.group(1))
                if mt.group(1) in table_fns and on(lab):
                    # a dispatch through the table (an inlined cold block's
                    # `next`): compare its pc with the trace's next steps
                    d = fresh("doff"); x1 = fresh("dp"); x2 = fresh("db")
                    emit("  %s = ptrtoint ptr %s to i64" % (x1, pcv))
                    emit("  %s = ptrtoint ptr %%tr.pc to i64" % x2)
                    emit("  %s = sub i64 %s, %s" % (d, x1, x2))
                    xl = fresh("dexit")[1:]
                    cases = []
                    for t in targets_for_dynamic():
                        tl = "%s%d." % ("s" if (cp == "p" and t == H) else cp, t)
                        cases.append((4 * (t - H), fresh("dedge")[1:], tl, t))
                    emit("  switch i64 %s, label %%%s [ %s ]" % (d, xl, " ".join("i64 %d, label %%%s" % (c, el) for c, el, _, _ in cases)))
                    for c, el, tl, t in cases:
                        label(el)
                        ei = nblocks[0]
                        nblocks[0] += 1
                        blockmap["%d:%s" % (pc, el)] = ei
                        if counting:
                            counter(emit, ei)
                        edges_out.append((pc, ei, t))
                        goto_head(tl, C, ctx, k, fp, t)
                    label(xl)
                exit_call(callee, ctx, pcv, C, k, fp, w)
                skip_ret = True
                continue
            if not a.no_promote:
                m = re.match(r'store ptr (%[-\w$.]+), ptr (%[-\w$.]+)', s)
                if m and m.group(2) in slots and m.group(1) not in exact:
                    # regs changes: an offset into the window becomes the
                    # real file (written back); the real file itself goes
                    # back to the window; another frame writes the window back
                    X, Sl = R(m.group(1)), R(m.group(2))
                    if m.group(1) in derived:
                        writeback(emit)
                        emit("  store ptr %s, ptr %s" % (rebase_val(X), Sl))
                    else:
                        q = fresh("req"); sel = fresh("rsel")
                        lw = fresh("rw")[1:]; lk = fresh("rk")[1:]
                        emit("  %s = icmp eq ptr %s, %%tr.regs" % (q, X))
                        emit("  %s = select i1 %s, ptr %%lregs, ptr %s" % (sel, q, X))
                        emit("  br i1 %s, label %%%s, label %%%s" % (q, lk, lw))
                        label(lw)
                        writeback(emit)
                        emit("  br label %%%s" % lk)
                        label(lk)
                        emit("  store ptr %s, ptr %s" % (sel, Sl))
                    continue
                if m and m.group(1) in regsish and m.group(2) not in allocas_l:
                    # a register-file pointer stored into memory (a frame's base)
                    emit("  store ptr %s, ptr %s" % (rebase_val(R(m.group(1))), R(m.group(2))))
                    continue
            l2 = R(l)
            s2 = l2.strip()
            cm = CALL_RE.search(s2)
            if cm and "asm" not in s2 and not (cm.group(1) and safe_fn(cm.group(1))):
                # a call that may see the register file or collect
                writeback(emit)

                def rebase(mm):
                    v = mm.group(1)
                    if v.startswith("%" + P) and ("%" + v[len(P) + 1:]) in regsish:
                        return "ptr " + rebase_val(v)
                    if v == "%lregs":
                        return "ptr %tr.regs"
                    return mm.group(0)
                head, _, rest = l2.partition("(")
                rest = re.sub(r'ptr (%[-\w$."]+)', rebase, rest)
                emit(head + "(" + rest)
                if not a.no_promote:
                    reload(emit)
                continue
            if s2.startswith("br i1"):
                m2 = re.match(r'(\s*br i1 [^,]+), label (%[-\w$."]+), label (%[-\w$."]+)', l2)
                labs = []
                for tl in (m2.group(2), m2.group(3)):
                    orig = tl[len(P) + 1:]
                    if (lab, orig) in guard_edges:
                        # untaken on the trace: exit, the interpreter redoes the instruction
                        g = fresh("guard")[1:]
                        labs.append("%" + g)
                        guards.append((g, pc, P))
                        nreexec[0] += 1
                    else:
                        labs.append(tl)
                l2 = "%s, label %s, label %s" % (m2.group(1), labs[0], labs[1])
            emit(l2)
    # allocas of unreachable blocks
    for lab, ls in blocks:
        if lab in reach:
            continue
        for l in ls:
            if re.match(r'\s*%[-\w$.]+ = alloca', l):
                allocas.append(LOCAL.sub(ren, l))
    return


guards = []
FIRST = "p%d." % H if a.peel else "s%d." % H
if a.peel:
    # LuaJIT-style loop peeling: one copy of the trace as a preamble, then
    # the loop, so that facts established by the preamble hold in the loop
    for si, pc in enumerate(steps):
        emit_step(si, pc, "p")
for si, pc in enumerate(steps):
    emit_step(si, pc)

# guard exits: write the step's window back, re-dispatch at its pc
gout = []
for g, pc, P in guards:
    gout.append(g + ":")
    if a.counters:
        blockmap["guard:%d:%s" % (pc, g)] = nblocks[0]
        counter(gout.append, nblocks[0])
        nblocks[0] += 1
    if a.blacklist:
        # too many failures: the trace has no side traces, so drop it
        f0 = fresh("gf"); f1 = fresh("gf"); hit = fresh("gh")
        lb = fresh("gb")[1:]; lc = fresh("gc")[1:]
        gout.append('  %s = load i64, ptr @"loimi.trace.fails.%s"' % (f0, a.name))
        gout.append('  %s = add i64 %s, 1' % (f1, f0))
        gout.append('  store i64 %s, ptr @"loimi.trace.fails.%s"' % (f1, a.name))
        gout.append('  %s = icmp eq i64 %s, %d' % (hit, f1, a.blacklist))
        gout.append('  br i1 %s, label %%%s, label %%%s' % (hit, lb, lc))
        gout.append(lb + ":")
        gout.append('  call void @loimi_trace_blacklist(ptr @"loimi.trace.name.%s")' % a.name)
        gout.append('  br label %%%s' % lc)
        gout.append(lc + ":")
    if not a.no_promote:
        writeback(gout.append, "%snap", "%tr.regs")
    dispatch(gout.append, "%" + P + "ctx", "%" + P + "pc", "%" + P + "k", "%" + P + "fp")

if refused:
    sys.exit("refused:\n  " + "\n  ".join(sorted(set(refused))))

# phis
heads_in[FIRST].insert(0, ("%entry", "%tr.ctx", "%tr.k", "%tr.fp"))
res = []
for l in out:
    if l.startswith("@@PHIS"):
        P = l.split()[1]
        ins = heads_in[P]
        if not ins:
            for nm in ("ctx", "k", "fp"):
                res.append("  %%%s%s = getelementptr i8, ptr %%tr.%s, i64 0" % (P, nm, nm))
            continue
        for j, nm in ((1, "ctx"), (2, "k"), (3, "fp")):
            res.append("  %%%s%s = phi ptr %s" % (P, nm, ", ".join("[ %s, %s ]" % (x[j], x[0]) for x in ins)))
    else:
        res.append(l)

hdr = []
fname = "loimi.trace.%s" % a.name
hdr.append('define preserve_nonecc void @"%s"(ptr %%tr.ctx, ptr %%tr.pc, ptr %%tr.regs, ptr %%tr.k, ptr %%tr.fp, i32 %%tr.insn) nounwind "no-builtin-memset" "no-builtin-memcpy" "no-builtin-memmove" {' % fname)
hdr.append("entry:")
hdr.extend(allocas)
if not a.no_promote:
    hdr.append("  %%lregs = alloca [%d x i64], align 8" % NR)
    hdr.append("  %%snap = alloca [%d x i64], align 8" % NR)
    reload(hdr.append)
hdr.append("  br label %%%shead" % FIRST)
text = "\n".join(hdr + res + gout) + "\n}\n"

# declarations the program does not have
refs = set(re.findall(r'@("[^"]+"|[-\w$.]+)', text))
extra = []
for r in sorted(refs):
    if r == "loimi_trace_blacklist":
        extra.append("declare void @loimi_trace_blacklist(ptr)")
        continue
    if r in main_defs or r in main_decls or r == '"%s"' % fname or r.startswith('"loimi.trace.'):
        continue
    if r in decls:
        extra.append(decls[r])
    elif r in fn_head:
        extra.append(re.sub(r'^define (available_externally |internal |dso_local )*', 'declare ', fn_head[r]).split("{")[0])
    else:
        sys.exit("unknown symbol @%s" % r)
lo = min(steps)
hi = 0
for pc in steps:
    for l in fn_lines[stencil_of(pc)]:
        for m in re.finditer(r'@"__jit\.[fwcj]\.(\d+)\.', l):
            hi = max(hi, pc + int(m.group(1)) + 1)
    hi = max(hi, pc + 1)
hsh = 1469598103934665603
for pc in range(lo, hi):
    if pc in code["op"]:
        hsh ^= W[pc]
        hsh = (hsh * 1099511628211) & ((1 << 64) - 1)
ncount = nblocks[0] if (a.counters or (a.mode == "region" and not a.nocount)) else 0
glob = ['@"loimi.trace.counts.%s" = global [%d x i64] zeroinitializer' % (a.name, max(ncount, 1)),
        '@"loimi.trace.fails.%s" = internal global i64 0' % a.name,
        '@"loimi.trace.name.%s" = private constant [%d x i8] c"%s\\00"' % (a.name, len(a.name) + 1, a.name),
        '@loimi_traces = constant [1 x { i64, i64, i64, i64, i64, ptr, ptr, i64, ptr }] [{ i64, i64, i64, i64, i64, ptr, ptr, i64, ptr } { i64 %d, i64 %d, i64 %d, i64 %d, i64 %d, ptr @"%s", ptr @"loimi.trace.counts.%s", i64 %d, ptr @"loimi.trace.name.%s" }]'
        % (code["n"], lo, hi, i64s(hsh), H, fname, a.name, ncount, a.name),
        '@loimi_ntraces = constant i64 1']
open(a.out, "w").write("\n; trace-tier experiment: %s, %s mode, steps %s\n" % (a.name, a.mode, steps) + "\n".join(extra + glob) + "\n" + text)
if a.mapout:
    json.dump({"blocks": blockmap, "edges": edges_out, "steps": steps}, open(a.mapout, "w"))
print("blocks %d, guards %d, steps with side effects %s, heap constants loaded at %s" % (nblocks[0], nreexec[0], side_effect_steps, sorted(set(heap_consts))), file=sys.stderr)
