print(string.len(""), string.len("abc"), string.len("a\0b"))
local s = "hello"
print(string.sub(s, 2, 4), string.sub(s, 2), string.sub(s, -3), string.sub(s, -3, -2))
print(string.sub(s, 0), string.sub(s, 1, 0), string.sub(s, 4, 2), string.sub(s, 3, 100))
print(string.sub(s, -100, 2), string.sub(s, 6), string.sub(s, 5, 5), string.sub(s, 1, -1))
print(string.sub("", 1, 1) == "", #string.sub(s, 2, -2))
print(string.byte("A"), string.byte("abc", 1), string.byte("abc", 3), string.byte("abc", -1))
print(string.byte("\0"), string.byte("\255"), string.byte(" "))
local b = string.byte("abc", 10)
print(b)
print(string.char(72, 105), string.char(), #string.char(0, 1, 2), string.char(97))
print(string.char(string.byte("z")) == "z")
print(string.rep("ab", 3), string.rep("x", 1), string.rep("x", 0) == "", string.rep("", 5) == "")
print(#string.rep("abc", 1000), string.rep("-", 10))
-- strings are immutable values
local t = "abc"
local u = t
t = t .. "d"
print(u, t)
