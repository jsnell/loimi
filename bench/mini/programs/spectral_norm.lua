-- spectral-norm (Computer Language Benchmarks Game): float arithmetic,
-- array indexing, nested loops, many small calls.
local N = 1000

local function A(i, j)
  local ij = i + j - 1
  return 1.0 / (ij * (ij - 1) * 0.5 + i)
end

local function Av(x, y, n)
  for i = 1, n do
    local a = 0.0
    for j = 1, n do a = a + x[j] * A(i, j) end
    y[i] = a
  end
end

local function Atv(x, y, n)
  for i = 1, n do
    local a = 0.0
    for j = 1, n do a = a + x[j] * A(j, i) end
    y[i] = a
  end
end

local function AtAv(x, y, t, n)
  Av(x, t, n)
  Atv(t, y, n)
end

local u, v, t = {}, {}, {}
for i = 1, N do u[i] = 1.0 end

for i = 1, 10 do
  AtAv(u, v, t, N)
  AtAv(v, u, t, N)
end

local vBv, vv = 0.0, 0.0
for i = 1, N do
  local ui, vi = u[i], v[i]
  vBv = vBv + ui * vi
  vv = vv + vi * vi
end
print(string.format("%.9f", math.sqrt(vBv / vv)))
