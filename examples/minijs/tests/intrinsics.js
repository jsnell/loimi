// Natives that CALL runs inline (charCodeAt, charAt, push, Math.floor,
// Math.sqrt): edge cases, and sites whose callee changes.
function codes(s) {
  const r = [];
  for (let i = -1; i <= s.length; i++) r.push(s.charCodeAt(i));
  return r;
}
console.log(codes("abc").join(" "));
console.log(codes("").join(" "));
let rope = "";
for (let i = 0; i < 40; i++) rope += "xy";
console.log(rope.charCodeAt(3), rope.charAt(5), rope.charCodeAt(1.5), rope.charAt(-1) === "");
console.log("hello".charAt(1), "hello".charAt(10) === "", "hello".charAt("2"));

function floors(xs) {
  const r = [];
  for (const x of xs) r.push(1 / Math.floor(x) < 0 ? "neg:" + Math.floor(x) : Math.floor(x));
  return r;
}
console.log(floors([1, -1, 0.5, -0.5, -0, 2.5, -2.5, 3e10, -3e10, NaN, Infinity, "7.9"]).join(" "));
const roots = [];
for (const x of [0, -0, 4, 2, 2.25, -1, 1e300, "9", Infinity]) roots.push(Math.sqrt(x));
console.log(roots.join(" "), 1 / Math.sqrt(-0));

const a = [];
for (let i = 0; i < 20; i++) console.log(a.push(i * i), a.length);
console.log(a.join(","));
const b = [1];
console.log(b.push(), b.push(2, 3), b.join(","));

// The same sites with other callees.
function call1(o, f, x) { return o[f](x); }
const fake = { charCodeAt(i) { return "fake" + i; }, push(x) { return "pushed " + x; } };
for (let i = 0; i < 3; i++) {
  console.log(call1("abc", "charCodeAt", i), call1(fake, "charCodeAt", i), call1([], "push", i), call1(fake, "push", i));
}
const saved = String.prototype.charCodeAt;
function first(s) { return s.charCodeAt(0); }
console.log(first("A"), first("B"));
String.prototype.charCodeAt = function (i) { return "patched " + this + i; };
console.log(first("A"), first("B"));
String.prototype.charCodeAt = saved;
console.log(first("A"), first("B"));
const m = Math.floor;
Math.floor = function (x) { return "floor(" + x + ")"; };
console.log(Math.floor(2.5));
Math.floor = m;
console.log(Math.floor(2.5));
