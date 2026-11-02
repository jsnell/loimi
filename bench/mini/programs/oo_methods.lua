-- oo-methods: object-oriented code with method calls (obj:m()) on tables
-- of functions, without metatables; "classes" copy their methods into each
-- instance and subclasses override some of them. Inspired by the classic
-- Richards/DeltaBlue style of method-heavy benchmarks; original code.
local N = 5000000

local function Shape_new(kind, a, b)
  local self = { kind = kind, a = a, b = b, moved = 0 }
  function self:area() return self.a * self.b end
  function self:perimeter() return 2 * (self.a + self.b) end
  function self:move(d) self.moved = self.moved + d; return self end
  function self:describe(acc) return acc + self:area() + self:perimeter() end
  return self
end

local function Square_new(s)
  local self = Shape_new("square", s, s)
  function self:perimeter() return 4 * self.a end
  return self
end

local function Tri_new(a, h)
  local self = Shape_new("tri", a, h)
  function self:area() return (self.a * self.b) / 2 end
  return self
end

local Account = {}
function Account.new(owner)
  return { owner = owner, balance = 0, ops = 0,
           deposit = Account.deposit, withdraw = Account.withdraw,
           apply = Account.apply }
end
function Account.deposit(self, n) self.balance = self.balance + n; self.ops = self.ops + 1 end
function Account:withdraw(n)
  if n <= self.balance then
    self.balance = self.balance - n
    self.ops = self.ops + 1
    return true
  end
  return false
end
function Account:apply(i)
  if i % 3 == 0 then
    if not self:withdraw(i % 50) then self:deposit(i % 70) end
  else
    self:deposit(i % 13)
  end
end

local shapes = {}
for i = 1, 300 do
  local k = i % 3
  if k == 0 then shapes[i] = Shape_new("rect", i % 17 + 1, i % 11 + 1)
  elseif k == 1 then shapes[i] = Square_new(i % 13 + 1)
  else shapes[i] = Tri_new(i % 19 + 2, i % 7 + 2) end
end

local acc = 0.0
local nshapes = #shapes
for i = 1, N do
  local s = shapes[i % nshapes + 1]
  acc = s:move(1):describe(acc)
end

local accounts = {}
for i = 1, 50 do accounts[i] = Account.new("o" .. i) end
for i = 1, N do
  accounts[i % 50 + 1]:apply(i)
end
local bal, ops = 0, 0
for i = 1, #accounts do
  bal = bal + accounts[i].balance
  ops = ops + accounts[i].ops
end
local moved = 0
for i = 1, nshapes do moved = moved + shapes[i].moved end

print(string.format("acc %.1f moved %d balance %d ops %d", acc, moved, bal, ops))
