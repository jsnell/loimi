p = '/tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3/tracegen.py'
s = open(p).read()
old = '''    if not a.no_promote:
        writeback(gout.append, "%snap", "%tr.regs")
    dispatch(gout.append, "%" + P + "ctx", "%" + P + "pc", "%" + P + "k", "%" + P + "fp")
'''
new = '''    if a.blacklist:
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
'''
assert old in s
s = s.replace(old, new)
old = '''glob = ['@"loimi.trace.counts.%s" = global [%d x i64] zeroinitializer' % (a.name, max(ncount, 1)),'''
new = '''glob = ['@"loimi.trace.counts.%s" = global [%d x i64] zeroinitializer' % (a.name, max(ncount, 1)),
        '@"loimi.trace.fails.%s" = internal global i64 0' % a.name,'''
assert old in s
s = s.replace(old, new)
old = '''    if r in main_defs or r in main_decls or r == '"%s"' % fname or r.startswith('"loimi.trace.'):'''
new = '''    if r == "loimi_trace_blacklist":
        extra.append("declare void @loimi_trace_blacklist(ptr)")
        continue
    if r in main_defs or r in main_decls or r == '"%s"' % fname or r.startswith('"loimi.trace.'):'''
assert old in s
s = s.replace(old, new)
old = '''ap.add_argument("--peel",'''
new = '''ap.add_argument("--blacklist", type=int, default=1000, help="drop the trace after N guard failures (0: never)")
ap.add_argument("--peel",'''
assert old in s
s = s.replace(old, new)
open(p, 'w').write(s)
