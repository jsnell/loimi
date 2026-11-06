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
                refused.append("pc %d: a heap constant (%x): not stable across runs" % (pc, v))
            return "= add i64 0, %d" % i64s(v)
        l = re.sub(r'= call i64 asm "movabsq [^"]*", "=r,i"\(ptr @"__jit\.c\.(\d+)\.(\d+)\.(\d+)\.u"\)', chole, l)
        if "@\"__jit." in l and "musttail" not in l:
            sys.exit("unhandled hole at %d: %s" % (pc, l))
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
    for lab, ls in blocks:
        if lab not in reach:
            continue
        for l in ls:
            s = l.strip()
            m = re.match(r'store ptr (%[-\w$.]+), ptr (%[-\w$.]+)', s)
            if m and m.group(2) in slots and m.group(1) not in exact:
                refused.append("pc %d (%s): changes regs" % (pc, lab))
            if m and m.group(1) in regsish and m.group(2) not in allocas_l:
                refused.append("pc %d (%s): stores a register-file pointer" % (pc, lab))
    need_snap = any(True for _ in guard_edges)

    emit = out.append
    emit(P + "head:")
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

    for lab, ls in blocks:
        if lab not in reach:
            # still hoist its allocas (other blocks may not use them, harmless)
            continue
        emit(P + lab + ":")
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
                ctx, _, _, k, fp = [LOCAL.sub(ren, x) for x in args[:5]]
                internal = (succ[pc] == t) if succ else (t in seen)
                if internal and on(lab):
                    tl = "%s%d." % ("s" if (cp == "p" and t == H) else cp, t)
                    emit("  br label %%%shead" % tl)
                    heads_in[tl].append(("%" + P + lab, ctx, k, fp))
                    edges_out.append((pc, idx, t))
                else:
                    writeback(emit)
                    pcv = fresh("xpc")
                    emit("  %s = getelementptr inbounds i32, ptr %%tr.pc, i64 %d" % (pcv, t - H))
                    dispatch(emit, ctx, pcv, k, fp)
                skip_ret = True
                continue
            mt = MT_RE.match(l)
            if mt and mt.group(1) in table_fns and on(lab):
                # a dispatch through the table (an inlined cold block's `next`):
                # compare its pc with the trace's next steps
                args = [x.split(" ", 1)[1] for x in split_args(mt.group(2))]
                ctx, pcv, _, k, fp, w = [holes(LOCAL.sub(ren, x)) for x in args[:6]]
                d = fresh("doff")
                x1 = fresh("dp")
                x2 = fresh("db")
                emit("  %s = ptrtoint ptr %s to i64" % (x1, pcv))
                emit("  %s = ptrtoint ptr %%tr.pc to i64" % x2)
                emit("  %s = sub i64 %s, %s" % (d, x1, x2))
                xl = fresh("dexit")[1:]
                cases = []
                for t in targets_for_dynamic():
                    tl = "%s%d." % ("s" if (cp == "p" and t == H) else cp, t)
                    el = fresh("dedge")[1:]
                    cases.append((4 * (t - H), el, tl, t))
                emit("  switch i64 %s, label %%%s [ %s ]" % (d, xl, " ".join("i64 %d, label %%%s" % (c, el) for c, el, _, _ in cases)))
                for c, el, tl, t in cases:
                    emit(el + ":")
                    ei = nblocks[0]
                    nblocks[0] += 1
                    blockmap["%d:%s" % (pc, el)] = ei
                    if counting:
                        counter(emit, ei)
                    emit("  br label %%%shead" % tl)
                    heads_in[tl].append(("%" + el, ctx, k, fp))
                    edges_out.append((pc, ei, t))
                emit(xl + ":")
                writeback(emit)
                emit("  musttail call preserve_nonecc void %s(ptr %s, ptr %s, ptr %%tr.regs, ptr %s, ptr %s, i32 %s)" % (
                    LOCAL.sub(ren, mt.group(1)), ctx, pcv, k, fp, w))
                continue
            l2 = holes(LOCAL.sub(ren, l))
            s2 = l2.strip()
            if s2.startswith("musttail call"):
                # an exit (cold path, dispatch): write back, pass the real regs
                writeback(emit)
                l2 = re.sub(r'(\(ptr [^,]+, ptr [^,]+, )ptr [^,]+,', r'\1ptr %tr.regs,', l2, count=1)
                emit(l2)
                continue
            cm = CALL_RE.search(s2)
            if cm and "asm" not in s2 and not (cm.group(1) and safe_fn(cm.group(1))):
                # a call that may see the register file or collect
                writeback(emit)

                def rebase(mm):
                    v = mm.group(1)
                    if v.startswith("%" + P) and ("%" + v[len(P) + 1:]) in regsish:
                        x = fresh("ri"); y = fresh("rl"); d = fresh("rd"); g = fresh("rg")
                        emit("  %s = ptrtoint ptr %s to i64" % (x, v))
                        emit("  %s = ptrtoint ptr %%lregs to i64" % y)
                        emit("  %s = sub i64 %s, %s" % (d, x, y))
                        emit("  %s = getelementptr i8, ptr %%tr.regs, i64 %s" % (g, d))
                        return "ptr " + g
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


