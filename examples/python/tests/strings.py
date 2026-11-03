s = "Hello, World"
print(s.lower(), s.upper(), s[0], s[-1], s[2:5], s[::-1], s[::2], len(s))
print(s.split(", "), "a b  c".split(), "a,b,,c".split(","), "x".join(["1", "2", "3"]))
print(s.find("o"), s.rfind("o"), s.find("z"), s.index("W"), s.count("l"), "o" in s, "zz" not in s)
print(s.replace("l", "L"), s.startswith("Hell"), s.endswith(("x", "ld")), "  pad  ".strip() + "|")
print("abc".center(9, "*"), "7".zfill(3), "ab".ljust(4) + "|", "ab".rjust(4), "-5".zfill(4))
print(ord("A"), chr(97), "abc" < "abd", "b" > "abc", "ab" * 3, str(12) + str(3.5))
print("a-b-c".rsplit("-", 1), "a-b-c".split("-", 1), "line1\nline2\n".splitlines())
print("title case words".title(), "Cap".swapcase(), "x".isalpha(), "12".isdigit(), " ".isspace())
print("k=v".partition("="), repr("it's"), repr('say "hi"'), repr("tab\t\n"), str(None), str([1, "a"]))
print(sorted("banana"), list("abc"), "".join(sorted(set("mississippi"))))
words = "the quick brown fox jumps over the lazy dog".split()
counts = {}
for w in words:
    counts[w] = counts.get(w, 0) + 1
print(counts, max(words, key=len), sorted(words, key=lambda w: (-len(w), w))[:3])
print("%s is %d years" % ("Bob", 30), "%(a)s-%(b)s" % {"a": 1, "b": 2})
t = "x"
t += "y"
t *= 2
print(t, t == "xyxy", t is not None)
print(", ".join(f"{k}:{v}" for k, v in sorted(counts.items()) if v > 1))
print("abc"[1:], "abc"[:-1], "abc"[5:], "abcdef"[1:5:2], "hello"[::-2])
print(str(1.0), str(2.50), repr(0.1), str(1e100), "%.3f" % 2.0005, f"{2.5}")
text = """multi
line
  string"""
print(text.splitlines(), [l.strip() for l in text.split("\n")])
print("%-6s|%6s|" % ("ab", "cd"), "%3d|%-3d|" % (5, 5), "%c" % 65 if False else "", "a\tb")
name = "world"
print(f"hello {name}!", f"{name!r:>10}", f"{{literal}}", f"{1 + 2 * 3}", f"{'nested ' + name}")
width = 8
print(f"{3.14159:{width}.3f}|", f"{'left':<{width}}|", f"{[1, 2]}", f"{ {'a': 1}['a'] }")
print(" ".join(["%02d" % i for i in range(5)]), "{:>6.2f}".format(3.14159))
bs = "a,b;c d"
for sep in (",", ";", " "):
    bs = bs.replace(sep, "|")
print(bs, bs.split("|"), "x".join("abc"))
print("Ab1".isalnum(), "AB".isupper(), "ab".islower(), "".isdigit(), "a b".split(" "))
lines = ["  first  ", "second\n", "\tthird"]
print([l.strip() for l in lines], [l.lstrip() for l in lines], [l.rstrip() for l in lines])
print(str(True), str(3), repr([1.5, "x", None, (1,)]), str((1, "a")), {"k": "v"})
