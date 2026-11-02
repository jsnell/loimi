print("before")
local function fail(msg) error(msg) end
fail("boom")
print("not reached")
