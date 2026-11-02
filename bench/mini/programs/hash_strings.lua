-- hash-strings: hash tables with string keys -- insertion, lookup, update,
-- deletion (assigning nil) and pairs traversal. Original (word-frequency style).
local N = 400000
local REPEAT = 120

local seed = 42
local function rand(n) -- Numerical Recipes LCG, stays within 31 bits
  seed = (seed * 1366 + 150889) % 714025
  return seed % n
end

local syll = { "ka", "lo", "mi", "ne", "su", "ta", "ri", "po", "an", "el",
               "or", "ul", "zi", "ve", "qu", "xo" }

local function word()
  local n = 1 + rand(3)
  local w = syll[1 + rand(16)]
  for i = 2, n do w = w .. syll[1 + rand(16)] end
  return w
end

-- pre-generate a word stream so the timed part is mostly table work
local words = {}
for i = 1, N do words[i] = word() end

local counts = {}
local distinct = 0
for rep = 1, REPEAT do
  for i = 1, N do
    local w = words[i]
    local c = counts[w]
    if c then
      counts[w] = c + 1
    else
      counts[w] = 1
      distinct = distinct + 1
    end
  end
  -- delete some words (count + length divisible by 7)
  for k, v in pairs(counts) do
    if (v + #k) % 7 == 0 then
      counts[k] = nil
      distinct = distinct - 1
    end
  end
end

-- order-independent checksum over pairs
local sum, nkeys = 0, 0
for k, v in pairs(counts) do
  nkeys = nkeys + 1
  local h = 0
  for i = 1, #k do h = (h * 31 + string.byte(k, i)) % 65521 end
  sum = (sum + h * (v % 1000)) % 1000003
end

-- record-style tables with string field names
local recs = {}
for i = 1, 20000 do
  recs[i] = { name = words[i], len = #words[i], id = i }
end
local fieldsum = 0
for rep = 1, 10 do
  for i = 1, #recs do
    local r = recs[i]
    fieldsum = (fieldsum + r.len * r.id + (counts[r.name] or 0)) % 1000000007
  end
end

print(string.format("distinct %d keys %d sum %d fields %d", distinct, nkeys, sum, fieldsum))
