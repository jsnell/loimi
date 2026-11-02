var outcomes = [];
function attempt(name, fn) {
  try { fn(); outcomes.push(name + ":ok"); } catch (e) { outcomes.push(name + ":threw"); }
}
attempt("call-undefined", function () { var f; f(); });
attempt("call-number", function () { var n = 5; n(); });
attempt("call-missing-method", function () { var o = {}; o.nope(); });
attempt("prop-of-undefined", function () { var u; return u.x; });
attempt("prop-of-null", function () { var n = null; return n.x; });
attempt("index-of-undefined", function () { var u; return u[0]; });
attempt("set-on-undefined", function () { var u; u.x = 1; });
attempt("nested-missing", function () { var o = { a: {} }; return o.a.b.c; });
attempt("this-before-super", function () {
  class A { constructor() { this.a = 1; } }
  class B extends A { constructor() { this.b = 2; super(); } }
  return new B();
});
attempt("missing-super", function () {
  class A {}
  class B extends A { constructor() { } }
  return new B();
});
attempt("fine", function () { var o = { f() { return 1; } }; return o.f(); });
attempt("user-throw", function () { throw new Error("mine"); });
console.log(outcomes.join("\n"));
var e;
try { null.x; } catch (err) { e = err; }
console.log(typeof e, typeof e.message);
try { var arr = [1]; arr[5].y; } catch (err2) { console.log("deep index caught"); }
function rethrow() { try { undefined.f(); } catch (err3) { throw new Error("wrapped"); } }
try { rethrow(); } catch (err4) { console.log(err4.message); }
