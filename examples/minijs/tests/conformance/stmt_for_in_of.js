var arr = [10, 20, 30];
var s = [];
for (const v of arr) { s.push(v * 2); }
console.log(s.join(","));
for (let v of arr) { v += 1; s.push(v); }
console.log(s.join(","), arr.join(","));
var empty = 0;
for (const v of []) { empty++; }
for (const k in {}) { empty++; }
console.log("empty", empty);
var o = { x: 1, y: "two", z: [3] };
var parts = [];
for (const k in o) { parts.push(k + "=" + String(o[k])); }
console.log(parts.join("&"));
var grow = [1, 2, 3];
var seen = [];
for (const v of grow) { seen.push(v); if (v < 3) { grow.push(v + 10); } }
console.log(seen.join(","));
var strs = ["a", "b"];
var cat = "";
for (const ch of strs) { for (const ch2 of strs) { cat += ch + ch2 + " "; } }
console.log(cat);
var nested = [[1, 2], [3], []];
var flat = [];
for (const row of nested) { for (const x of row) { flat.push(x); } }
console.log(flat.join(","));
function Proto() { this.own1 = 1; this.own2 = 2; }
var inst = new Proto();
var ik = [];
for (const k in inst) { ik.push(k); }
console.log(ik.join(","));
var del = { a: 1, b: 2, c: 3 };
var dk = [];
for (const k in del) { dk.push(k); }
console.log(dk.join(","));
var acc = 0;
for (const v2 of arr) { acc += v2; }
console.log(acc);
var ret = (function () { for (const v of [1, 2, 3, 4]) { if (v === 3) { return "returned " + v; } } return "none"; })();
console.log(ret);
