function globalf(a, b) return a + b end
print(globalf(1, 2))

local M = {}
M.sub = {}
M.sub.deep = {}
function M.f(x) return "M.f " .. x end
function M.sub.g(x) return "M.sub.g " .. x end
function M.sub.deep.h(x) return "M.sub.deep.h " .. x end
print(M.f(1), M.sub.g(2), M.sub.deep.h(3))

local obj = {name = "obj", n = 0}
function obj:greet(greeting) return greeting .. ", " .. self.name end
function obj:inc(d) self.n = self.n + (d or 1); return self end
print(obj:greet("hello"))
print(obj.greet(obj, "hi"))
print(obj.greet({name = "other"}, "hey"))
obj:inc():inc(5):inc()
print(obj.n)

-- local function can call itself
local function fact(n) if n <= 1 then return 1 end return n * fact(n - 1) end
print(fact(10))

-- mutual recursion through forward-declared locals
local is_even, is_odd
function is_even(n) if n == 0 then return true end return is_odd(n - 1) end
function is_odd(n) if n == 0 then return false end return is_even(n - 1) end
print(is_even(10), is_odd(7), is_even(3))

-- anonymous function literal
local sq = function(x) return x * x end
print(sq(9))
-- functions as values in tables and arguments
local function apply(f, v) return f(v) end
print(apply(sq, 4), apply(function(x) return -x end, 4))
-- function with no return gives nil
local function nothing() end
print((nothing()))
