var a = [1, 2, 3];
console.log(a.length, a[0], a[2], a[3], a[-1], String(a));
a[3] = 4;
a[a.length] = 5;
console.log(a.length, a.join(","));
a[0] = "x";
console.log(a.join(","));
a.length = 2;
console.log(a.length, a.join(","), a[2]);
a.length = 0;
console.log(a.length, "[" + a.join(",") + "]");
var b = [];
for (var i = 0; i < 10; i++) { b[i] = i * i; }
console.log(b.length, b.join(" "));
var n = new Array(4);
console.log(n.length);
for (var j = 0; j < n.length; j++) { n[j] = j + 100; }
console.log(n.join(","));
var z = new Array(0);
z[0] = "first";
console.log(z.length, z[0]);
var mixed = [1, "two", true, null, undefined, [3, 4], { k: 5 }];
console.log(mixed.length, typeof mixed[5], mixed[5][1], mixed[6].k, String(mixed[3]), String(mixed[4]));
var grid = [];
for (var r = 0; r < 3; r++) { var row = []; for (var c = 0; c < 3; c++) { row.push(r * 3 + c); } grid.push(row); }
console.log(grid[1][2], grid[2].join(""), grid.length, String(grid));
var aa = [1, 2];
var ab = aa;
ab.push(3);
console.log(aa.length, aa === ab);
var idx = "1";
console.log(a.length, b[idx], b["2"], b[1.0]);
var big = [];
for (var k = 0; k < 100000; k++) { big.push(k); }
var s = 0;
for (var k2 = 0; k2 < big.length; k2++) { s += big[k2]; }
console.log(big.length, s);
var t = [5, 6, 7];
t.length = 3;
console.log(t.join(","));
console.log([].length, [[]].length, [[], []].length, [undefined].length, typeof [][0]);
