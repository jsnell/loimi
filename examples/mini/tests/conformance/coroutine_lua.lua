-- Derived from testes/coroutine.lua of the PUC-Lua 5.4.9 test suite
-- (https://www.lua.org/tests/lua-5.4.9-tests.tar.gz, see bench/lua/MANIFEST).
--
-- Copyright (C) 1994-2025 Lua.org, PUC-Rio.
-- Permission is hereby granted, free of charge, to any person obtaining a
-- copy of this software and associated documentation files (the
-- "Software"), to deal in the Software without restriction, including
-- without limitation the rights to use, copy, modify, merge, publish,
-- distribute, sublicense, and/or sell copies of the Software, and to permit
-- persons to whom the Software is furnished to do so, subject to the
-- following conditions: The above copyright notice and this permission
-- notice shall be included in all copies or substantial portions of the
-- Software. THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
-- EXPRESS OR IMPLIED. (Full text: the MIT license in the Lua distribution.)
--
-- Reduced to the Mini subset (examples/mini/SPEC.md plus the coroutine
-- library). General rewrites, applied throughout:
--  * pcall(f, ...) is replaced by try(f, a), which runs f in a fresh
--    coroutine and returns resume's status and error message (a coroutine
--    used as pcall). Where the test needs pcall around a yield from the
--    main thread, or pcall *inside* a coroutine around code that then keeps
--    running in that coroutine, the check is dropped (noted below).
--  * string.find(msg, "x") is replaced by contains(msg, "x"), written with
--    string.sub. Error messages are only ever checked for a substring.
--  * _G.x is replaced by a plain global (gx, gf, F); `undef` by nil.
--  * error(<non-string>) (error(100), error(foo), error{...}) uses string
--    messages instead, checked with contains.
--  * Varargs and table.unpack: fixed parameter lists, explicit t[1], t[2],
--    t[3]; {coroutine.yield(...)} becomes `local r1, r2, r3 = yield(...)`.
--  * math.fmod(n, p) becomes n % p.
--  * `for x in coroutine.wrap(...)` becomes an explicit while loop.
-- Sections removed or rewritten (original line numbers):
--   14       assert(not pcall(coroutine.yield)) from the main thread: removed
--            (no pcall; a coroutine cannot stand in for the main thread).
--   18-19    trivial errors: rewritten with try.
--   34-69    varargs foo / table.unpack / {...} of yield results / select:
--            fixed params, explicit unpacking, element-wise comparison.
--   112      math.fmod -> %.
--   132-222  to-be-closed variables (<close>, setmetatable/__close): removed,
--            except: closing dead coroutines (kept), "cannot close running"
--            and "cannot close normal" (rewritten with try), closing a
--            coroutine after an error (kept, string error value).
--   224-277  <close> versus pcall, errors in __close: removed (<close>,
--            pcall, metatables, debug.getinfo).
--   283      not pcall(table.sort, {1,2,3}, coroutine.yield): rewritten as a
--            nested coroutine whose resume must fail ("C-call boundary").
--   293-319  yields across pcall/xpcall, custom generic-for iterator:
--            removed (pcall, xpcall, varargs, generic for).
--   322-335  unyieldable C call via string.gsub: rewritten with a
--            table.sort comparator (string.gsub not in Mini).
--   339-356  debug.sethook trace: removed (debug library).
--   359-361  debug.getinfo line checks in foo: removed; error(foo) uses a
--            string; pcall(x) -> try(x).
--   394      generic for over coroutine.wrap: explicit while loop.
--   401,411,416-417  weak table C / collectgarbage check: removed
--            (metatables); the closure checks are kept.
--   451-457  <close> plus pcall(A, 1) on a running wrap: rewritten without
--            <close>, with try.
--   461-474  bug in 5.4.1 (<close> during reset): rewritten as two calls to
--            a dead erroring wrap via try.
--   491-492  infinite recursion of coroutines (pcall): rewritten with try.
--   496-513  <close>/xpcall message handler bug: removed.
--   530-759  T (testC) block, including the "testC not active" message:
--            removed.
--   772-785  table.unpack stack limit: removed (table.unpack, 64-bit size).
--   793-973  yields inside metamethods: removed (metatables, bitwise, //,
--            _ENV, debug.setupvalue).
--   975-986  yields inside custom 'for' iterators: removed (generic for
--            only supports pairs/ipairs).
--   990-1173 coroutine C API (T): removed.

print "testing coroutines"

local function contains (s, sub)
  if type(s) ~= "string" then return false end
  local n, m = string.len(s), string.len(sub)
  for i = 1, n - m + 1 do
    if string.sub(s, i, i + m - 1) == sub then return true end
  end
  return false
end

-- run fn(a) in a new coroutine; like pcall(fn, a) for the status and the
-- error message.
local function try (fn, a)
  local st, msg = coroutine.resume(coroutine.create(fn), a)
  return st, msg
end

local f

local main, ismain = coroutine.running()
assert(type(main) == "thread" and ismain)
assert(not coroutine.resume(main))
assert(not coroutine.isyieldable(main) and not coroutine.isyieldable())


-- trivial errors
assert(not try(function () coroutine.resume(0) end))
assert(not try(function () coroutine.status(0) end))


-- tests for multiple yield/resume arguments

local function eqtab3 (t1, t2)
  for i = 1, 3 do
    assert(t1[i] == t2[i])
  end
end

gx = nil
gf = nil
local function foo (a, b, c, d)
  local x, y = coroutine.running()
  assert(x == gf and y == false)
  -- next call should not corrupt coroutine (but must fail,
  -- as it attempts to resume the running coroutine)
  assert(coroutine.resume(gf) == false)
  assert(coroutine.status(gf) == "running")
  local arg = {b, c, d}
  assert(coroutine.isyieldable(x))
  for i=1,#arg do
    local t = arg[i]
    local r1, r2, r3 = coroutine.yield(t[1], t[2], t[3])
    gx = {r1, r2, r3}
  end
  return a[1], a[2], a[3]
end

gf = coroutine.create(foo)
f = gf
assert(coroutine.isyieldable(f))
assert(type(f) == "thread" and coroutine.status(f) == "suspended")
assert(string.sub(tostring(f), 1, 8) == "thread: ")
local s,a,b,c,d
s,a,b,c,d = coroutine.resume(f, {1,2,3}, {}, {1}, {'a', 'b', 'c'})
assert(coroutine.isyieldable(f))
assert(s and a == nil and coroutine.status(f) == "suspended")
s,a,b,c,d = coroutine.resume(f)
eqtab3(gx, {})
assert(s and a == 1 and b == nil)
assert(coroutine.isyieldable(f))
s,a,b,c,d = coroutine.resume(f, 1, 2, 3)
eqtab3(gx, {1, 2, 3})
assert(s and a == 'a' and b == 'b' and c == 'c' and d == nil)
s,a,b,c,d = coroutine.resume(f, "xuxu")
eqtab3(gx, {"xuxu"})
assert(s and a == 1 and b == 2 and c == 3 and d == nil)
assert(coroutine.status(f) == "dead")
s, a = coroutine.resume(f, "xuxu")
assert(not s and contains(a, "dead") and coroutine.status(f) == "dead")

gf = nil

print "yields in tail calls"
local function foo (i) return coroutine.yield(i) end
local f = coroutine.wrap(function ()
  for i=1,10 do
    assert(foo(i) == gx)
  end
  return 'a'
end)
for i=1,10 do gx = i; assert(f(i) == i) end
gx = 'xuxu'; assert(f('xuxu') == 'a')

gx = nil

print "recursive"
local function pf (n, i)
  coroutine.yield(n)
  pf(n*i, i+1)
end

f = coroutine.wrap(pf)
local s=1
for i=1,10 do
  assert(f(1, 1) == s)
  s = s*i
end

print "sieve"
local function gen (n)
  return coroutine.wrap(function ()
    for i=2,n do coroutine.yield(i) end
  end)
end


local function filter (p, g)
  return coroutine.wrap(function ()
    while 1 do
      local n = g()
      if n == nil then return end
      if n % p ~= 0 then coroutine.yield(n) end
    end
  end)
end

local x = gen(80)
local a = {}
while 1 do
  local n = x()
  if n == nil then break end
  table.insert(a, n)
  x = filter(n, x)
end

assert(#a == 22 and a[#a] == 79)
x, a = nil


print("coroutine.close")

do
  -- ok to close a dead coroutine
  local co = coroutine.create(print)
  assert(coroutine.resume(co, "testing 'coroutine.close'"))
  assert(coroutine.status(co) == "dead")
  local st, msg = coroutine.close(co)
  assert(st and msg == nil)
  -- also ok to close it again
  st, msg = coroutine.close(co)
  assert(st and msg == nil)


  -- cannot close the running coroutine
  local st, msg = try(function () coroutine.close(coroutine.running()) end)
  assert(not st and contains(msg, "running"))

  local main = coroutine.running()

  -- cannot close a "normal" coroutine
  local w = coroutine.wrap(function ()
    local st, msg = try(function () coroutine.close(main) end)
    assert(not st and contains(msg, "normal"))
  end)
  w()

  -- closing a coroutine after an error
  local co = coroutine.create(error)
  local st, msg = coroutine.resume(co, "e100")
  assert(not st and contains(msg, "e100"))
  st, msg = coroutine.close(co)
  assert(not st and contains(msg, "e100"))
  -- after closing, no more errors
  st, msg = coroutine.close(co)
  assert(st and msg == nil)
end


print "yielding across C boundaries"

local co = coroutine.wrap(function()
       local st, msg = try(function ()
         table.sort({1,2,3}, coroutine.yield)
       end)
       assert(not st and contains(msg, "C-call boundary"))
       assert(coroutine.isyieldable())
       coroutine.yield(20)
       return 30
     end)

assert(co() == 20)
assert(co() == 30)


-- unyieldable C call
do
  local function f (a, b)
          assert(not coroutine.isyieldable())
          return a < b
        end

  local co = coroutine.wrap(function (c)
               assert(coroutine.isyieldable())
               local t = {c, "b", "a"}
               table.sort(t, f)
               return t[1] .. t[2] .. t[3]
             end)
  assert(co("c") == "abc")
end


print "errors in coroutines"
function foo ()
  coroutine.yield(3)
  error("foo-err")
end

function goo() foo() end
x = coroutine.wrap(goo)
assert(x() == 3)
local a,b = try(x)
assert(not a and contains(b, "foo-err"))

x = coroutine.create(goo)
a,b = coroutine.resume(x)
assert(a and b == 3)
a,b = coroutine.resume(x)
assert(not a and contains(b, "foo-err") and coroutine.status(x) == "dead")
a,b = coroutine.resume(x)
assert(not a and contains(b, "dead") and coroutine.status(x) == "dead")

goo = nil

-- co-routines x for loop
local function all (a, n, k)
  if k == 0 then coroutine.yield(a)
  else
    for i=1,n do
      a[k] = i
      all(a, n, k-1)
    end
  end
end

local a = 0
local it = coroutine.wrap(function () all({}, 5, 4) end)
while true do
  local t = it()
  if t == nil then break end
  a = a+1
end
assert(a == 5^4)


-- access to locals of collected corroutines
local x = coroutine.wrap (function ()
            local a = 10
            local function f () a = a+10; return a end
            while true do
              a = a+1
              coroutine.yield(f)
            end
          end)

local f = x()
assert(f() == 21 and x()() == 32 and x() == f)
x = nil
assert(f() == 43 and f() == 53)


print "old bug: attempt to resume itself"

local function co_func (current_co)
  assert(coroutine.running() == current_co)
  assert(coroutine.resume(current_co) == false)
  coroutine.yield(10, 20)
  assert(coroutine.resume(current_co) == false)
  coroutine.yield(23)
  return 10
end

local co = coroutine.create(co_func)
local a,b,c = coroutine.resume(co, co)
assert(a == true and b == 10 and c == 20)
a,b = coroutine.resume(co, co)
assert(a == true and b == 23)
a,b = coroutine.resume(co, co)
assert(a == true and b == 10)
assert(coroutine.resume(co, co) == false)
assert(coroutine.resume(co, co) == false)


-- other old bug when attempting to resume itself
-- (trigger C-code assertions)
do
  local A = coroutine.running()
  local B = coroutine.create(function() return coroutine.resume(A) end)
  local st, res = coroutine.resume(B)
  assert(st == true and res == false)

  A = coroutine.wrap(function()
    local st, msg = try(function () A(1) end)
    return st, msg
  end)
  st, res = A()
  assert(not st and contains(res, "non-suspended"))
end


-- calling a dead wrap
do
  local co
  co = coroutine.wrap(function()
    error("e111")
  end)
  local st, errobj = try(co)
  assert(not st and contains(errobj, "e111"))
  st, errobj = try(co)
  assert(not st and contains(errobj, "dead coroutine"))
end


-- attempt to resume 'normal' coroutine
local co1, co2
co1 = coroutine.create(function () return co2() end)
co2 = coroutine.wrap(function ()
        assert(coroutine.status(co1) == 'normal')
        assert(not coroutine.resume(co1))
        coroutine.yield(3)
      end)

a,b = coroutine.resume(co1)
assert(a and b == 3)
assert(coroutine.status(co1) == 'dead')

print "infinite recursion of coroutines"
a = function(a) coroutine.wrap(a)(a) end
assert(not try(a, a))
a = nil


-- access to locals of erroneous coroutines
local x = coroutine.create (function ()
            local a = 10
            F = function () a=a+1; return a end
            error('x')
          end)

assert(not coroutine.resume(x))
-- overwrite previous position of local `a'
assert(not coroutine.resume(x, 1, 1, 1, 1, 1, 1, 1))
assert(F() == 11)
assert(F() == 12)
F = nil


-- leaving a pending coroutine open
TO_SURVIVE = coroutine.wrap(function ()
      local a = 10
      local x = function () a = a+1 end
      coroutine.yield()
    end)

TO_SURVIVE()


assert(coroutine.running() == main)

print"+"

print'OK'
