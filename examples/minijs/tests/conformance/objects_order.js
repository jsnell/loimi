var o = {};
o.z = 1; o[10] = 2; o.a = 3; o[2] = 4; o.b9 = 5; o["x-3"] = 6; o[0] = 7; o["1x"] = 8; o.m = 9; o[123456] = 10; o["1000"] = 11;
console.log(Object.keys(o).join("|"));
var seen = [];
for (var k in o) { seen.push(k); }
console.log(seen.join("|"));
delete o.a;
o.a = 99;
delete o[2];
console.log(Object.keys(o).join("|"));
o.z = 100;
console.log(Object.keys(o).join("|"));
var lit = { b: 1, 3: 1, a: 1, 1: 1, "2": 1 };
console.log(Object.keys(lit).join(","));
var arr = ["x", "y", "z"];
console.log(Object.keys(arr).join(","), typeof Object.keys(arr)[0]);
var ak = [];
for (var i in arr) { ak.push(i + ":" + arr[i]); }
console.log(ak.join(","));
var big = {};
for (var n = 20; n > 0; n -= 3) { big["k" + n] = n; big[n] = n; }
console.log(Object.keys(big).join(","));
