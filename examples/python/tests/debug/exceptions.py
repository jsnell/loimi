class Invalid(ValueError):
    pass

def parse(s):
    if not s.isdigit():
        raise Invalid("bad: " + s)
    return int(s)

def check(s):
    n = parse(s)
    return n * 2

def run(items):
    out = []
    for s in items:
        try:
            try:
                out.append(check(s))
            finally:
                print("checked", s)
        except ValueError as e:
            print("error", e)
            out.append(-1)
    return out

print(run(["4", "x", "7", "y"]))
