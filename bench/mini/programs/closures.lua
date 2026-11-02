-- closures: closure creation, upvalue reads/writes, shared upvalues,
-- per-iteration loop variables and higher-order functions. Original.
local N = 1500000

local function make_counter(start)
  local n = start
  local function inc(d) n = n + d; return n end
  local function get() return n end
  return inc, get
end

local function compose(f, g)
  return function(x) return f(g(x)) end
end

local function adder(k) return function(x) return x + k end end
local function scaler(k) return function(x) return (x * k) % 65521 end end

local function map(t, f)
  local r = {}
  for i = 1, #t do r[i] = f(t[i]) end
  return r
end

local function fold(t, f, acc)
  for i = 1, #t do acc = f(acc, t[i]) end
  return acc
end

local total = 0
for round = 1, N do
  local inc, get = make_counter(round % 100)
  inc(1); inc(2); inc(3)
  local f = compose(adder(round % 7), scaler(3))
  total = (total + get() + f(round % 1000)) % 1000000007
end

-- closures capturing fresh loop variables
local fs = {}
for i = 1, 1000 do
  fs[i] = function(x) return x + i end
end
local base = {}
for i = 1, 100 do base[i] = i end
local plus = function(a, b) return (a + b) % 1000003 end
for rep = 1, 60 do
  for i = 1, #fs, 37 do
    local mapped = map(base, fs[i])
    total = (total + fold(mapped, plus, rep)) % 1000000007
  end
end

-- shared upvalue between sibling closures, mutated in a loop
local acc = 0
local function bump(x) acc = (acc + x) % 99991 end
local function twice(x) bump(x); bump(x + 1) end
for i = 1, N * 2 do twice(i % 1000) end

print(string.format("total %d acc %d", total, acc))
