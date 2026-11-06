p = '/tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3/tracegen.py'
s = open(p).read()
old = '''    work.extend(conts(pc))
region = sorted(seen)'''
new = '''    work.extend(conts(pc))
# continuations through the table (inlined cold blocks) are not static:
# the region is every instruction of the loop's range
seen |= set(pc for pc in code["op"] if H <= pc <= B)
region = sorted(seen)'''
assert old in s
s = s.replace(old, new)
old = '''        if not ins:
            sys.exit("step %s has no predecessor" % P)
        for j, nm in ((1, "ctx"), (2, "k"), (3, "fp")):'''
new = '''        if not ins:
            for nm in ("ctx", "k", "fp"):
                res.append("  %%%s%s = getelementptr i8, ptr %%tr.%s, i64 0" % (P, nm, nm))
            continue
        for j, nm in ((1, "ctx"), (2, "k"), (3, "fp")):'''
assert old in s
s = s.replace(old, new)
open(p, 'w').write(s)
