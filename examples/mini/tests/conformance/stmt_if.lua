local function classify(n)
  if n < 0 then
    return "negative"
  elseif n == 0 then
    return "zero"
  elseif n < 10 then
    return "small"
  elseif n < 100 then
    return "medium"
  else
    return "large"
  end
end
print(classify(-5), classify(0), classify(3), classify(50), classify(1000))
if true then print("then") end
if false then print("no") else print("else") end
if nil then print("no") elseif false then print("no") end
print("after empty if")
if 1 then if nil then print("no") else print("nested else") end end
local x = 5
if x > 3 then local x = "shadow"; print(x) end
print(x)
