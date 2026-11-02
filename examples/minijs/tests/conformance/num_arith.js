console.log(1 + 2, 7 - 10, 6 * 7, 5 / 2, 1 / 3, 2 / 3);
console.log(10 / 4, -7 / 2, 0.1 * 3, 1.1 + 2.2, 3 - 0.1);
console.log(1 / 0, -1 / 0, String(0 / 0));
console.log(2 * 3 + 4, 2 + 3 * 4, (2 + 3) * 4, 20 - 5 - 3, 64 / 4 / 2);
console.log(-(-5), +7, -(3 - 5), - -2);
var big = 2147483647;
console.log(big + 1, big * 2, -big - 2, big * big);
console.log(4294967296 * 4294967296, 9007199254740992 + 1, 9007199254740992 + 2);
console.log(1e308 * 10, -1e308 * 10, 5e-324 / 2);
console.log(0.1 + 0.2 === 0.3, 0.5 + 0.25 === 0.75);
var i = 0x40000000;
console.log(i + i, i * 4, (i + i) * 2 - 1);
var acc = 0;
for (var k = 0; k < 100; k++) { acc = acc + k * 0.1; }
console.log(acc);
console.log(100 * 1.1, 3 * 1.1, 0.7 + 0.1, 0.2 + 0.4);
console.log(123456789 * 987654321);
