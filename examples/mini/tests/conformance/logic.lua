print(nil and 1, false and 1, 1 and 2, 0 and "zero", "" and "empty")
print(nil or 1, false or nil, 1 or 2, false or false)
print(not nil, not false, not 0, not "", not {})
local calls = 0
local function f(v) calls = calls + 1; return v end
local r = f(false) and f(1)
print(r, calls)
r = f(1) or f(2)
print(r, calls)
r = f(nil) or f(false) or f("last")
print(r, calls)
local x = nil
local d = x and x.field or "default"
print(d)
if 0 then print("0 is true") end
if "" then print("empty string is true") end
