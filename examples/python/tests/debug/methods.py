class Counter:
    def __init__(self, start):
        self.n = start
        self.log = []

    def bump(self, by):
        old = self.n
        self.n = old + by
        self.log.append(by)
        return self.n

    def total(self):
        return sum(self.log)

def main():
    c = Counter(10)
    for k in range(3):
        c.bump(k)
    print(c.n, c.total())
    return c

result = main()
print(result.log)
