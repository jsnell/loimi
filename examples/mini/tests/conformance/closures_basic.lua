local function counter()
  local n = 0
  return function() n = n + 1; return n end
end
local c1, c2 = counter(), counter()
-- each call is a separate statement: argument evaluation order is not specified
local r1 = c1(); local r2 = c1(); local r3 = c1()
local r4 = c2(); local r5 = c1(); local r6 = c2()
print(r1, r2, r3, r4, r5, r6)

local function adder(x) return function(y) return x + y end end
local add5 = adder(5)
print(add5(1), add5(10), adder(-1)(1))

-- upvalue changed after closure creation is visible
local v = 1
local function getv() return v end
v = 2
print(getv())

-- closures see later assignments to captured parameters
local function mk(p)
  local f = function() return p end
  p = p * 10
  return f
end
print(mk(4)())

-- three levels of nesting
local function outer()
  local a = 1
  return function()
    local b = 10
    return function()
      a = a + 1
      b = b + 1
      return a + b
    end
  end
end
local mid = outer()
local in1 = mid()
local in2 = mid()
local v1 = in1(); local v2 = in1(); local v3 = in2(); local v4 = in1()
print(v1, v2, v3, v4)
