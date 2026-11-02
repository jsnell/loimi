console.log(7 % 3, -7 % 3, 7 % -3, -7 % -3);
console.log(5.5 % 2, -5.5 % 2, 5 % 2.5, 0.3 % 0.1);
console.log(String(1 % 0), String(-1 % 0), String(Infinity % 2), 5 % Infinity, -5 % Infinity);
console.log(1 / (0 % 5), 1 / (-0 % 5), 1 / (-4 % 2), 1 / (4 % -2));
console.log(2147483648 % 7, -2147483649 % 10, 1e20 % 7, 1e17 % 3);
for (var i = -6; i <= 6; i += 3) {
  console.log("i", i, i % 4, i % -4);
}
console.log(10 % 3 * 2, 10 * 3 % 7, 13 % 5 % 2);
