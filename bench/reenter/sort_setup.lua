-- The other benchmarks without the sort: filling the table and the
-- final scan, to subtract.
local N = 200000
local t = {}
local x = 42
for i = 1, N do x = (x * 1103515245 + 12345) % 2147483648; t[i] = x end
local n = 0
for i = 2, N do if t[i - 1] <= t[i] then n = n + 1 end end
print(n > 0)
