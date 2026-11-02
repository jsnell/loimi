print("a" .. "b" .. "c")
print("n=" .. 1, "f=" .. 1.5, "g=" .. 2.0, 1 .. 2)
print(1 .. "", -0.0 .. "", 1e100 .. "", 2^53 .. "", -7 .. "")
local s = ""
for i = 1, 5 do s = s .. i .. "," end
print(s)
print("" .. "", #("ab" .. "cd"))
print(10 .. 20 + 1)
