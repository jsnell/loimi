function classify(n) {
  if (n < 0) { return "neg"; } else if (n === 0) { return "zero"; } else if (n < 10) { return "small"; } else { return "big"; }
}
console.log(classify(-5), classify(0), classify(5), classify(50));
if (true) console.log("no braces");
if (false) console.log("never"); else console.log("else no braces");
if (1) { if (0) { console.log("x"); } else { console.log("dangling else binds inner"); } }
function sw(v) {
  var out = [];
  switch (v) {
    case 1: out.push("one");
    case 2: out.push("two"); break;
    case "1": out.push("string one"); break;
    case 3:
    case 4: out.push("three-or-four");
    default: out.push("default");
    case 5: out.push("five"); break;
    case 6: { out.push("six-block"); break; }
  }
  return out.join("+");
}
for (const v of [1, 2, "1", 3, 4, 5, 6, 7, "2", null]) { console.log(String(v), sw(v)); }
function noDefault(v) { switch (v) { case "a": return "A"; case "b": return "B"; } return "none"; }
console.log(noDefault("a"), noDefault("b"), noDefault("c"));
function defFirst(v) { var r = ""; switch (v) { default: r += "d"; case 1: r += "1"; break; case 2: r += "2"; } return r; }
console.log(defFirst(1), defFirst(2), defFirst(3));
var evals = [];
function tag(x) { evals.push(x); return x; }
switch (tag(3)) { case tag(1): break; case tag(2): break; case tag(3): evals.push("hit"); break; case tag(4): break; }
console.log(evals.join(","));
function nanCase(v) { switch (v) { case NaN: return "nan-matched"; default: return "no-match"; } }
console.log(nanCase(NaN), nanCase(0));
function zeroCase(v) { switch (v) { case 0: return "zero"; default: return "other"; } }
console.log(zeroCase(-0));
var obj = {};
function objCase(v) { switch (v) { case obj: return "same obj"; case "x": return "x"; default: return "other"; } }
console.log(objCase(obj), objCase({}));
for (var i = 0; i < 5; i++) {
  switch (i % 3) { case 0: continue; case 1: break; default: console.log("i default", i); }
  console.log("after switch", i);
}
function scoped(v) { switch (v) { case 1: { let t = "block1"; return t; } case 2: { let t = "block2"; return t; } } return "-"; }
console.log(scoped(1), scoped(2), scoped(3));
;;;
{ }
{ var inBlock = "visible"; }
console.log(inBlock);
