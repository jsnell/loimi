local function sort(a)
  for i = 2, #a do
    local v = a[i]
    local j = i - 1
    while j >= 1 and a[j] > v do a[j + 1] = a[j]; j = j - 1 end
    a[j + 1] = v
  end
  return a
end

local t = {apple = 3, banana = 5, cherry = 7}
local keys = {}
for k, v in pairs(t) do keys[#keys + 1] = k .. "=" .. v end
print(table.concat(sort(keys), " "))

-- one variable form drops the value
local ks = {}
for k in pairs({x = 1, y = 2, z = 3}) do ks[#ks + 1] = k end
print(table.concat(sort(ks), ","))

-- array part and hash part together; integer keys come back as integers
local mix = {10, 20, 30, a = 1, [2.0 * 5] = "ten"}
local ik, sk = {}, {}
for k, v in pairs(mix) do
  if type(k) == "number" then ik[#ik + 1] = k else sk[#sk + 1] = k end
end
print(table.concat(sort(ik), " "), table.concat(sort(sk), " "))

-- each pair visited exactly once
local big = {}
for i = 1, 100 do big[i] = i; big["s" .. i] = i end
local count, sum = 0, 0
for k, v in pairs(big) do count = count + 1; sum = sum + v end
print(count, sum)

-- assigning to existing fields during traversal (including nil)
local m = {}
for i = 1, 50 do m[i] = i; m["k" .. i] = i end
for k, v in pairs(m) do
  if v % 2 == 0 then m[k] = nil else m[k] = v * 10 end
end
count, sum = 0, 0
for k, v in pairs(m) do count = count + 1; sum = sum + v end
print(count, sum)

-- empty table
local e = 0
for k, v in pairs({}) do e = e + 1 end
print(e)
-- the table expression is evaluated once
local evals = 0
local function get() evals = evals + 1; return {1, 2, 3} end
for k, v in pairs(get()) do end
print(evals)
