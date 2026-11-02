-- queens: count all solutions of the N-queens problem by backtracking.
-- Recursion, boolean arrays, early exits. Classic benchmark.
local N = 12
local REPEAT = 3

local function solve(n)
  local cols, d1, d2 = {}, {}, {}
  for i = 1, n do cols[i] = false end
  for i = 1, 2 * n do d1[i] = false; d2[i] = false end
  local count = 0
  local function place(row)
    if row > n then
      count = count + 1
      return
    end
    for c = 1, n do
      local a, b = row + c, row - c + n
      if not cols[c] and not d1[a] and not d2[b] then
        cols[c], d1[a], d2[b] = true, true, true
        place(row + 1)
        cols[c], d1[a], d2[b] = false, false, false
      end
    end
  end
  place(1)
  return count
end

local total = 0
for r = 1, REPEAT do
  for n = 4, N do
    local c = solve(n)
    if r == 1 then print(string.format("%d-queens: %d", n, c)) end
    total = total + c
  end
end
print(string.format("total %d", total))
