local t = {}
print(#t)
for i = 1, 100 do t[i] = i end
print(#t)
for i = 100, 51, -1 do t[i] = nil end
print(#t)
t[#t + 1] = "x"
print(#t, t[51])
local s = {"a", "b", "c", "d", "e"}
s[5] = nil
s[4] = nil
print(#s)
s[4] = "D"
print(#s)
-- non-sequence keys do not affect the border
local m = {1, 2, 3, x = 1, y = 2, [0] = 0, [-1] = -1, [1.5] = 1.5}
print(#m)
-- growing in the hash part, then reading back
local h = {}
h[3] = 3; h[2] = 2; h[1] = 1
print(#h)
