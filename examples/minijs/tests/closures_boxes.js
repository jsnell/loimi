// Closure capture: flat closures, boxes for assigned variables, hoisted
// functions that capture each other, per-iteration bindings.
function counter() {
  let n = 0;
  return { inc: () => ++n, get: () => n };
}
const c1 = counter(), c2 = counter();
c1.inc(); c1.inc(); c2.inc();
console.log(c1.get(), c2.get());

function hoisted() {
  function isEven(n) { return n === 0 ? true : isOdd(n - 1); }
  function isOdd(n) { return n === 0 ? false : isEven(n - 1); }
  return isEven(10) + "," + isOdd(7) + "," + isEven(3);
}
console.log(hoisted());

function usesLaterLet() {
  function get() { return later; }
  let later = "set later";
  return get();
}
console.log(usesLaterLet());

function capturedBeforeInit() {
  const f = () => g() + 1;
  const g = () => 41;
  return f();
}
console.log(capturedBeforeInit());

const fns = [];
for (let i = 0; i < 4; i++) { fns.push(() => i * 10); }
console.log(fns.map(f => f()).join(" "));

const fns2 = [];
for (let i = 0; i < 3; i++) { let j = i; fns2.push(() => j++); }
console.log(fns2[0](), fns2[0](), fns2[1](), fns2[2]());

const fns3 = [];
for (const x of ["a", "b", "c"]) { fns3.push(() => x); }
console.log(fns3.map(f => f()).join(""));

var shared = [];
for (var k = 0; k < 3; k++) { shared.push(() => k); }
console.log(shared.map(f => f()).join(""));

function makeAdders() {
  const out = [];
  let base = 100;
  for (let i = 1; i <= 3; i++) out.push(x => x + i + base);
  base = 1000;
  return out;
}
console.log(makeAdders().map(f => f(1)).join(" "));

function outer() {
  let a = 1;
  function mid() {
    let b = 2;
    return function inner() { a += 10; b += 20; return a + b; };
  }
  const f = mid();
  f();
  return f() + a;
}
console.log(outer());

const fact = function self(n) { return n <= 1 ? 1 : n * self(n - 1); };
console.log(fact(10));

function paramCaptured(p) {
  const get = () => p;
  p = p * 2;
  return get();
}
console.log(paramCaptured(21));

class Thing {
  constructor() { this.v = 5; }
  arrows() { return [1, 2].map(x => this.v * x); }
}
console.log(new Thing().arrows().join(","));
