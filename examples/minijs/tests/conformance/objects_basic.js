var o = { a: 1, "b": 2, "with space": 3, 4: "four" };
console.log(o.a, o["b"], o["with space"], o[4], o["4"], o.missing);
o.c = 5;
o["d" + 1] = 6;
console.log(o.c, o.d1, Object.keys(o).join(","));
o.a = 100;
console.log(o.a, Object.keys(o).length);
var a = 1, b = "two";
var short = { a, b };
console.log(short.a, short.b, Object.keys(short).join(","));
var key = "dyn";
var comp = { [key]: 1, [key + "2"]: 2, ["x" + 1 + 2]: 3, [1 + 2]: "three" };
console.log(comp.dyn, comp.dyn2, comp.x12, comp[3], Object.keys(comp).join(","));
var m = {
  n: 10,
  get2() { return this.n * 2; },
  add(x, y) { return this.n + x + y; },
  "quoted"() { return "q"; },
  fn: function () { return "fn" + this.n; },
  arrow: () => "arrow"
};
console.log(m.get2(), m.add(1, 2), m.quoted(), m.fn(), m.arrow(), m["get2"]());
var nested = { p: { q: { r: [1, 2, { s: "deep" }] } } };
console.log(nested.p.q.r[2].s, nested["p"]["q"].r.length);
nested.p.q.r[2].s = "changed";
console.log(nested.p.q.r[2].s);
var alias = nested.p;
alias.extra = true;
console.log(nested.p.extra);
var num = { 1.5: "float", "-1": "neg", 1e3: "thousand" };
console.log(num[1.5], num["1.5"], num[-1], num["-1"], num[1000], Object.keys(num).join(","));
var objKey = {};
var holder = {};
holder[objKey] = "objkey";
console.log(holder["[object Object]"], holder[[1, 2]] === undefined, Object.keys(holder).join(","));
holder[[1, 2]] = "arrkey";
console.log(holder["1,2"]);
var t = {};
t[true] = 1; t[null] = 2; t[undefined] = 3;
console.log(t["true"], t["null"], t["undefined"]);
var e = {};
console.log(Object.keys(e).length, String(e));
