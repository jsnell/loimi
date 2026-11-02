var s = "Hello, World";
console.log(s.length, s[0], s[7], s[11], s[12], s[-1], "".length, ""[0]);
var rev = "";
for (var i = s.length - 1; i >= 0; i--) { rev += s[i]; }
console.log(rev);
var t = s;
t += "!";
console.log(s, t, s.length, t.length);
console.log("abc" + "def", "a" + "" + "b", ("x" + "y").length);
var big = "";
for (var j = 0; j < 1000; j++) { big += String.fromCharCode(97 + j % 26); }
console.log(big.length, big.slice(0, 30), big[999]);
console.log("abc"[1] + "abc"[2], "abc".length + 1);
var counts = {};
var text = "the quick brown fox jumps over the lazy dog";
for (var k = 0; k < text.length; k++) {
  var c = text[k];
  if (c === " ") { continue; }
  counts[c] = (counts[c] || 0) + 1;
}
var keys = Object.keys(counts);
console.log(keys.length, counts.o, counts.e, counts.t, counts.z);
console.log("a\tb".length, "line1\nline2");
console.log("" + "" === "", "ab" < "abc", "abc" === "ab" + "c");
