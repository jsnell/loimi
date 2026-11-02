-- table.sort without a comparator: the native sort comparing with `<`
-- itself, no Mini code runs (the floor for sort_native.lua).
local N = 200000
local t = {}
local x = 42
for i = 1, N do x = (x * 1103515245 + 12345) % 2147483648; t[i] = x end
table.sort(t)
for i = 2, N do assert(t[i - 1] <= t[i]) end
print(#t)
