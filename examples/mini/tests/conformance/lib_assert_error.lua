local v = assert(5)
print(v)
v = assert("s", "message")
print(v)
local t = {}
print(assert(t) == t)
print(assert(0), assert(""))
print("all asserts passed")
