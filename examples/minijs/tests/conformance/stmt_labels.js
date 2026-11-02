var pairs = [];
outer: for (var i = 0; i < 5; i++) {
  for (var j = 0; j < 5; j++) {
    if (j === 3) { continue outer; }
    if (i === 3) { break outer; }
    pairs.push("" + i + j);
  }
}
console.log(pairs.join(" "));
var found = null;
search: for (const row of [[1, 2], [3, 4], [5, 6]]) {
  for (const v of row) { if (v === 4) { found = v; break search; } }
}
console.log("found", found);
var log = [];
loop1: while (log.length < 20) {
  var k = 0;
  loop2: do {
    k++;
    if (k === 2) { continue loop2; }
    if (k === 4) { continue loop1; }
    log.push(k);
  } while (k < 10);
}
console.log(log.length, log.slice(0, 6).join(","));
var visited = [];
a: for (let x = 0; x < 3; x++) {
  b: for (let y = 0; y < 3; y++) {
    c: for (let z = 0; z < 3; z++) {
      if (z === 1) { continue b; }
      if (y === 2) { continue a; }
      if (x === 2) { break a; }
      visited.push(x + "" + y + z);
    }
  }
}
console.log(visited.join(" "));
var r = [];
blk: {
  r.push("start");
  if (r.length > 0) { break blk; }
  r.push("never");
}
r.push("after");
console.log(r.join(","));
var obj = { a: 1, b: 2, c: 3 };
var ks = [];
keys: for (const key in obj) {
  for (var t = 0; t < 2; t++) { if (key === "b") { continue keys; } }
  ks.push(key);
}
console.log(ks.join(""));
var sw = [];
lab: for (var q = 0; q < 4; q++) {
  switch (q) {
    case 1: continue lab;
    case 2: break;
    case 3: break lab;
  }
  sw.push(q);
}
console.log(sw.join(","), q);
