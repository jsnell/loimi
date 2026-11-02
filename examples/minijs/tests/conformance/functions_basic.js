console.log(hoisted(3));
function hoisted(x) { return x * 2; }
function noReturn() { var x = 1; }
function emptyReturn() { return; }
console.log(noReturn(), emptyReturn());
function args(a, b, c) { return String(a) + "," + String(b) + "," + String(c); }
console.log(args(1), args(1, 2), args(1, 2, 3, 4, 5), args());
var expr = function (x) { return x + 1; };
var named = function fact(n) { return n <= 1 ? 1 : n * fact(n - 1); };
console.log(expr(1), named(10));
var a1 = x => x * x;
var a2 = (x, y) => x + y;
var a3 = () => 42;
var a4 = (x) => { var y = x + 1; return y * 2; };
var a5 = () => ({ obj: 1 });
var a6 = x => y => z => x + y + z;
console.log(a1(5), a2(2, 3), a3(), a4(4), a5().obj, a6(1)(2)(3));
function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
console.log(fib(20));
function isEven(n) { return n === 0 ? true : isOdd(n - 1); }
function isOdd(n) { return n === 0 ? false : isEven(n - 1); }
console.log(isEven(10), isOdd(7), isEven(7));
function apply(f, v) { return f(v); }
console.log(apply(Math.sqrt, 16), apply(String, 5) + 1, apply(a1, 9));
function makeAdder(n) { return function (x) { return x + n; }; }
var adders = [makeAdder(1), makeAdder(10), makeAdder(100)];
console.log(adders[0](1), adders[1](1), adders[2](1), adders.map(f => f(5)).join(","));
function depth(n) { return n === 0 ? 0 : 1 + depth(n - 1); }
console.log(depth(5000));
function outer() {
  return inner();
  function inner() { return "inner-hoisted"; }
}
console.log(outer());
var fnObj = function () {};
fnObj.prop = "functions are objects";
console.log(fnObj.prop, typeof fnObj);
(function () { console.log("iife"); })();
((x) => { console.log("arrow iife", x); })(7);
