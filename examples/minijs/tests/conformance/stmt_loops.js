var s = 0;
for (var i = 0; i < 10; i++) { s += i; }
console.log("for", s, i);
var w = 0, n = 0;
while (n < 5) { w += n; n++; }
console.log("while", w, n);
var d = 0;
do { d++; } while (d < 0);
console.log("do-once", d);
var dd = 0;
do { dd += 2; } while (dd < 10);
console.log("do", dd);
var out = [];
for (var a = 0; a < 10; a++) {
  if (a % 2 === 0) { continue; }
  if (a > 7) { break; }
  out.push(a);
}
console.log("cont/break", out.join(","));
var k = 0;
for (;;) { k++; if (k === 4) { break; } }
console.log("for(;;)", k);
var m = 0;
for (; m < 3;) { m++; }
console.log("for(;c;)", m);
var x = 0, y = 10;
for (x = 0, y = 10; x < y; x += 2, y -= 1) {}
console.log("comma header", x, y);
var cnt = 0;
while (true) { cnt++; if (cnt >= 3) break; }
console.log("while(true)", cnt);
var e = 0;
do { e++; if (e === 2) { continue; } if (e > 4) { break; } } while (e < 10);
console.log("do-continue", e);
for (let i2 = 0; i2 < 2; i2++) { for (let j2 = 0; j2 < 2; j2++) { console.log("nested", i2, j2); } }
var sum = 0;
for (let p = 0, q = 10; p < q; p++, q--) { sum += q - p; }
console.log("let-two", sum);
var count = 0;
for (var z = 10; z--;) { count++; }
console.log("decrement-cond", count, z);
var fl = 0;
for (var f = 0; f < 1; f += 0.25) { fl++; }
console.log("float-step", fl);
var body = 0;
while (body < 3) body++;
console.log("braceless", body);
for (var r = 0; r < 3; r++) for (var t = 0; t < 2; t++) body++;
console.log("braceless-nested", body);
