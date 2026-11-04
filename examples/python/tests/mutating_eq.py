# __eq__ that changes the list being searched or compared: each step must
# read the list's current items (the list moves to a new array as it
# grows, and the collector may free the old one under the scan).
class Grow:
    def __init__(self, target, result):
        self.target = target
        self.result = result
    def __eq__(self, o):
        for i in range(3000):
            self.target.append([i])
        return self.result

L = []
L.extend([Grow(L, False), 5, 7, 9])
print(L.index(9))
print(L.count(7))
print(len(L))

M = []
M.extend([Grow(M, True), 1, 2])
print(M < [0, 1, 3])
print(len(M))
print((1, 2, 3) < (1, 2, 4))
print([1, 2] < [1, 2, 0])
