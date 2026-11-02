print(#"", #"abc", #"a\0b\0")
print(#{}, #{1, 2, 3}, #{n = 1}, #{1, 2, x = 3})
local t = {}
for i = 1, 10 do t[#t + 1] = i * i end
print(#t, t[#t])
t[#t] = nil
print(#t, t[#t])
t[#t] = nil
t[#t] = nil
print(#t)
local s = "hello"
print(#s * 2, #s .. "!")
local u = {"a", "b"}
u[3] = "c"
print(#u)
