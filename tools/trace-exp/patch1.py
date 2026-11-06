p = '/tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3/tracegen.py'
s = open(p).read()
old = '''            if (v >> 48) >= 0xfffa and (v & 0xffffffffffff) > 0x10000:
                refused.append("pc %d: a heap constant (%x): not stable across runs" % (pc, v))
            return "= add i64 0, %d" % i64s(v)'''
new = '''            if (v >> 48) >= 0xfffa and (v & 0xffffffffffff) > 0x10000:
                # a heap constant: its address differs between runs, so
                # the trace loads it from the pool (k[field])
                heap_consts.append(pc)
                o, sh, b = int(m.group(1)), int(m.group(2)), int(m.group(3))
                g = fresh("kp")
                return "@@PRE %s = getelementptr inbounds i64, ptr %%%sk, i64 %d@@= load i64, ptr %s" % (g, P, field(W[pc + o], sh, b, False), g)
            return "= add i64 0, %d" % i64s(v)'''
assert old in s
s = s.replace(old, new)
old = '''            sys.exit("unhandled hole at %d: %s" % (pc, l))
        return l'''
new = '''            sys.exit("unhandled hole at %d: %s" % (pc, l))
        m = re.match(r'(.*?)@@PRE (.*?)@@(.*)$', l)
        if m:
            l = "  " + m.group(2) + "\\n" + m.group(1) + m.group(3)
        return l'''
assert old in s
s = s.replace(old, new)
s = s.replace("side_effect_steps = []\n", "side_effect_steps = []\nheap_consts = []\n", 1)
old = '''print("blocks %d, guards %d, steps with side effects %s" % (nblocks[0], nreexec[0], side_effect_steps), file=sys.stderr)'''
assert old in s
s = s.replace(old, '''print("blocks %d, guards %d, steps with side effects %s, heap constants loaded at %s" % (nblocks[0], nreexec[0], side_effect_steps, sorted(set(heap_consts))), file=sys.stderr)''')
open(p, 'w').write(s)
