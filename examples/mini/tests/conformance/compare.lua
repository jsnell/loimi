print(1 < 2, 2 < 1, 1 <= 1, 1 >= 2, 2 > 1, 1 == 1, 1 ~= 1)
print(1 == 1.0, 1 < 1.5, -0.0 == 0, 1 / 0 > 2147483647, -1 / 0 < (-2147483647 - 1))
print("a" < "b", "abc" < "abd", "" < "a", "Z" < "a", "a" < "a\0", "10" < "9")
print("abc" == "abc", "abc" ~= "abd", "a" <= "a", "b" >= "a")
print(1 == "1", "1" == 1, nil == false, false == false, nil == nil)
local t1, t2 = {}, {}
local t3 = t1
print(t1 == t2, t1 == t3, t1 ~= t2)
local f1 = function() end
local f2 = function() end
print(f1 == f2, f1 == f1, print == print)
print(true == true, true ~= false)
