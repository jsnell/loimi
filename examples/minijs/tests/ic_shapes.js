// Inline caches: polymorphic sites, prototype methods that change after a
// site has cached them, deletion (dictionary mode), shadowing, adds.
function P(x, y) { this.x = x; this.y = y; }
P.prototype.sum = function () { return this.x + this.y; };
function Q(y, x) { this.y = y; this.x = x; this.z = 0; }
Q.prototype.sum = function () { return this.x * this.y; };
const objs = [new P(1, 2), new Q(3, 4), { x: 5, y: 6, sum() { return 0; } }, new P(7, 8)];
let total = 0;
for (let r = 0; r < 3; r++) for (const o of objs) total += o.x + o.sum();
console.log(total);

// Replacing a prototype method after the site cached it.
const p = new P(1, 1);
const seen = [];
for (let i = 0; i < 4; i++) {
  if (i === 2) P.prototype.sum = function () { return -1; };
  seen.push(p.sum());
}
console.log(seen.join(","));

// Adding a shadowing own property after a prototype hit was cached.
const seen2 = [];
for (let i = 0; i < 4; i++) {
  if (i === 2) p.sum = () => 99;
  seen2.push(p.sum());
}
console.log(seen2.join(","));

// Adding a property to an intermediate prototype.
class A { who() { return "A"; } }
class B extends A {}
class C extends B {}
const c = new C();
const seen3 = [];
for (let i = 0; i < 4; i++) {
  if (i === 2) B.prototype.who = function () { return "B"; };
  seen3.push(c.who());
}
console.log(seen3.join(","));

// Deleting a property, then reading and writing through cached sites.
function rw(o) { o.a = (o.a || 0) + 1; return o.a + (o.b || 0); }
const d = { a: 1, b: 2, c: 3 };
const out = [rw(d), rw(d)];
delete d.b;
out.push(rw(d), rw(d));
d.b = 10;
out.push(rw(d));
console.log(out.join(" "), Object.keys(d).join(""));

// Many shapes through one site (megamorphic).
let acc = 0;
for (let i = 0; i < 40; i++) {
  const o = {};
  o["k" + (i % 13)] = i;
  o.v = i;
  acc += o.v;
}
console.log(acc);

// Objects with many properties (dictionary mode) and integer keys.
const big = {};
for (let i = 0; i < 100; i++) big["p" + i] = i;
let s = 0;
for (const k in big) s += big[k];
console.log(s, Object.keys(big).length);
const ord = { b: 1, 2: "two", a: 2, 1: "one", 10: "ten" };
console.log(Object.keys(ord).join(","));
for (const k in ord) { if (k === "a") console.log("in has", ord[k]); }
console.log("b" in ord, "zz" in ord, 1 in ord, ord.hasOwnProperty("2"));

// Objects made by one constructor grow; later ones start bigger.
function Grow() { this.a = 1; }
const gs = [];
for (let i = 0; i < 5; i++) {
  const g = new Grow();
  for (let j = 0; j < i; j++) g["f" + j] = j;
  gs.push(Object.keys(g).length);
}
console.log(gs.join(","));
