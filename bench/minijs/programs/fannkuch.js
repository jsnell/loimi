// fannkuch-redux, after the Computer Language Benchmarks Game.
// Stresses small integer arrays, tight loops and array element access.
const N = 9;
const REPEAT = 3;

function fannkuch(n) {
  const perm = new Array(n).fill(0);
  const perm1 = new Array(n).fill(0);
  const count = new Array(n).fill(0);
  for (let i = 0; i < n; i++) {
    perm1[i] = i;
  }
  let maxFlips = 0;
  let checksum = 0;
  let permCount = 0;
  let r = n;
  while (true) {
    while (r !== 1) {
      count[r - 1] = r;
      r--;
    }
    for (let i = 0; i < n; i++) {
      perm[i] = perm1[i];
    }
    let flips = 0;
    let k = perm[0];
    while (k !== 0) {
      let i = 0;
      let j = k;
      while (i < j) {
        const t = perm[i];
        perm[i] = perm[j];
        perm[j] = t;
        i++;
        j--;
      }
      flips++;
      k = perm[0];
    }
    if (flips > maxFlips) {
      maxFlips = flips;
    }
    if (permCount % 2 === 0) {
      checksum += flips;
    } else {
      checksum -= flips;
    }
    // next permutation
    while (true) {
      if (r === n) {
        return [checksum, maxFlips];
      }
      const perm0 = perm1[0];
      let i = 0;
      while (i < r) {
        const j = i + 1;
        perm1[i] = perm1[j];
        i = j;
      }
      perm1[r] = perm0;
      count[r] = count[r] - 1;
      if (count[r] > 0) {
        break;
      }
      r++;
    }
    permCount++;
  }
}

for (let rep = 0; rep < REPEAT; rep++) {
  const res = fannkuch(N);
  console.log(String(res[0]));
  console.log("Pfannkuchen(" + N + ") = " + res[1]);
}
