var parts = "a,b,,c,".split(",");
console.log(parts.length, parts.join("|"));
console.log("abc".split("").join("-"), "".split("").length, "".split(",").length, "abc".split("x").length);
console.log("one two  three".split(" ").length, "one two  three".split(" ").join("_"));
console.log("a--b--c".split("--").join("+"), "--a--".split("--").length, "aaa".split("a").length);
var csv = "name=ann;age=31;city=oslo";
var fields = csv.split(";");
var rec = {};
for (var i = 0; i < fields.length; i++) {
  var kv = fields[i].split("=");
  rec[kv[0]] = kv[1];
}
console.log(rec.name, rec.age, rec.city, Number(rec.age) + 1);
var words = "the rain in spain".split(" ");
for (var j = 0; j < words.length; j++) { words[j] = words[j][0].toUpperCase() + words[j].slice(1); }
console.log(words.join(" "));
console.log([1, 2, 3].join(), [1, 2, 3].join(""), [1, 2, 3].join(", "), [].join("x") + "|", [5].join("-"));
console.log(["a", null, "b", undefined, "c"].join("."), [[1, 2], [3, 4]].join(";"), [true, false].join());
console.log("x".split("x").length, "xyx".split("y").join("+"));
