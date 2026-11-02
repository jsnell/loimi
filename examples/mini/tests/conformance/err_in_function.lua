local function inner(n)
  print("inner", n)
  if n == 3 then error("depth reached") end
  inner(n + 1)
  print("not reached")
end
inner(1)
print("not reached")
