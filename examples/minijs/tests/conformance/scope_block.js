let a = 1;
{ let a = 2; { let a = 3; console.log("inner", a); } console.log("middle", a); }
console.log("outer", a);
const c = "const";
{ const c = "shadow"; console.log(c); }
console.log(c);
var v = "var-outer";
function f() { var v = "var-inner"; return v; }
console.log(f(), v);
if (true) { var hoistedVar = "var escapes block"; let blockLet = "hidden"; }
console.log(hoistedVar);
for (let i = 0; i < 1; i++) { let i = "body-shadow"; console.log(i); }
const obj = { n: 1 };
obj.n = 2;
console.log(obj.n);
const arr = [1];
arr.push(2);
console.log(arr.join(","));
let x = 10;
function readX() { return x; }
{ let x = 20; console.log(readX(), x); }
function shadowParam(p) { { let p = "inner-p"; } return p; }
console.log(shadowParam("param"));
function paramIsLocal(p) { p = "changed"; return p; }
var orig = "orig";
console.log(paramIsLocal(orig), orig);
function mutateObj(o) { o.k = "mutated"; o = { k: "rebound" }; return o.k; }
var target = { k: "start" };
console.log(mutateObj(target), target.k);
let counter = 0;
const incr = () => { counter++; };
incr(); incr();
console.log(counter);
function nestedFns() {
  var lvl = 1;
  function g() { var lvl = 2; function h() { return lvl; } return h(); }
  return lvl + g();
}
console.log(nestedFns());
var sw = "x";
switch (sw) { case "x": { let sw2 = "in case"; console.log(sw2); } }
let late;
console.log(late);
late = 5;
console.log(late);
