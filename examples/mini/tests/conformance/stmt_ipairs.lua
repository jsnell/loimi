local t = {"a", "b", "c"}
for i, v in ipairs(t) do print(i, v) end
-- stops at the first nil
local h = {1, 2, nil, 4}
local n = 0
for i, v in ipairs(h) do n = i end
print(n)
local u = {}
u[1] = "x"; u[2] = "y"; u[4] = "w"
n = 0
for i in ipairs(u) do n = n + 1 end
print(n)
-- ignores non-sequence keys
local m = {10, 20, k = "v", [0] = 0}
local s = 0
for _, v in ipairs(m) do s = s + v end
print(s)
-- empty
for i, v in ipairs({}) do print("never") end
-- assigning to existing fields during traversal
local w = {1, 2, 3, 4}
for i, v in ipairs(w) do w[i] = v * v end
print(table.concat(w, ","))
-- break
for i, v in ipairs({5, 6, 7, 8}) do
  if v == 7 then print("found at", i); break end
end
-- nested
local grid = {{1, 2}, {3, 4}, {5, 6}}
local acc = {}
for i, row in ipairs(grid) do
  for j, v in ipairs(row) do acc[#acc + 1] = i .. j .. "=" .. v end
end
print(table.concat(acc, " "))
