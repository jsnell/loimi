// Arithmetic fast paths: Int overflow at Int sites, `/` sites that see
// Ints then floats (and the reverse), comparisons with number constants
// beyond 8 bits, string concatenation through ADD, and `s[i]` out of range.
function show(label, v) {
  console.log(label, String(v), typeof v, v === 0 && 1 / v < 0);
}

// Int sites that overflow (MUL's -0 too).
function mul(a, b) { return a * b; }
function add(a, b) { return a + b; }
function sub(a, b) { return a - b; }
show("mul", mul(3, 4));
show("mul ovf", mul(65536, 65536));
show("mul -0", mul(0, -5));
show("mul -0b", mul(-3, 0));
show("mul ovf neg", mul(-2147483648, 2));
show("add ovf", add(2147483647, 1));
show("add", add(1, 2));
show("sub ovf", sub(-2147483648, 1));
let seed = 12345;
for (let i = 0; i < 5; i++) seed = (seed * 16807) % 2147483647;
show("seed", seed);

// Division: Int sites, float sites, mixed, zero divisors.
function div(a, b) { return a / b; }
const pairs = [[6, 3], [7, 2], [0, 5], [0, -5], [1, 0], [-1, 0], [0, 0],
               [-2147483648, -1], [7.5, 2.5], [1, 3], [9, 1.5], [2.5, 5], [-6, 3], [6, -3]];
for (const p of pairs) show("div " + p[0] + "/" + p[1], div(p[0], p[1]));
function fdiv(a, b) { return a / b; }
show("fdiv", fdiv(1.5, 0.5));
show("fdiv int", fdiv(10, 4));
show("fdiv int exact", fdiv(10, 5));
show("fdiv -0", fdiv(0, -1));
show("fdiv str", fdiv("9", 3));
let acc = 0;
for (let i = 0; i < 100; i++) acc += ((i + 1) * (i + 2)) / 2 + 1 / (i + 1);
show("acc", acc);

// Comparisons with constants that do not fit in 8 bits.
const BIG = 5000;
function cmp(x) {
  let r = "";
  r += x < 5000 ? "a" : "b";
  r += x <= BIG ? "c" : "d";
  r += x > 1000.5 ? "e" : "f";
  r += x >= -300 ? "g" : "h";
  r += 5000 < x ? "i" : "j";
  r += 0.25 >= x ? "k" : "l";
  if (x < 70000) r += "m";
  if (!(x >= 4999.5)) r += "n";
  return r;
}
for (const x of [0, 4999, 5000, 5001, 1000.5, 1000.75, -300, -301, 0.25, NaN, "6000", "abc", null, undefined, true, 70000])
  console.log("cmp", x, cmp(x));
let n = 0;
for (let i = 0; i <= 5000; i++) n += 1;
for (let d = 0.5; d < 300.25; d += 0.5) n += 1;
console.log("loops", n);

// String concatenation at Int ADD sites.
function cat(a, b) { return a + b; }
console.log(cat(1, 2), cat("x", 1), cat(1, "y"), cat("a", "b"), cat([1, 2], 3), cat({}, "!"), cat(null, "n"), cat("u", undefined));
let s = "";
for (let i = 0; i < 40; i++) s = s + i + ",";
console.log(s.length, s.slice(0, 20), s.charCodeAt(5));
let big = 2147483600;
for (let i = 0; i < 100; i++) big = big + 1;
show("big", big);

// Indexing out of range.
const u = "ab";
console.log(u[0], u[1], u[2], u[-1], u.charAt(1), u.charAt(9) === "");

// `===` and `!==` against constants, as values.
const KIND = 3;
function eqs(x) {
  const a = x === 3, b = x !== KIND, c = x === "s", d = x === null, e = x !== 2.5, f = x === undefined;
  return [a, b, c, d, e, f].join(",");
}
for (const x of [3, 3.0, 2.5, "s", "3", null, undefined, NaN, 0, -0, true])
  console.log("eq", String(x), eqs(x));
