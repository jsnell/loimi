local a, b = 1, 2
a, b = b, a
print(a, b)
local x, y, z = 1, 2, 3
x, y, z = z, x, y
print(x, y, z)
-- all expressions are evaluated before assignment (Lua manual example)
local t = {}
local i = 3
i, t[i] = i + 1, 20
print(i, t[3], t[4])
-- swapping table elements
local arr = {1, 2, 3}
arr[1], arr[3] = arr[3], arr[1]
print(arr[1], arr[2], arr[3])
-- field and index targets
local o = {inner = {}}
o.inner.v, o["k"], arr[2] = "iv", "kv", "two"
print(o.inner.v, o.k, arr[2])
-- globals
g1, g2 = "G1", "G2"
print(g1, g2)
g1 = nil
print(g1)
-- local with no value
local u, v
print(u, v)
-- re-declaring a local shadows
local s = 1
local s = s + 1
print(s)
