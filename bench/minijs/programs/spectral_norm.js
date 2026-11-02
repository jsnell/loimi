// spectral-norm, after the Computer Language Benchmarks Game.
// Stresses floating-point loops, function calls and array access.
const N = 700;

function A(i, j) {
  return 1 / (((i + j) * (i + j + 1)) / 2 + i + 1);
}

function Au(u, v) {
  const n = u.length;
  for (let i = 0; i < n; i++) {
    let t = 0;
    for (let j = 0; j < n; j++) {
      t += A(i, j) * u[j];
    }
    v[i] = t;
  }
}

function Atu(u, v) {
  const n = u.length;
  for (let i = 0; i < n; i++) {
    let t = 0;
    for (let j = 0; j < n; j++) {
      t += A(j, i) * u[j];
    }
    v[i] = t;
  }
}

function AtAu(u, v, w) {
  Au(u, w);
  Atu(w, v);
}

function spectralNorm(n) {
  const u = new Array(n).fill(1);
  const v = new Array(n).fill(0);
  const w = new Array(n).fill(0);
  for (let i = 0; i < 10; i++) {
    AtAu(u, v, w);
    AtAu(v, u, w);
  }
  let vBv = 0;
  let vv = 0;
  for (let i = 0; i < n; i++) {
    vBv += u[i] * v[i];
    vv += v[i] * v[i];
  }
  return Math.sqrt(vBv / vv);
}

console.log(spectralNorm(N).toFixed(9));
