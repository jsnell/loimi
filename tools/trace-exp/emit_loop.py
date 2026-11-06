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
        line = "  musttail call preserve_nonecc void %s(ptr %s, ptr %s, ptr %%s, ptr %s, ptr %s, i32 %s)" % (callee, ctx, pcv, k, fp, w)
        if a.no_promote or C in ("%lregs", "%tr.regs"):
            writeback(emit)
            emit(line % "%tr.regs")
            emit("  ret void")
            return
        q = fresh("ceq")
        lw = fresh("xw")[1:]
        ln = fresh("xn")[1:]
        emit("  %s = icmp eq ptr %s, %%lregs" % (q, C))
        emit("  br i1 %s, label %%%s, label %%%s" % (q, lw, ln))
        label(lw)
        writeback(emit)
        emit(line % "%tr.regs")
        emit("  ret void")
        label(ln)
        emit(line % C)
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
