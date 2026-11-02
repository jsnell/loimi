var vals = [false, true, 0, -0, 1, -1, 0.5, NaN, Infinity, "", "0", "false", " ", null, undefined, {}, [], [0], function () {}];
var names = ["false", "true", "0", "-0", "1", "-1", "0.5", "NaN", "Infinity", "''", "'0'", "'false'", "' '", "null", "undefined", "{}", "[]", "[0]", "fn"];
for (var i = 0; i < vals.length; i++) {
  var v = vals[i];
  console.log(names[i], v ? "truthy" : "falsy", !v, !!v, typeof v);
}
class C {}
console.log(typeof C, typeof new C(), typeof Math, typeof Math.floor, typeof console.log, typeof Object, typeof (x => x));
console.log(typeof typeof 1, typeof [].length, typeof "s".length, typeof "abc"[0]);
console.log(typeof null, typeof undefined, typeof void 0, void 0, void "x");
if ("") { console.log("bad"); } else { console.log("empty string falsy"); }
var n = 0;
while (!n) { n++; }
console.log(n);
console.log(!0, !1, !"", !"a", !null, !undefined, !NaN, !{});
