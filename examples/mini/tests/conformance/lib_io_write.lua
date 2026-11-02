io.write("a", "b", "c")
io.write("\n")
io.write(1, " ", 2, " ", -3, "\n")
io.write(1.5, " ", 0.25, " ", -2.5, "\n")
io.write()
io.write("")
io.write("no newline then ")
io.write("newline\n")
local parts = {"x", "y", "z"}
for i = 1, #parts do io.write(parts[i], i < #parts and "," or "\n") end
print("after")
