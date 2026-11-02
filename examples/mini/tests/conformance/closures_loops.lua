-- numeric for: fresh variable per iteration
local fs = {}
for i = 1, 3 do fs[i] = function() return i end end
print(fs[1](), fs[2](), fs[3]())

-- loop-body locals are fresh per iteration (while)
local gs = {}
local j = 1
while j <= 3 do
  local k = j * 10
  gs[j] = function() k = k + 1; return k end
  j = j + 1
end
local w1 = gs[1](); local w2 = gs[1](); local w3 = gs[2](); local w4 = gs[3]()
print(w1, w2, w3, w4)

-- repeat
local hs = {}
local n = 0
repeat
  n = n + 1
  local m = n
  hs[n] = function() return m end
until n == 3
print(hs[1](), hs[2](), hs[3]())

-- ipairs and pairs loop variables
local ps = {}
for idx, val in ipairs({"a", "b", "c"}) do
  ps[idx] = function() return idx .. val end
end
print(ps[1](), ps[2](), ps[3]())
local qs = {}
for key, val in pairs({x = 1}) do
  qs[1] = function() return key .. "=" .. val end
end
print(qs[1]())

-- closures in the same iteration share; different iterations don't
local set, get = {}, {}
for i = 1, 2 do
  local v = 0
  set[i] = function(x) v = x end
  get[i] = function() return v end
end
set[1](100)
print(get[1](), get[2]())

-- modifying the numeric loop variable inside a closure doesn't affect the loop
local count = 0
for i = 1, 3 do
  local f = function() i = i + 10; return i end
  f()
  count = count + 1
end
print(count)
