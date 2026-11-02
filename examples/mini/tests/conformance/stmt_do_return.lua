local x = 1
do
  local x = 2
  print(x)
end
print(x)
do local y = 3 end
print(y)
local function early(n)
  if n > 0 then return "positive" end
  do return "non-positive" end
end
print(early(1), early(-1))
local function noval() return end
print((noval()))
local function multi(n)
  for i = 1, 10 do
    if i == n then return i * 100 end
  end
  return -1
end
print(multi(4), multi(20))
-- return from inside nested loops
local function find(grid, v)
  for i = 1, #grid do
    for j = 1, #grid[i] do
      if grid[i][j] == v then return i, j end
    end
  end
  return nil
end
local i, j = find({{1, 2}, {3, 4}}, 4)
print(i, j)
-- semicolons separating statements
local a = 1; local b = 2; a = a + b; print(a);
-- break as the last statement of a block
while true do break end
print("done")
