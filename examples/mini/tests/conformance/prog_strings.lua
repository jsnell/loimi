-- string processing with byte/sub/char only
local function split(s, sep)
  local out, start = {}, 1
  for i = 1, #s do
    if string.sub(s, i, i) == sep then
      out[#out + 1] = string.sub(s, start, i - 1)
      start = i + 1
    end
  end
  out[#out + 1] = string.sub(s, start)
  return out
end
local function upper(s)
  local r = {}
  for i = 1, #s do
    local c = string.byte(s, i)
    if c >= 97 and c <= 122 then c = c - 32 end
    r[i] = string.char(c)
  end
  return table.concat(r)
end
local function reverse(s)
  local r = {}
  for i = #s, 1, -1 do r[#r + 1] = string.sub(s, i, i) end
  return table.concat(r)
end
local function trim(s)
  local i, j = 1, #s
  while i <= j and string.byte(s, i) == 32 do i = i + 1 end
  while j >= i and string.byte(s, j) == 32 do j = j - 1 end
  return string.sub(s, i, j)
end
local parts = split("alpha,beta,,gamma", ",")
print(#parts, table.concat(parts, "|"))
print(upper("Hello, World 123"), reverse("stressed"), "[" .. trim("   pad me  ") .. "]")
-- word frequency with sorted output
local text = "the quick brown fox jumps over the lazy dog the fox"
local freq = {}
for _, w in ipairs(split(text, " ")) do freq[w] = (freq[w] or 0) + 1 end
local words = {}
for w in pairs(freq) do words[#words + 1] = w end
for i = 2, #words do
  local v, j = words[i], i - 1
  while j >= 1 and words[j] > v do words[j + 1] = words[j]; j = j - 1 end
  words[j + 1] = v
end
for _, w in ipairs(words) do io.write(w, "=", freq[w], " ") end
io.write("\n")
-- number <-> string round trips
local nums = {}
for i = 1, 5 do nums[i] = tostring(i * 1.5) end
print(table.concat(nums, " "))
local back = 0
for i = 1, #nums do back = back + tonumber(nums[i]) end
print(back, string.format("%5.1f|%-6d|%x", back, 42, 255))
