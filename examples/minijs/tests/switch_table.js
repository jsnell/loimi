// switch statements on integer cases (compiled to jump tables when dense).
function kind(c) {
  switch (c) {
    case 123: return "{";
    case 125: return "}";
    case 91: return "[";
    case 93: return "]";
    case 58: return ":";
    case 44: return ",";
    case 34: return "quote";
  }
  return "other";
}
const out = [];
for (const c of [123, 125, 91, 93, 58, 44, 34, 33, 35, 126, 0, -1, 123.0, 91.5, "123", null, undefined, NaN, -0, 2147483647, -2147483648]) {
  out.push(kind(c));
}
console.log(out.join(" "));

function fall(x) {
  let s = "";
  switch (x) {
    case 3: s += "3";
    case 1: s += "1"; break;
    default: s += "d";
    case 2: s += "2";
    case 0: s += "0";
  }
  return s;
}
console.log([0, 1, 2, 3, 4, -1, 1.5, "2"].map(fall).join(" "));

function dup(x) {
  switch (x) {
    case 0: return "zero";
    case 1: return "one";
    case 0: return "zero again";
    case -2: return "minus two";
  }
  return "none";
}
console.log([0, 1, -2, -1, 2].map(dup).join(" "));

function loopy(n) {
  let total = 0;
  for (let i = 0; i < n; i++) {
    switch (i % 5) {
      case 0: total += 1; continue;
      case 1: total += 10; break;
      case 2: { let t = i * 2; total += t; break; }
      case 3: if (i > 10) break; total += 1000;
    }
    total += 7;
  }
  return total;
}
console.log(loopy(30));
