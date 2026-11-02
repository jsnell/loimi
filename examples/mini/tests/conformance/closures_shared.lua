local function pair()
  local n = 0
  local function inc() n = n + 1 end
  local function get() return n end
  return inc, get
end
local inc, get = pair()
inc(); inc()
print(get())
local inc2, get2 = pair()
inc2()
print(get(), get2())

-- closures created in the same scope share variables
local fs = {}
do
  local shared = 0
  fs.add = function(d) shared = shared + d end
  fs.read = function() return shared end
  fs.reset = function() shared = 0 end
end
fs.add(5); fs.add(7)
print(fs.read())
fs.reset()
print(fs.read())

-- a local captured then reassigned in the enclosing function
local x = "before"
local show = function() return x end
x = "after"
print(show())

-- upvalue of an upvalue
local function a()
  local count = 0
  local function b()
    local function c() count = count + 1; return count end
    return c
  end
  return b(), b(), function() return count end
end
local c1, c2, peek = a()
c1(); c2(); c1()
print(peek())
