// Closures and higher-order functions: map, filter, reduce, forEach, some,
// every, plus hand-written combinators (compose, curry, memoizing counters).
// Stresses closure creation, captured-variable access and callback calls.
const SIZE = 2000;
const ROUNDS = 1200;

function makeRng(seed) {
  let state = seed;
  return function () {
    state = (state * 16807) % 2147483647; // Park-Miller, exact in doubles
    return state;
  };
}

function compose(f, g) {
  return x => f(g(x));
}

function curry2(f) {
  return a => b => f(a, b);
}

function makeCounter() {
  let count = 0;
  return {
    inc: () => { count++; return count; },
    get: () => count
  };
}

function pipeline(fns) {
  return x => fns.reduce((acc, f) => f(acc), x);
}

function run(round, data) {
  const add = curry2((a, b) => a + b);
  const mul = curry2((a, b) => a * b);
  const addR = add(round);
  const f = compose(mul(3), addR);
  const g = pipeline([x => x + 1, x => x % 1000, mul(2), add(-7)]);

  const mapped = data.map(f);
  const evens = mapped.filter(x => x % 2 === 0);
  const sum = evens.reduce((acc, x) => acc + x, 0);

  const counter = makeCounter();
  data.forEach((x, i) => {
    if ((x + i) % 3 === 0) {
      counter.inc();
    }
  });

  const piped = data.map(g);
  const maxv = piped.reduce((m, x) => (x > m ? x : m), -Infinity);
  const anyBig = piped.some(x => x > 1990);
  const allPos = data.every(x => x >= 0);

  // closures created in a loop with per-iteration bindings
  const thunks = [];
  for (let i = 0; i < 50; i++) {
    thunks.push(() => i * round);
  }
  let tsum = 0;
  for (const t of thunks) {
    tsum += t();
  }

  // indices passed to callbacks
  const idxSum = data.filter((x, i) => i % 7 === 0).map((x, i, arr) => x + i + arr.length)
    .reduce((a, b) => a + b, 0);

  return (sum % 1000003) + counter.get() + maxv + (anyBig ? 1 : 0) + (allPos ? 2 : 0) +
    tsum + (idxSum % 99991);
}

function main() {
  const rng = makeRng(42);
  const data = [];
  for (let i = 0; i < SIZE; i++) {
    data.push(rng() % 10000);
  }
  let checksum = 0;
  for (let r = 0; r < ROUNDS; r++) {
    checksum = (checksum * 31 + run(r, data)) % 1000000007;
  }
  console.log("checksum " + checksum);
}

main();
