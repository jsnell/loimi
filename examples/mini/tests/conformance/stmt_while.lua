local i, s = 1, 0
while i <= 10 do s = s + i; i = i + 1 end
print(i, s)
while false do print("never") end
-- break exits the innermost loop only
local out = {}
local a = 0
while true do
  a = a + 1
  local b = 0
  while true do
    b = b + 1
    if b > a then break end
  end
  out[#out + 1] = a .. ":" .. b
  if a >= 3 then break end
end
print(table.concat(out, " "))
-- condition re-evaluated each iteration
local t = {1, 2, 3}
local n = 0
while #t > 0 do table.remove(t); n = n + 1 end
print(n, #t)
