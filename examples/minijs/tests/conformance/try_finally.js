var log = [];
function f1() { try { log.push("try"); return "from-try"; } finally { log.push("finally"); } }
console.log(f1(), log.join(","));
log = [];
function f2() { try { return "try"; } finally { return "finally-wins"; } }
console.log(f2());
function f3() { try { throw new Error("boom"); } catch (e) { return "caught " + e.message; } finally { log.push("f3-finally"); } }
console.log(f3(), log.join(","));
log = [];
function f4() {
  for (var i = 0; i < 3; i++) {
    try { if (i === 1) { continue; } if (i === 2) { break; } log.push("body" + i); } finally { log.push("fin" + i); }
  }
  return log.join(",");
}
console.log(f4());
log = [];
function f5() {
  try {
    try { throw "inner"; } finally { log.push("inner-finally"); }
  } catch (e) { log.push("outer-catch:" + e); }
  return log.join(",");
}
console.log(f5());
log = [];
function f6() {
  try { throw 1; } catch (e) { log.push("c" + e); throw e + 1; } finally { log.push("fin"); }
}
try { f6(); } catch (e2) { log.push("top" + e2); }
console.log(log.join(","));
function f7() { var x = 1; try { return x; } finally { x = 2; } }
console.log(f7());
function f8() { try { throw "a"; } catch (e) { try { throw "b"; } catch (e) { log.push("inner " + e); } return "outer " + e; } }
log = [];
console.log(f8(), log.join(","));
try { throw { code: 42, msg: "obj" }; } catch (e) { console.log(e.code, e.msg); }
try { throw null; } catch (e) { console.log("null thrown", e === null); }
try { throw undefined; } catch (e) { console.log("undefined thrown", e === undefined); }
try { throw 0; } catch (e) { console.log("zero thrown", e); }
var err = new Error("message text");
console.log(err.message, typeof err, err.message.length);
var e2 = new Error("x");
e2.extra = 5;
try { throw e2; } catch (c) { console.log(c === e2, c.extra, c.message); }
function deep(n) { if (n === 0) { throw new Error("bottom"); } try { return deep(n - 1); } finally { log.push(n); } }
log = [];
try { deep(4); } catch (e) { console.log(e.message, log.join(",")); }
var order = [];
for (const i of [1, 2]) {
  try { order.push("t" + i); if (i === 1) { throw "x"; } } catch (e) { order.push("c" + i); } finally { order.push("f" + i); }
}
console.log(order.join(" "));
try { console.log("no catch, only finally"); } finally { console.log("finally ran"); }
function f9() { lbl: try { break lbl; } finally { log.push("f9fin"); } return "f9 done"; }
log = [];
console.log(f9(), log.join(","));
function f10() { try { return "t"; } catch (e) { return "c"; } }
console.log(f10());
var caughtVar = "outside";
try { throw "inside"; } catch (caughtVar) { console.log(caughtVar); }
console.log(caughtVar);
