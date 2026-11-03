def score(p):
    name, age = p
    return age

def by_len(w):
    return len(w)

def report(people):
    ages = [score(p) for p in people]
    older = {p[0]: a for p, a in zip(people, ages) if a > 26}
    return ages, older

people = [("ann", 31), ("bob", 25), ("cy", 40)]
print(report(people))
ordered = sorted(people, key=score)
print(ordered)
words = sorted(["ccc", "a", "bb"], key=lambda w: by_len(w))
print(words)
total = sum(x for x in range(5) if x % 2)
print(total)
