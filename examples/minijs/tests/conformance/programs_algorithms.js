function sieve(n) {
  var flags = new Array(n + 1).fill(true);
  flags[0] = false; flags[1] = false;
  for (var i = 2; i * i <= n; i++) { if (flags[i]) { for (var j = i * i; j <= n; j += i) { flags[j] = false; } } }
  var primes = [];
  for (var k = 0; k <= n; k++) { if (flags[k]) { primes.push(k); } }
  return primes;
}
var ps = sieve(1000);
console.log(ps.length, ps.slice(0, 10).join(","), ps[ps.length - 1]);
function gcd(a, b) { while (b !== 0) { var t = b; b = a % b; a = t; } return a; }
console.log(gcd(1071, 462), gcd(17, 5), gcd(0, 9), gcd(2 ** 40, 6 ** 10));
function bsearch(arr, v) {
  var lo = 0, hi = arr.length - 1;
  while (lo <= hi) { var mid = (lo + hi) >> 1; if (arr[mid] === v) { return mid; } if (arr[mid] < v) { lo = mid + 1; } else { hi = mid - 1; } }
  return -1;
}
console.log(bsearch(ps, 997), bsearch(ps, 2), bsearch(ps, 4), bsearch([], 1));
function qsort(a) {
  if (a.length <= 1) { return a; }
  var p = a[0];
  return qsort(a.slice(1).filter(x => x < p)).concat([p], qsort(a.slice(1).filter(x => x >= p)));
}
var seed = 42;
function rnd() { seed = (seed * 16807) % 2147483647; return seed; }
var data = [];
for (var i = 0; i < 200; i++) { data.push(rnd() % 1000); }
var sorted = qsort(data);
var ok = true;
for (var j = 1; j < sorted.length; j++) { if (sorted[j - 1] > sorted[j]) { ok = false; } }
console.log(ok, sorted.length, sorted[0], sorted[199], data.slice(0, 5).join(","));
function hanoi(n, from, to, via, moves) { if (n === 0) { return moves; } hanoi(n - 1, from, via, to, moves); moves.push(from + to); return hanoi(n - 1, via, to, from, moves); }
var mv = hanoi(10, "A", "C", "B", []);
console.log(mv.length, mv.slice(0, 7).join(" "));
function queens(n) {
  var count = 0, cols = new Array(n).fill(0);
  function ok(r, c) { for (var i = 0; i < r; i++) { if (cols[i] === c || Math.abs(cols[i] - c) === r - i) { return false; } } return true; }
  function place(r) { if (r === n) { count++; return; } for (var c = 0; c < n; c++) { if (ok(r, c)) { cols[r] = c; place(r + 1); } } }
  place(0);
  return count;
}
console.log(queens(6), queens(8));
function wordFreq(text) {
  var freq = {};
  for (const w of text.toLowerCase().split(" ")) { if (w.length > 0) { freq[w] = (freq[w] || 0) + 1; } }
  return Object.keys(freq).sort((a, b) => freq[b] - freq[a] || (a < b ? -1 : a > b ? 1 : 0)).map(w => w + "=" + freq[w]).join(" ");
}
console.log(wordFreq("The cat and the hat and THE bat sat"));
function toBinary(n) { var s = ""; do { s = (n & 1) + s; n = n >>> 1; } while (n > 0); return s; }
console.log(toBinary(10), toBinary(255), toBinary(0), toBinary(-1).length);
function collatz(n) { var steps = 0; while (n !== 1) { n = n % 2 === 0 ? n / 2 : 3 * n + 1; steps++; } return steps; }
console.log(collatz(27), collatz(97), collatz(871));
