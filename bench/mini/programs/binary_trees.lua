-- binary-trees (Computer Language Benchmarks Game): allocation and GC.
local N = 16

local function bottom_up(depth)
  if depth == 0 then return {} end
  depth = depth - 1
  return { bottom_up(depth), bottom_up(depth) }
end

local function check(tree)
  if tree[1] then
    return 1 + check(tree[1]) + check(tree[2])
  end
  return 1
end

local mindepth = 4
local maxdepth = mindepth + 2
if maxdepth < N then maxdepth = N end

do
  local stretch = maxdepth + 1
  local t = bottom_up(stretch)
  print(string.format("stretch tree of depth %d\t check: %d", stretch, check(t)))
end

local longlived = bottom_up(maxdepth)

local depth = mindepth
while depth <= maxdepth do
  local iters = 1
  for i = 1, maxdepth - depth + mindepth do iters = iters * 2 end
  local c = 0
  for i = 1, iters do
    c = c + check(bottom_up(depth))
  end
  print(string.format("%d\t trees of depth %d\t check: %d", iters, depth, c))
  depth = depth + 2
end

print(string.format("long lived tree of depth %d\t check: %d", maxdepth, check(longlived)))
