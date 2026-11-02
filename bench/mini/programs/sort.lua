-- sort: quicksort, heapsort and insertion sort written in Mini (there is no
-- table.sort), on integer, float and string arrays. Comparisons, array
-- reads/writes, recursion. Original.
local N = 100000
local ROUNDS = 6

local seed = 12345
local function rand(n) -- Numerical Recipes LCG, stays within 31 bits
  seed = (seed * 1366 + 150889) % 714025
  return seed % n
end

local function insertion(a, lo, hi)
  for i = lo + 1, hi do
    local v = a[i]
    local j = i - 1
    while j >= lo and a[j] > v do
      a[j + 1] = a[j]
      j = j - 1
    end
    a[j + 1] = v
  end
end

local function quicksort(a, lo, hi)
  while hi - lo > 16 do
    local mid = math.floor((lo + hi) / 2)
    -- median of three
    if a[mid] < a[lo] then a[mid], a[lo] = a[lo], a[mid] end
    if a[hi] < a[lo] then a[hi], a[lo] = a[lo], a[hi] end
    if a[hi] < a[mid] then a[hi], a[mid] = a[mid], a[hi] end
    local p = a[mid]
    local i, j = lo, hi
    while i <= j do
      while a[i] < p do i = i + 1 end
      while a[j] > p do j = j - 1 end
      if i <= j then
        a[i], a[j] = a[j], a[i]
        i = i + 1
        j = j - 1
      end
    end
    -- recurse into the smaller half, loop on the larger
    if j - lo < hi - i then
      quicksort(a, lo, j)
      lo = i
    else
      quicksort(a, i, hi)
      hi = j
    end
  end
  insertion(a, lo, hi)
end

local function heapsort(a, n)
  local function sift(start, stop)
    local root = start
    while true do
      local child = root * 2
      if child > stop then return end
      if child + 1 <= stop and a[child] < a[child + 1] then child = child + 1 end
      if a[root] < a[child] then
        a[root], a[child] = a[child], a[root]
        root = child
      else
        return
      end
    end
  end
  for start = math.floor(n / 2), 1, -1 do sift(start, n) end
  for stop = n, 2, -1 do
    a[1], a[stop] = a[stop], a[1]
    sift(1, stop - 1)
  end
end

local function check(a, n)
  for i = 2, n do
    if a[i - 1] > a[i] then error("not sorted at " .. i) end
  end
end

local h = 0
local function mix(x) h = (h * 31 + x) % 1000003 end

for round = 1, ROUNDS do
  -- integers, quicksort
  local a = {}
  for i = 1, N do a[i] = rand(1000000) end
  quicksort(a, 1, N)
  check(a, N)
  for i = 1, N, 997 do mix(a[i]) end

  -- floats, heapsort
  local b = {}
  for i = 1, N do b[i] = rand(100000) / 7 end
  heapsort(b, N)
  check(b, N)
  for i = 1, N, 991 do mix(math.floor(b[i] * 7 + 0.5)) end

  -- strings, quicksort (bytewise string comparison)
  local s = {}
  local M = math.floor(N / 4)
  for i = 1, M do
    -- one rand() per statement: argument evaluation order is not specified
    local c1 = 97 + rand(26)
    local c2 = 97 + rand(26)
    local c3 = 97 + rand(26)
    local num = rand(1000)
    s[i] = string.char(c1, c2, c3) .. num
  end
  quicksort(s, 1, M)
  check(s, M)
  for i = 1, M, 499 do mix(string.byte(s[i], 1) * 100 + #s[i]) end

  -- nearly sorted input, insertion sort
  local c = {}
  local K = 3000
  for i = 1, K do c[i] = i * 10 + rand(25) end
  insertion(c, 1, K)
  check(c, K)
  mix(c[1]); mix(c[K])
end

print(string.format("checksum %d", h))
