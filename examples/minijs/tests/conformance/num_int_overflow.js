// An implementation may use small integers internally; none of this may be observable.
var max = 2147483647, min = -2147483648;
console.log(max + 1, min - 1, -min, max * 2, min * 2, min * -1, max * max, min * min);
var i = max;
i++;
console.log(i, i | 0);
var j = min;
j--;
console.log(j, j | 0);
var z = 0;
var negz = -z;
console.log(1 / negz, 1 / (0 * -5), 1 / (-5 * 0), 1 / (z - 0), 1 / (-z - 0), 1 / (-z + -z));
var k = -3;
console.log(1 / (k % 3), 1 / (3 % 3), 1 / (k * 0), 1 / Math.floor(-0.2) , 1 / Math.ceil(-0.2));
console.log(7 / 2, -7 / 2, 6 / 3, 1 / 8, max / 2, min / -1);
var acc = 1;
for (var n = 0; n < 40; n++) { acc = acc * 3; }
console.log(acc, acc % 1000, (acc | 0), acc > max);
var s = 0;
for (var m = 0; m < 70000; m++) { s += m; }
console.log(s, s * s, (s * s) % 97);
var big = 1073741824;
console.log(big + big, big * 2 + big * 2, (big + big) | 0, (big * 4) | 0, big << 1, big << 2);
console.log(65536 * 65536, 65535 * 65537, 46341 * 46341, 46340 * 46340);
var neg = 0;
neg -= 1;
console.log(neg, neg * neg, -neg, neg * 0 === 0, 1 / (neg * 0));
var q = 10;
q /= 4;
console.log(q, q * 4, q % 1);
console.log(2 ** 31, -(2 ** 31), 2 ** 31 | 0, 2 ** 32 + 5 | 0, -(2 ** 31) - 1 | 0);
console.log(Math.abs(min), Math.abs(min) | 0, -min | 0, ~min, ~max);
var c = 0x7ffffffe;
c += 1; c += 1; c += 1;
console.log(c, c - 3);
var arrIdx = [10, 20, 30];
var idx = 3 / 3;
console.log(arrIdx[idx], arrIdx[idx * 2], arrIdx[0.5 * 4]);
console.log(0.1 * 10 === 1, 0.1 * 3 === 0.3, 9007199254740991 + 2, 9007199254740991 * 2);
