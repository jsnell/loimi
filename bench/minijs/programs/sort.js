// Sorting: Array.prototype.sort with consistent comparators (numbers,
// strings, multi-key records), checked against hand-written quicksort,
// merge sort and heap sort. Stresses comparator calls and array writes.
const SIZE = 20000;
const ROUNDS = 6;

function makeRng(seed) {
  let state = seed;
  return function () {
    state = (state * 16807) % 2147483647;
    return state;
  };
}

function quicksort(a, lo, hi) {
  while (lo < hi) {
    const pivot = a[(lo + hi) >> 1];
    let i = lo;
    let j = hi;
    while (i <= j) {
      while (a[i] < pivot) {
        i++;
      }
      while (a[j] > pivot) {
        j--;
      }
      if (i <= j) {
        const t = a[i];
        a[i] = a[j];
        a[j] = t;
        i++;
        j--;
      }
    }
    if (j - lo < hi - i) {
      quicksort(a, lo, j);
      lo = i;
    } else {
      quicksort(a, i, hi);
      hi = j;
    }
  }
}

function mergeSort(a, cmp) {
  if (a.length <= 1) {
    return a;
  }
  const mid = a.length >> 1;
  const left = mergeSort(a.slice(0, mid), cmp);
  const right = mergeSort(a.slice(mid), cmp);
  const out = [];
  let i = 0;
  let j = 0;
  while (i < left.length && j < right.length) {
    if (cmp(right[j], left[i]) < 0) {
      out.push(right[j]);
      j++;
    } else {
      out.push(left[i]);
      i++;
    }
  }
  while (i < left.length) {
    out.push(left[i]);
    i++;
  }
  while (j < right.length) {
    out.push(right[j]);
    j++;
  }
  return out;
}

function heapSort(a) {
  const n = a.length;
  function sift(start, end) {
    let root = start;
    while (root * 2 + 1 <= end) {
      let child = root * 2 + 1;
      if (child + 1 <= end && a[child] < a[child + 1]) {
        child++;
      }
      if (a[root] < a[child]) {
        const t = a[root];
        a[root] = a[child];
        a[child] = t;
        root = child;
      } else {
        return;
      }
    }
  }
  for (let start = (n - 2) >> 1; start >= 0; start--) {
    sift(start, n - 1);
  }
  for (let end = n - 1; end > 0; end--) {
    const t = a[0];
    a[0] = a[end];
    a[end] = t;
    sift(0, end - 1);
  }
}

function sameArray(a, b) {
  if (a.length !== b.length) {
    return false;
  }
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) {
      return false;
    }
  }
  return true;
}

function wordOf(n) {
  let s = "";
  do {
    s += String.fromCharCode(97 + (n % 26));
    n = Math.floor(n / 26);
  } while (n > 0);
  return s;
}

function main() {
  const rng = makeRng(12345);
  let checksum = 0;
  let ok = true;
  for (let r = 0; r < ROUNDS; r++) {
    const nums = [];
    for (let i = 0; i < SIZE; i++) {
      nums.push(rng() % 100000);
    }
    const a1 = nums.slice().sort((x, y) => x - y);
    const a2 = nums.slice();
    quicksort(a2, 0, a2.length - 1);
    const a3 = mergeSort(nums, (x, y) => x - y);
    const a4 = nums.slice();
    heapSort(a4);
    ok = ok && sameArray(a1, a2) && sameArray(a1, a3) && sameArray(a1, a4);

    const desc = nums.slice().sort((x, y) => y - x);
    ok = ok && desc[0] === a1[a1.length - 1];

    // stable multi-key sort of records
    const recs = nums.map((v, i) => ({ key: v % 100, id: i, name: wordOf(v) }));
    recs.sort((p, q) => p.key - q.key);
    const recs2 = mergeSort(nums.map((v, i) => ({ key: v % 100, id: i, name: wordOf(v) })),
      (p, q) => p.key - q.key);
    for (let i = 0; i < recs.length; i++) {
      if (recs[i].id !== recs2[i].id) {
        ok = false;
      }
    }

    // strings
    const words = nums.slice(0, SIZE >> 1).map(wordOf);
    words.sort((x, y) => (x < y ? -1 : (x > y ? 1 : 0)));
    const words2 = mergeSort(nums.slice(0, SIZE >> 1).map(wordOf),
      (x, y) => (x < y ? -1 : (x > y ? 1 : 0)));
    ok = ok && sameArray(words, words2);

    let h = 0;
    for (let i = 0; i < a1.length; i += 97) {
      h = (h * 31 + a1[i] + recs[i].id) % 1000000007;
    }
    checksum = (checksum * 7 + h) % 1000000007;
    console.log("round " + r + ": " + a1[0] + " " + a1[a1.length - 1] + " " + words[0] + " " +
      words[words.length - 1] + " " + recs[0].id + " " + h);
  }
  console.log("ok " + ok);
  console.log("checksum " + checksum);
}

main();
