-- string-build: string concatenation with `..`, table.concat, string.rep,
-- string.sub, string.byte, string.char, string.format and tostring.
-- Original.
local N = 60000
local ROUNDS = 20

local function checksum(s, h)
  -- rolling hash over every 7th byte; stays well inside 32 bits
  for i = 1, #s, 7 do
    h = (h * 31 + string.byte(s, i)) % 1000003
  end
  return h
end

local h = 0
local total = 0
for round = 1, ROUNDS do
  -- build lines with `..` and tostring
  local parts = {}
  for i = 1, N do
    local line = "item " .. i .. ": " .. tostring(i * round % 977) .. ";"
    parts[#parts + 1] = line
  end
  local big = table.concat(parts, "\n")
  total = total + #big
  h = checksum(big, h)

  -- incremental concatenation of short pieces, flushed into a buffer
  local buf = {}
  local cur = ""
  for i = 1, N do
    cur = cur .. string.char(97 + (i * 7 + round) % 26)
    if #cur >= 64 then
      buf[#buf + 1] = cur
      cur = ""
    end
  end
  buf[#buf + 1] = cur
  local s2 = table.concat(buf)
  total = total + #s2
  h = checksum(s2, h)

  -- substrings and formatting
  local acc = {}
  for i = 1, N, 3 do
    local a = string.sub(s2, i, i + 4)
    acc[#acc + 1] = string.format("%s-%d|%5s", a, i % 100, string.rep("x", i % 4))
  end
  local s3 = table.concat(acc, ",")
  total = total + #s3
  h = checksum(s3, h)
end

print(string.format("length %d checksum %d", total, h))
