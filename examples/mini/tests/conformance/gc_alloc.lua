-- allocate far more than stays live; unreachable objects must be reclaimed
local live = {}
local total = 0
for i = 1, 200000 do
  local t = {i, tostring(i), {i}}
  total = total + t[3][1] % 7
  if i % 1000 == 0 then live[#live + 1] = t end
end
print(#live, total, live[200][2])
local s
for i = 1, 20000 do s = string.rep("x", i % 100) .. i end
print(s)
local function chain(n) local c = nil for i = 1, n do c = {next = c, v = i} end return c end
local sum = 0
for r = 1, 50 do
  local c = chain(1000)
  while c do sum = sum + c.v; c = c.next end
end
print(sum)
