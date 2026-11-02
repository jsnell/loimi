var trace = [];
function t(name, v) { trace.push(name); return v; }
function flush(label) { console.log(label, trace.join(" ")); trace = []; }
var r = t("a", 1) + t("b", 2) * t("c", 3);
flush("arith " + r);
function f3(x, y, z) { return x + y + z; }
r = f3(t("x", 1), t("y", 2), t("z", 3));
flush("args " + r);
var obj = { m: function (v) { trace.push("call"); return v; } };
r = t("obj", obj).m(t("arg", 9));
flush("method " + r);
var o = {};
t("target", o)[t("key", "k")] = t("value", 5);
flush("assign " + o.k);
var arr = [t("e0", 0), t("e1", 1), t("e2", 2)];
flush("array " + arr.length);
var lit = { a: t("pa", 1), [t("ck", "b")]: t("pb", 2), c: t("pc", 3) };
flush("object " + Object.keys(lit).join(""));
r = t("l", 1) < t("r", 2);
flush("compare " + r);
var i = 1;
r = i + (i = 10) + i;
console.log("reassign", r);
var j = 0;
var a2 = [0, 0, 0];
a2[j] = (j = 2);
console.log("lhs-first", a2.join(","));
var x = 1;
x += (x = 5);
console.log("compound", x);
var F = function (a, b) { trace.push("ctor"); this.s = a + b; };
r = new (t("F", F))(t("p1", 1), t("p2", 2));
flush("new " + r.s);
r = t("c1", true) ? t("then", 1) : t("else", 2);
flush("cond " + r);
