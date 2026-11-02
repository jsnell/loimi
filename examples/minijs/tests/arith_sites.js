// Arithmetic sites that see several types in turn (they quicken to
// float-first or mixed members and must stay correct for everything).
function add(a, b) { return a + b; }
function sub(a, b) { return a - b; }
function mul(a, b) { return a * b; }
function div(a, b) { return a / b; }
function mod(a, b) { return a % b; }
const vals = [1, -1, 0, -0, 2147483647, -2147483648, 0.5, -2.5, 1e300, NaN, Infinity, -Infinity, "3", "x", true, null, undefined];
function show(x) { return (x === 0 && 1 / x < 0) ? "-0" : String(x); }
const fns = [add, sub, mul, div, mod];
const names = ["add", "sub", "mul", "div", "mod"];
for (let fi = 0; fi < fns.length; fi++) {
  const f = fns[fi];
  const name = names[fi];
  const out = [];
  // Floats first, then everything: the sites become _F members early.
  for (const a of [0.25, 1.5, 3]) for (const b of [0.5, -1.25]) out.push(show(f(a, b)));
  for (const a of vals) for (const b of vals) out.push(show(f(a, b)));
  console.log(name, out.join(" "));
}
// Mixed members: Int op Num first, then other kinds.
function mixed(n) {
  let s = 0;
  for (let i = 0; i < n; i++) s = i + s * 0.5;
  return s;
}
console.log(mixed(10), mixed(3));
function mixed2(a, b) { return a * b; }
console.log(mixed2(3, 0.5), mixed2(0.5, 3), show(mixed2(-3, 0)), show(mixed2(3, -0)), mixed2("2", 4), mixed2(65536, 65536));
// Park-Miller and modulo of large integral doubles.
let seed = 42;
for (let i = 0; i < 5; i++) { seed = (seed * 16807) % 2147483647; console.log(seed); }
console.log(-7 % 3, show(-6 % 3), 7.5 % 2, -7.5 % 2, 2 ** 53 % 7, -(2 ** 52) % 10, 5 % -3, show(-0 % 5));
