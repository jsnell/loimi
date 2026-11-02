var nums = [5, 3, 10, 1, 22, -4, 3, 0.5];
var r = nums.sort((a, b) => a - b);
console.log(nums.join(","), r === nums);
console.log(nums.sort((a, b) => b - a).join(","));
var strs = ["pear", "Apple", "fig", "banana", "apple", "Fig"];
strs.sort(function (a, b) { return a < b ? -1 : a > b ? 1 : 0; });
console.log(strs.join(","));
strs.sort((a, b) => a.length - b.length);
console.log(strs.join(","));
var recs = [];
var names = ["d", "a", "c", "b", "e", "f", "g", "h", "i", "j", "k", "l"];
for (var i = 0; i < names.length; i++) { recs.push({ name: names[i], grp: i % 3 }); }
recs.sort((x, y) => x.grp - y.grp);
console.log(recs.map(x => x.grp + x.name).join(" "));
var big = [];
var seed = 12345;
for (var j = 0; j < 500; j++) { seed = (seed * 16807) % 2147483647; big.push({ key: seed % 10, ord: j }); }
big.sort((x, y) => x.key - y.key);
var stable = true;
for (var k = 1; k < big.length; k++) {
  if (big[k - 1].key > big[k].key) { stable = false; }
  if (big[k - 1].key === big[k].key && big[k - 1].ord > big[k].ord) { stable = false; }
}
console.log("sorted+stable", stable, big[0].key, big[499].key);
console.log([].sort((a, b) => a - b).length, [1].sort((a, b) => a - b).join(","));
var calls = 0;
[3, 1, 2].sort(function (a, b) { calls++; return a - b; });
console.log(calls > 0);
var multi = [[2, "b"], [1, "z"], [2, "a"], [1, "a"]];
multi.sort((p, q) => p[0] !== q[0] ? p[0] - q[0] : (p[1] < q[1] ? -1 : p[1] > q[1] ? 1 : 0));
console.log(multi.map(p => p[0] + p[1]).join(" "));
var bools = [3, 1, 2];
bools.sort((a, b) => a < b ? -1 : 1);
console.log(bools.join(","));
var dupes = [2, 1, 2, 1, 2];
dupes.sort((a, b) => a - b);
console.log(dupes.join(""));
var neg = [-1, -10, 5, -100, 50];
neg.sort((a, b) => a - b);
console.log(neg.join(","));
