local t = {}
table.insert(t, "a")
table.insert(t, "b")
table.insert(t, 3)
print(#t, t[1], t[2], t[3])
local r = table.remove(t)
print(r, #t)
r = table.remove(t)
print(r, #t, t[1])
r = table.remove(t)
print(r, #t)
for i = 1, 10 do table.insert(t, i * i) end
print(#t, t[10])
print(table.concat(t, ","))
print(table.concat(t))
print(table.concat({}), table.concat({}, ","), table.concat({"x"}, ","))
print(table.concat({1, 2.5, "s", 3.0}, " "))
print(table.concat({"a", "b", "c"}, ", "))
print(table.concat({"a", "b"}, ""))
-- insert/remove interplay with #
local s = {}
for i = 1, 5 do table.insert(s, i) end
while #s > 2 do table.remove(s) end
print(table.concat(s, "-"))
-- insert a table, remove returns the same object
local inner = {}
table.insert(s, inner)
print(table.remove(s) == inner)
