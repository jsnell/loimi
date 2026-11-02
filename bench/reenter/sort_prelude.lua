-- The same sort written in Lua (Lua 5.4's auxsort and partition, as
-- examples/mini/lib.lm has them), with the same comparator: every
-- comparison is an ordinary call in the one dispatch loop.
local function auxsort(a, lo, up, lt)
  while lo < up do
    if lt(a[up], a[lo]) then a[lo], a[up] = a[up], a[lo] end
    if up - lo == 1 then break end
    local p = math.floor((lo + up) / 2)
    if lt(a[p], a[lo]) then a[p], a[lo] = a[lo], a[p]
    elseif lt(a[up], a[p]) then a[p], a[up] = a[up], a[p] end
    if up - lo == 2 then break end
    local P = a[p]
    a[p], a[up - 1] = a[up - 1], P
    local i, j = lo, up - 1
    while true do
      i = i + 1
      while lt(a[i], P) do
        if i == up - 1 then error("invalid order function for sorting") end
        i = i + 1
      end
      j = j - 1
      while lt(P, a[j]) do
        if j < i then error("invalid order function for sorting") end
        j = j - 1
      end
      if j < i then break end
      a[i], a[j] = a[j], a[i]
    end
    a[up - 1], a[i] = a[i], P
    p = i
    if p - lo < up - p then
      auxsort(a, lo, p - 1, lt)
      lo = p + 1
    else
      auxsort(a, p + 1, up, lt)
      up = p - 1
    end
  end
end

local N = 200000
local t = {}
local x = 42
for i = 1, N do x = (x * 1103515245 + 12345) % 2147483648; t[i] = x end
local calls = 0
auxsort(t, 1, #t, function(a, b) calls = calls + 1; return a < b end)
for i = 2, N do assert(t[i - 1] <= t[i]) end
print(calls)
