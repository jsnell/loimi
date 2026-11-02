var ns = [0, 1, 7, 8, 10, 15, 16, 31, 32, 35, 36, 100, 255, 256, 1000, 65535, 2147483647, 2147483648, 4294967295, 4294967296, 9007199254740991, 1099511627776];
for (var i = 0; i < ns.length; i++) {
  var n = ns[i];
  console.log(n.toString(), n.toString(2), n.toString(8), n.toString(16), n.toString(36), (-n).toString(16));
}
console.log((255).toString(10), (35).toString(36), (123456789).toString(7), (123456789).toString(3));
var x = 3735928559;
console.log(x.toString(16), (x | 0).toString(16), (x >>> 0).toString(2));
for (var r = 2; r <= 36; r++) {
  console.log("r" + r, (1295).toString(r), (-1295).toString(r));
}
