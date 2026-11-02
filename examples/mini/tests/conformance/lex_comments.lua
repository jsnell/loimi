-- line comment
print("a") -- trailing comment
--[[ long
comment print("no") ]]
print("b")
--[==[ long comment with ]] inside
print("no")
]==]
print("c")
--[=[]=] print("d")
x = 1 --[[ inline ]] + 2
print(x)
---- many dashes
--[ not a long comment, just a line comment
print("e")
