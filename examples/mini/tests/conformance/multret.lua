local function three() return 1, 2, 3 end
local function none() end
local function one() return "one" end

local a, b, c, d = three()
print(a, b, c, d)
local x, y = three()
print(x, y)
local p, q, r = three(), 20
print(p, q, r)
local s, t2 = (three())
print(s, t2)
local n1, n2 = none()
print(n1, n2)
local z = none()
print(z)
print((three()))
print((none()))
-- call in argument / non-last positions is truncated to one value
print(three(), "end")
local tab = {three(), three(), "x"}
print(#tab, tab[1], tab[2], tab[3])
-- assignment
local m1, m2, m3
m1, m2, m3 = three()
print(m1, m2, m3)
m1, m2, m3 = 0, three()
print(m1, m2, m3)
m1, m2 = three(), three()
print(m1, m2)
-- extra values discarded, missing values nil
m1, m2 = 1, 2, 3
print(m1, m2)
m1, m2, m3 = 1
print(m1, m2, m3)
-- results into a function argument via local
local q1, q2 = three()
print(q1 + q2)
local function divmod(a, b) return math.floor(a / b), a % b end
local qq, rr = divmod(17, 5)
print(qq, rr)
local o = one()
print(o)
