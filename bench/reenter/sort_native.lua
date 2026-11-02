-- table.sort with a Lua comparator: the sort is native, and each
-- comparison re-enters the interpreter (a nested run).
local N = 200000
local t = {}
local x = 42
for i = 1, N do x = (x * 1103515245 + 12345) % 2147483648; t[i] = x end
local calls = 0
table.sort(t, function(a, b) calls = calls + 1; return a < b end)
for i = 2, N do assert(t[i - 1] <= t[i]) end
print(calls)
