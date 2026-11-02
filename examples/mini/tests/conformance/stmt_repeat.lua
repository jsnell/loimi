local i = 0
repeat i = i + 1 until i >= 5
print(i)
-- body runs at least once
local runs = 0
repeat runs = runs + 1 until true
print(runs)
-- the condition can see the body's locals
local n = 0
repeat
  local done = n >= 3
  n = n + 1
until done
print(n)
-- break inside repeat
local k = 0
repeat
  k = k + 1
  if k == 7 then break end
until false
print(k)
