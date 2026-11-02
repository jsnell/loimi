class Vec {
  constructor(x, y) { this.x = x; this.y = y; }
  add(o) { return new Vec(this.x + o.x, this.y + o.y); }
  scale(k) { return new Vec(this.x * k, this.y * k); }
  dot(o) { return this.x * o.x + this.y * o.y; }
  toStr() { return "(" + this.x + ", " + this.y + ")"; }
  static zero() { return new Vec(0, 0); }
  static of(x, y) { return new this(x, y); }
}
var a = new Vec(1, 2), b = new Vec(3, 4);
console.log(a.add(b).toStr(), a.scale(3).toStr(), a.dot(b), Vec.zero().toStr(), Vec.of(5, 6).toStr());
console.log(typeof Vec, typeof a, Object.keys(a).join(","), Object.getPrototypeOf(a) === Vec.prototype);
var keys = [];
for (var k in a) { keys.push(k); }
console.log(keys.join(","), a.hasOwnProperty("add"), "add" in a, Object.keys(Vec.prototype).length);
class Empty {}
var e = new Empty();
console.log(Object.keys(e).length, typeof e);
class NoCtor { hello() { return "hello"; } }
console.log(new NoCtor().hello());
class Stack {
  constructor() { this.items = []; }
  push(v) { this.items.push(v); return this; }
  pop() { return this.items.pop(); }
  get_size() { return this.items.length; }
  static from(arr) { var s = new Stack(); for (const v of arr) { s.push(v); } return s; }
}
var s = new Stack().push(1).push(2).push(3);
console.log(s.pop(), s.get_size(), Stack.from([7, 8, 9]).pop());
class Counter {
  constructor(start) { this.n = start; }
  next() { this.n++; return this.n; }
  ["comp" + "uted"]() { return "computed method"; }
  "string name"() { return "string-named"; }
}
var c = new Counter(10);
c.next(); c.next();
console.log(c.n, c.computed(), c["string name"]());
Vec.prototype.extra = function () { return "patched " + this.x; };
console.log(a.extra());
class ReturnsObj { constructor() { return { replaced: true }; } }
console.log(new ReturnsObj().replaced);
class Static {
  static counter() { Static.count = (Static.count || 0) + 1; return Static.count; }
}
Static.counter(); Static.counter();
console.log(Static.count, Static.counter());
