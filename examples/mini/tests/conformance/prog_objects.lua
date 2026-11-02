-- OO without metatables: constructors copy methods into instances
local function Stack()
  local self = {items = {}, size = 0}
  function self:push(v) self.size = self.size + 1; self.items[self.size] = v end
  function self:pop()
    local v = self.items[self.size]
    self.items[self.size] = nil
    self.size = self.size - 1
    return v
  end
  function self:empty() return self.size == 0 end
  return self
end

local function Queue()
  local first, last, items = 1, 0, {}
  local q = {}
  q.push = function(v) last = last + 1; items[last] = v end
  q.pop = function()
    if first > last then return nil end
    local v = items[first]
    items[first] = nil
    first = first + 1
    return v
  end
  q.len = function() return last - first + 1 end
  return q
end

local s = Stack()
for i = 1, 5 do s:push(i * i) end
local out = {}
while not s:empty() do out[#out + 1] = s:pop() end
print(table.concat(out, " "))

local q = Queue()
for i = 1, 5 do q.push("q" .. i) end
local a1 = q.pop()
local a2 = q.pop()
print(a1, a2, q.len())
q.push("q6")
out = {}
while q.len() > 0 do out[#out + 1] = q.pop() end
print(table.concat(out, " "), q.pop())

-- "inheritance" by copying and overriding
local function Animal(name)
  local a = {name = name}
  function a:speak() return self.name .. " makes a sound" end
  function a:intro() return "I am " .. self.name .. "; " .. self:speak() end
  return a
end
local function Dog(name)
  local d = Animal(name)
  function d:speak() return self.name .. " barks" end
  return d
end
print(Animal("cat"):intro())
print(Dog("rex"):intro())
