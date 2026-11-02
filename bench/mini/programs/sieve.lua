-- sieve: Sieve of Eratosthenes over a boolean array, repeated.
-- Classic interpreter benchmark. Integer loops and array stores.
local N = 1000000
local REPEAT = 24

local function sieve(n)
  local composite = {}
  for i = 1, n do composite[i] = false end
  local count = 0
  local limit = math.floor(math.sqrt(n)) -- keeps i * i inside 32 bits
  for i = 2, n do
    if not composite[i] then
      count = count + 1
      if i <= limit then
        for j = i * i, n, i do composite[j] = true end
      end
    end
  end
  return count
end

local count
for r = 1, REPEAT do count = sieve(N) end
print(string.format("primes below %d: %d", N + 1, count))
