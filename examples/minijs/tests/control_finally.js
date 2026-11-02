// try/finally with break, continue and return crossing it; nested
// handlers; labels; switch fallthrough.
const log = [];
function f1() {
  for (let i = 0; i < 5; i++) {
    try {
      if (i === 1) continue;
      if (i === 3) break;
      log.push("body" + i);
    } finally {
      log.push("fin" + i);
    }
  }
  return log.join(" ");
}
console.log(f1());

function f2() {
  try {
    try {
      return "inner";
    } finally {
      console.log("finally 1");
    }
  } finally {
    console.log("finally 2");
  }
}
console.log(f2());

function f3() {
  try {
    throw new Error("boom");
  } catch (e) {
    return "caught " + e.message;
  } finally {
    console.log("f3 finally");
  }
}
console.log(f3());

function f4() {
  let r = "";
  outer: for (let i = 0; i < 3; i++) {
    for (let j = 0; j < 3; j++) {
      try {
        if (j === 1) continue outer;
        r += i + "" + j + " ";
      } finally {
        r += "f ";
      }
    }
  }
  return r;
}
console.log(f4());

function f5() {
  try {
    try {
      throw 1;
    } finally {
      console.log("cleanup");
    }
  } catch (e) {
    return "outer caught " + e;
  }
}
console.log(f5());

function f6(x) {
  switch (x) {
    case 1: console.log("one");
    case 2: console.log("two"); break;
    case "3": console.log("string three"); break;
    default: console.log("default");
    case 4: console.log("four");
  }
}
f6(1); f6(2); f6(3); f6("3"); f6(4); f6(9);

function f7() {
  let n = 0;
  block: {
    n++;
    if (n > 0) break block;
    n = 100;
  }
  return n;
}
console.log(f7());

function thrower(depth) {
  if (depth === 0) throw new TypeError("deep");
  return thrower(depth - 1);
}
try { thrower(50); } catch (e) { console.log(e.message); }

let count = 0;
do { count++; } while (count < 5);
console.log(count);
