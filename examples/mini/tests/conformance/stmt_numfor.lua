local function collect(t) return table.concat(t, " ") end
local r = {}
for i = 1, 5 do r[#r + 1] = i end
print(collect(r))
r = {}
for i = 5, 1, -1 do r[#r + 1] = i end
print(collect(r))
r = {}
for i = 10, 1, -3 do r[#r + 1] = i end
print(collect(r))
r = {}
for i = 1, 10, 4 do r[#r + 1] = i end
print(collect(r))
r = {}
for i = 1, 0 do r[#r + 1] = i end
print(#r)
for i = 3, 3 do print("once", i) end
-- float loops
r = {}
for i = 1, 2, 0.5 do r[#r + 1] = i end
print(collect(r))
r = {}
for i = 1.0, 3 do r[#r + 1] = i end
print(collect(r))
r = {}
for i = 0.5, -0.5, -0.25 do r[#r + 1] = i end
print(collect(r))
-- integer start and step with a float limit: integer loop
r = {}
for i = 1, 3.5 do r[#r + 1] = i end
print(collect(r))
r = {}
for i = 3, 0.5, -1 do r[#r + 1] = i end
print(collect(r))
-- assigning to the loop variable does not affect iteration
r = {}
for i = 1, 3 do
  r[#r + 1] = i
  i = i * 100
  r[#r + 1] = i
end
print(collect(r))
-- start, limit and step are evaluated once
local n = 3
local calls = 0
local function lim() calls = calls + 1; return n end
for i = 1, lim() do n = 10 end
print(calls, n)
-- loop variable is local to the loop
local i = "outer"
for i = 1, 2 do end
print(i)
-- break
for j = 1, 100 do
  if j * j > 50 then print("break at", j); break end
end
-- negative ranges
r = {}
for j = -3, -1 do r[#r + 1] = j end
print(collect(r))
