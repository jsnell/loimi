local t = {}
print(#t, t[1])
t = {10, 20, 30}
print(#t, t[1], t[2], t[3], t[4])
t = {x = 1, y = "two", ["z w"] = 3, [1 + 1] = "two", [true] = "yes"}
print(t.x, t.y, t["z w"], t[2], t[true], t[1])
t = {1, 2; 3; n = 4, 5,}
print(#t, t[1], t[2], t[3], t[4], t.n)
t = {"p", "q", [10] = "r", s = {1, {2, {3}}}}
print(t[1], t[2], t[10], t.s[2][2][1], #t.s)
local k = "key"
t = {[k] = "v", [k .. "2"] = "v2", k = "literal"}
print(t.key, t.key2, t.k)
t = {f = function(x) return x * 2 end, 1}
print(t.f(21), t[1])
t = {{}, {}, {}}
print(#t, t[1] == t[2])
local n = 0
local function nxt() n = n + 1; return n end
t = {nxt(), nxt(), nxt()}
print(t[1] + t[2] + t[3], n, #t)
