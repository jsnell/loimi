-- mandelbrot (Computer Language Benchmarks Game): float arithmetic in tight
-- loops. The bitmap bytes are packed arithmetically (Mini has no bitwise
-- operators) and folded into a checksum instead of being written out.
local N = 1300
local ITER = 50
local SWITCH = 650 -- rows from SWITCH on have an integer ci

local h = 0
local inside = 0
local w, hgt = N, N
for y = 0, hgt - 1 do
  local ci = 2.0 * y / hgt - 1.0
  if y >= SWITCH then ci = 0 end
  local byte, bit = 0, 0
  for x = 0, w - 1 do
    local cr = 2.0 * x / w - 1.5
    local zr, zi, zrq, ziq = 0.0, 0.0, 0.0, 0.0
    local escaped = false
    for i = 1, ITER do
      zi = 2.0 * zr * zi + ci
      zr = zrq - ziq + cr
      zrq = zr * zr
      ziq = zi * zi
      if zrq + ziq > 4.0 then escaped = true; break end
    end
    byte = byte * 2
    if not escaped then byte = byte + 1; inside = inside + 1 end
    bit = bit + 1
    if bit == 8 then
      h = (h * 31 + byte) % 1000003
      byte, bit = 0, 0
    end
  end
  if bit > 0 then
    for k = bit, 7 do byte = byte * 2 end
    h = (h * 31 + byte) % 1000003
  end
end

print(string.format("P4 %d %d inside %d checksum %d", w, hgt, inside, h))
