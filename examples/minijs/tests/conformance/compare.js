console.log(1 < 2, 2 < 1, 1 <= 1, 1 >= 2, 2 > 1, -1 < 0, -Infinity < -1e308, 1e308 < Infinity);
console.log(0 < -0, 0 <= -0, -0 >= 0, NaN < NaN, NaN >= NaN, 1 < NaN);
console.log("a" < "b", "a" < "B", "B" < "a", "abc" < "abd", "abc" < "ab", "ab" < "abc", "" < "a", "a" <= "a");
console.log("10" < "9", "Z" < "a", "a" > "A", "zebra" > "apple", "a" >= "b", " " < "!", "0" < "A");
console.log(0.1 + 0.2 > 0.3, 1 / 3 < 0.34, 2147483647 < 2147483648, -2147483648 > -2147483649);
var xs = ["pear", "apple", "fig", "Banana", "apple2", "app"];
var min = xs[0];
for (var i = 1; i < xs.length; i++) { if (xs[i] < min) { min = xs[i]; } }
console.log(min);
console.log(1 < 2 === true, 1 + 1 < 3, "a" + "b" < "ac");
