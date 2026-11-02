console.log(5 & 3, 5 | 3, 5 ^ 3, ~5, ~-1, ~0);
console.log(1 << 31, 1 << 32, 1 << 33, -1 << 31, 3 << 30);
console.log(-16 >> 2, -16 >>> 2, -1 >>> 0, -1 >>> 31, 16 >> 33, 16 >>> 32);
console.log(0xffffffff | 0, 0x80000000 | 0, 0x7fffffff + 1 | 0, 4294967296 | 0, 4294967297 | 0);
console.log(3.7 | 0, -3.7 | 0, 2147483648.5 | 0, -2147483649 | 0, 1e10 | 0, 1e20 | 0, -1e20 | 0);
console.log(NaN | 0, Infinity | 0, -Infinity | 0, ~NaN, 1 / (-0 | 0));
console.log(0xdeadbeef & 0xffff, 0xdeadbeef >>> 16, 0xdeadbeef >> 16, 0xdeadbeef ^ 0xffffffff);
console.log(~~3.9, ~~-3.9, ~~1e10, ~~"x".length);
console.log(1 << -1, 1 >> -1, 8 >>> -29, 1 << 1.9, 256 >> 4.9);
console.log(5 & -1, -5 & 0xff, -5 | 0, -5 >>> 0, (-5 >>> 0) | 0);
var a = 0x12345678;
var rot = (a << 8) | (a >>> 24);
console.log(rot, rot >>> 0, (rot >>> 0).toString(16));
var h = 0;
for (var i = 0; i < 1000; i++) { h = (h * 31 + i) | 0; }
console.log(h);
var u = 2166136261;
for (var j = 0; j < 50; j++) { u = (u ^ j) >>> 0; u = (u * 16777619) >>> 0; }
console.log(u);
var x = 7;
x &= 3; console.log(x);
x |= 12; console.log(x);
x ^= 5; console.log(x);
x <<= 4; console.log(x);
x >>= 2; console.log(x);
x = -x; x >>>= 28; console.log(x);
console.log(1 + 2 << 3, 1 << 2 + 3, 6 & 3 | 8, 6 | 3 & 8, 6 ^ 3 & 5, 1 | 6 >> 1);
console.log((5 & 3) === 1, 5 & 3 + 1, 12 - 4 >>> 1, -8 >> 1 + 1);
