-- fib: naive doubly-recursive Fibonacci. Stresses function calls and returns.
local N = 37

local function fib(n)
  if n < 2 then return n end
  return fib(n - 1) + fib(n - 2)
end

print(string.format("fib(%d) = %d", N, fib(N)))
