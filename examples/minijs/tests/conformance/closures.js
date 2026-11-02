function counter() { var n = 0; return { inc: function () { return ++n; }, get: () => n }; }
var c1 = counter(), c2 = counter();
c1.inc(); c1.inc(); c2.inc();
console.log(c1.get(), c2.get());
var fs = [];
for (let i = 0; i < 3; i++) { fs.push(() => i); }
console.log(fs.map(f => f()).join(","));
var gs = [];
for (var j = 0; j < 3; j++) { gs.push(() => j); }
console.log(gs.map(f => f()).join(","));
var hs = [];
for (let i = 0; i < 4; i++) { hs.push(() => i); i++; }
console.log(hs.map(f => f()).join(","));
var ks = [];
for (let i = 0; i < 3; i++) { let k = i * 10; ks.push(function () { return k + i; }); }
console.log(ks.map(f => f()).join(","));
var ms = [];
for (const x of [5, 6, 7]) { ms.push(() => x); }
console.log(ms.map(f => f()).join(","));
var ns = [];
var obj = { a: 1, b: 2 };
for (const key in obj) { ns.push(() => key); }
console.log(ns.map(f => f()).join(","));
var ws = [];
var w = 0;
while (w < 3) { let captured = w; ws.push(() => captured); w++; }
console.log(ws.map(f => f()).join(","));
var mut = [];
for (let i = 0; i < 3; i++) { mut.push(() => i++); }
console.log(mut[0](), mut[0](), mut[1](), mut[2]());
function makeAcc() { var total = 0; return function (v) { total += v; return total; }; }
var acc = makeAcc();
acc(5); acc(10);
console.log(acc(0));
var shared = (function () {
  var secret = 0;
  return { set(v) { secret = v; }, read() { return secret; } };
})();
shared.set(42);
console.log(shared.read());
function compose(f, g) { return x => f(g(x)); }
var inc = x => x + 1, dbl = x => x * 2;
console.log(compose(inc, dbl)(5), compose(dbl, inc)(5));
function memo(f) {
  var cache = {};
  return function (n) { if (n in cache) { return cache[n]; } var r = f(n); cache[n] = r; return r; };
}
var calls = 0;
var slowSq = memo(function (n) { calls++; return n * n; });
slowSq(4); slowSq(4); slowSq(5);
console.log(calls, slowSq(5));
var fib = memo(n => n < 2 ? n : fib(n - 1) + fib(n - 2));
console.log(fib(80));
let outerLet = "outer";
function readOuter() { return outerLet; }
outerLet = "changed";
console.log(readOuter());
var level1 = function (a) { return function (b) { return function (c) { return function (d) { return a + b + c + d; }; }; }; };
console.log(level1("a")("b")("c")("d"));
var fns = {};
["x", "y"].forEach(function (name, idx) { fns[name] = () => name + idx; });
console.log(fns.x(), fns.y());
