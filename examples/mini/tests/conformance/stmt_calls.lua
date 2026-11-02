local function show(a, b, c) print(a, b, c) end
show(1)
show(1, 2, 3, 4, 5)
show()
local function first(t) return t[1] end
print(first{10, 20})
local function id(s) return s end
print(id"literal", id'single')
local function len(t) return #t end
print(len{1, 2, 3})
-- obj:m evaluates obj once
local evals = 0
local o = {v = 7}
function o:get(x) return self.v + (x or 0) end
local function getobj() evals = evals + 1; return o end
local gv = getobj():get(3)
print(gv, evals)
-- method on a field
local outer = {inner = {v = 1}}
outer.inner.twice = function(self) return self.v * 2 end
print(outer.inner:twice())
-- call through table index
local ops = {add = function(a, b) return a + b end, ["mul"] = function(a, b) return a * b end}
print(ops.add(2, 3), ops["mul"](2, 3))
-- calling the result of a call
local function mk() return function(x) return x + 1 end end
print(mk()(41))
-- parenthesized function expression
print((function(x) return x .. "!" end)("wow"))
-- deep recursion
local function depth(n) if n == 0 then return 0 end return 1 + depth(n - 1) end
print(depth(1000))
