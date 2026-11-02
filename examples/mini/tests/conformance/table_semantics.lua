local t = {}
t[1] = "int"
print(t[1.0], t[1])
t[2.0] = "float key"
print(t[2], #t)
t[4 / 2] = "replaced"
print(t[2])
t[1.5] = "fraction"
print(t[1.5], t[1], t[2])
print(t.missing, t[99], t[-1], t[false])
t.a = 1
t.a = nil
print(t.a)
-- keys of every type
local f = function() end
local tk = {}
local u = {}
u[true] = "T"; u[false] = "F"; u[f] = "func"; u[tk] = "table"; u["s"] = "str"; u[0] = "zero"; u[-3] = "neg"
print(u[true], u[false], u[f], u[tk], u.s, u[0], u[-3])
print(u[{}], u[function() end])
-- tables are references
local a = {v = 1}
local b = a
b.v = 2
print(a.v)
-- string keys vs number keys
local m = {}
m["1"] = "string one"
m[1] = "number one"
print(m["1"], m[1])
-- removing with nil and counting
local c = {}
for i = 1, 20 do c["k" .. i] = i end
for i = 1, 20, 2 do c["k" .. i] = nil end
local n, sum = 0, 0
for k, v in pairs(c) do n = n + 1; sum = sum + v end
print(n, sum)
