var o = { a: 1, b: 2, c: undefined };
console.log("a" in o, "c" in o, "d" in o, o.c === undefined, o.d === undefined);
console.log(delete o.a, "a" in o, o.a, Object.keys(o).join(","));
console.log(delete o["b"], Object.keys(o).join(","));
console.log(delete o.nothing, Object.keys(o).join(","));
var k = "c";
console.log(delete o[k], Object.keys(o).length);
o.a = "again";
console.log(o.a, "a" in o);
var proto = { inherited: 1 };
var child = Object.create(proto);
child.own = 2;
console.log("inherited" in child, "own" in child, child.hasOwnProperty("inherited"), child.hasOwnProperty("own"));
console.log(delete child.inherited, child.inherited, "inherited" in child);
child.inherited = "shadow";
console.log(child.inherited, proto.inherited);
delete child.inherited;
console.log(child.inherited);
var arr = [1, 2, 3];
console.log(0 in arr, 2 in arr, 3 in arr, "length" in arr, "push" in arr, arr.hasOwnProperty("length"), arr.hasOwnProperty(1), arr.hasOwnProperty("push"));
console.log("toString" in {}, "hasOwnProperty" in {}, ({}).hasOwnProperty("toString"));
var counter = {};
var words = ["a", "b", "a", "c", "a", "b"];
for (var i = 0; i < words.length; i++) {
  var w = words[i];
  if (w in counter) { counter[w]++; } else { counter[w] = 1; }
}
console.log(counter.a, counter.b, counter.c);
for (var j = 0; j < 100; j++) { counter["t" + j] = j; }
for (var j2 = 0; j2 < 100; j2 += 2) { delete counter["t" + j2]; }
console.log(Object.keys(counter).length, counter.t51, counter.t50, "t98" in counter, "t99" in counter);
