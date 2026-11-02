// "Are We Fast Yet" micro-benchmarks: Sieve, Permute, Queens, Towers, List.
// Stresses array loops, recursion, boolean arrays, small object chains.
const ITERATIONS = 120;

// --- Sieve
function sieve() {
  const flags = new Array(5000).fill(true);
  let primeCount = 0;
  for (let i = 2; i <= 5000; i++) {
    if (flags[i - 1]) {
      primeCount++;
      let k = i + i;
      while (k <= 5000) {
        flags[k - 1] = false;
        k += i;
      }
    }
  }
  return primeCount;
}

// --- Permute
class Permute {
  constructor() {
    this.count = 0;
    this.v = null;
  }
  run() {
    this.count = 0;
    this.v = new Array(6).fill(0);
    this.permute(6);
    return this.count;
  }
  permute(n) {
    this.count++;
    if (n !== 0) {
      const n1 = n - 1;
      this.permute(n1);
      for (let i = n1; i >= 0; i--) {
        this.swap(n1, i);
        this.permute(n1);
        this.swap(n1, i);
      }
    }
  }
  swap(i, j) {
    const tmp = this.v[i];
    this.v[i] = this.v[j];
    this.v[j] = tmp;
  }
}

// --- Queens
class Queens {
  constructor() {
    this.freeRows = null;
    this.freeMaxs = null;
    this.freeMins = null;
    this.queenRows = null;
  }
  run() {
    let result = true;
    for (let i = 0; i < 10; i++) {
      result = result && this.queens();
    }
    return result;
  }
  queens() {
    this.freeRows = new Array(8).fill(true);
    this.freeMaxs = new Array(16).fill(true);
    this.freeMins = new Array(16).fill(true);
    this.queenRows = new Array(8).fill(-1);
    return this.placeQueen(0);
  }
  placeQueen(c) {
    for (let r = 0; r < 8; r++) {
      if (this.getRowColumn(r, c)) {
        this.queenRows[r] = c;
        this.setRowColumn(r, c, false);
        if (c === 7) {
          return true;
        }
        if (this.placeQueen(c + 1)) {
          return true;
        }
        this.setRowColumn(r, c, true);
      }
    }
    return false;
  }
  getRowColumn(r, c) {
    return this.freeRows[r] && this.freeMaxs[c + r] && this.freeMins[c - r + 7];
  }
  setRowColumn(r, c, v) {
    this.freeRows[r] = v;
    this.freeMaxs[c + r] = v;
    this.freeMins[c - r + 7] = v;
  }
}

// --- Towers
class TowersDisk {
  constructor(size) {
    this.size = size;
    this.next = null;
  }
}

class Towers {
  constructor() {
    this.piles = null;
    this.movesDone = 0;
  }
  run() {
    this.piles = new Array(3).fill(null);
    this.buildTowerAt(0, 13);
    this.movesDone = 0;
    this.moveDisks(13, 0, 1);
    return this.movesDone;
  }
  pushDisk(disk, pile) {
    const top = this.piles[pile];
    if (top !== null && disk.size >= top.size) {
      throw new Error("Cannot put a big disk on a smaller one");
    }
    disk.next = top;
    this.piles[pile] = disk;
  }
  popDiskFrom(pile) {
    const top = this.piles[pile];
    if (top === null) {
      throw new Error("Attempting to remove a disk from an empty pile");
    }
    this.piles[pile] = top.next;
    top.next = null;
    return top;
  }
  moveTopDisk(fromPile, toPile) {
    this.pushDisk(this.popDiskFrom(fromPile), toPile);
    this.movesDone++;
  }
  buildTowerAt(pile, disks) {
    for (let i = disks; i >= 0; i--) {
      this.pushDisk(new TowersDisk(i), pile);
    }
  }
  moveDisks(disks, fromPile, toPile) {
    if (disks === 1) {
      this.moveTopDisk(fromPile, toPile);
    } else {
      const otherPile = 3 - fromPile - toPile;
      this.moveDisks(disks - 1, fromPile, otherPile);
      this.moveTopDisk(fromPile, toPile);
      this.moveDisks(disks - 1, otherPile, toPile);
    }
  }
}

// --- List
class Element {
  constructor(v) {
    this.val = v;
    this.next = null;
  }
  length() {
    if (this.next === null) {
      return 1;
    }
    return 1 + this.next.length();
  }
}

class List {
  run() {
    const result = this.tail(this.makeList(15), this.makeList(10), this.makeList(6));
    return result.length();
  }
  makeList(length) {
    if (length === 0) {
      return null;
    }
    const e = new Element(length);
    e.next = this.makeList(length - 1);
    return e;
  }
  isShorterThan(x, y) {
    let xTail = x;
    let yTail = y;
    while (yTail !== null) {
      if (xTail === null) {
        return true;
      }
      xTail = xTail.next;
      yTail = yTail.next;
    }
    return false;
  }
  tail(x, y, z) {
    if (this.isShorterThan(y, x)) {
      return this.tail(this.tail(x.next, y, z), this.tail(y.next, z, x), this.tail(z.next, x, y));
    }
    return z;
  }
}

function main() {
  const results = { sieve: 0, permute: 0, queens: 0, towers: 0, list: 0 };
  let ok = true;
  for (let i = 0; i < ITERATIONS; i++) {
    for (let j = 0; j < 20; j++) {
      results.sieve = sieve();
      ok = ok && results.sieve === 669;
    }
    results.permute = new Permute().run();
    ok = ok && results.permute === 8660;
    results.queens = new Queens().run() ? 1 : 0;
    ok = ok && results.queens === 1;
    results.towers = new Towers().run();
    ok = ok && results.towers === 8191;
    for (let j = 0; j < 4; j++) {
      results.list = new List().run();
      ok = ok && results.list === 10;
    }
  }
  for (const k in results) {
    console.log(k + " " + results[k]);
  }
  console.log("ok " + ok);
}

main();
