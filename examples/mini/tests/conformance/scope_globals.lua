print(undefined_global)
gx = 10
print(gx)
local function setg() gy = "set in function" end
setg()
print(gy)
local gx = "local shadows global"
print(gx)
-- a local is visible from the next statement
local z = 1
local z = z + 1
print(z)
-- local function sees itself; plain local does not
local function rec(n) if n == 0 then return "rec ok" end return rec(n - 1) end
print(rec(3))
local notrec = 5
local notrec = function() return notrec end
print(notrec())
-- blocks limit scope
local v = "outer"
if true then local v = "inner"; print(v) end
for i = 1, 1 do local v = "loop"; print(v) end
print(v)
-- assigning to a global that shares a name with an out-of-scope local
do local w = 1 end
w = "global w"
print(w)
-- functions see globals defined later
local function later() return defined_later end
defined_later = "late"
print(later())
