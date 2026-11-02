var seen = [];
[1, 2, 3, 4].forEach(function (x) {
  console.log("visit", x);
  if (x === 3) { throw new Error("stop in callback"); }
});
console.log("never");
