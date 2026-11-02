print(type(nil), type(true), type(false))
print(type(0), type(1.5), type(-1), type(1 / 0))
print(type(""), type("x"), type("1"))
print(type({}), type({1}), type(print), type(function() end))
print(type(type), type(math), type(string.sub), type(table))
print(type(type(1)))
local u
print(type(u), type(undefined_name))
