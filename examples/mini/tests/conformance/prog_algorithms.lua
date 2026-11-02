-- memoized fibonacci with a table
local memo = {}
local function fib(n)
  if n <= 2 then return 1 end
  local v = memo[n]
  if v then return v end
  v = fib(n - 1) + fib(n - 2)
  memo[n] = v
  return v
end
print(fib(40), fib(46))
-- gcd / primes
local function gcd(a, b) while b ~= 0 do a, b = b, a % b end return a end
print(gcd(48, 18), gcd(17, 5), gcd(0, 9))
local primes = {}
for n = 2, 60 do
  local p = true
  for d = 2, n - 1 do
    if d * d > n then break end
    if n % d == 0 then p = false; break end
  end
  if p then primes[#primes + 1] = n end
end
print(table.concat(primes, " "))
-- matrix multiply
local function matmul(a, b)
  local n, m, p = #a, #b, #b[1]
  local c = {}
  for i = 1, n do
    c[i] = {}
    for j = 1, p do
      local s = 0
      for k = 1, m do s = s + a[i][k] * b[k][j] end
      c[i][j] = s
    end
  end
  return c
end
local c = matmul({{1, 2}, {3, 4}}, {{5, 6}, {7, 8}})
print(c[1][1], c[1][2], c[2][1], c[2][2])
-- binary search
local arr = {}
for i = 1, 100 do arr[i] = i * 3 end
local function bsearch(t, x)
  local lo, hi = 1, #t
  while lo <= hi do
    local mid = math.floor((lo + hi) / 2)
    if t[mid] == x then return mid
    elseif t[mid] < x then lo = mid + 1
    else hi = mid - 1 end
  end
  return nil
end
print(bsearch(arr, 150), bsearch(arr, 151), bsearch(arr, 3), bsearch(arr, 300))
-- Newton square root with floats
local function newton(x)
  local g = x / 2
  for i = 1, 20 do g = (g + x / g) / 2 end
  return g
end
print(string.format("%.10f", newton(2)), newton(16))
