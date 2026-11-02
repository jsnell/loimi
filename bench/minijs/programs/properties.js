// Properties: property access on objects of many different shapes.
// Polymorphic and megamorphic call sites (the same `o.x`, `o.weight()` site
// sees a dozen shapes), objects whose property sets differ only in order,
// properties added after construction, deleted properties, computed keys,
// prototype-chain reads at different depths and dictionary-style objects.
const ROUNDS = 1600;
const N_OBJECTS = 2400;

let seed = 7;
function rand() {
  seed = (seed * 16807) % 2147483647;
  return seed;
}

// --- old-style constructors with different field orders ---
function PointXY(x, y) { this.x = x; this.y = y; }
PointXY.prototype.weight = function () { return this.x + this.y; };

function PointYX(x, y) { this.y = y; this.x = x; }
PointYX.prototype.weight = function () { return this.x - this.y; };

function Point3(x, y, z) { this.x = x; this.y = y; this.z = z; }
Point3.prototype.weight = function () { return this.x + this.y + this.z; };

function Tagged(x, y) { this.tag = "t"; this.x = x; this.y = y; this.extra = 1; }
Tagged.prototype.weight = function () { return this.x * 2 + this.extra; };

// --- a class hierarchy: methods found at different prototype depths ---
class Base {
  constructor(x, y) { this.x = x; this.y = y; }
  weight() { return this.x; }
  kind() { return 1; }
}
class Mid extends Base {
  constructor(x, y) { super(x, y); this.m = 3; }
  kind() { return 2; }
}
class Leaf extends Mid {
  constructor(x, y) { super(x, y); this.l = 5; }
  weight() { return super.weight() + this.l; }
}
class Other extends Base {
  constructor(x, y) { super(y, x); this.o = 0; }
  weight() { return this.y * 3; }
}

// --- prototype-less style via Object.create ---
const protoA = { weight() { return this.x + 100; }, kind() { return 7; } };
const protoB = Object.create(protoA);
protoB.weight = function () { return this.y + 200; };

function makeObject(i, x, y) {
  switch (i % 14) {
    case 0: return new PointXY(x, y);
    case 1: return new PointYX(x, y);
    case 2: return new Point3(x, y, i % 5);
    case 3: return new Tagged(x, y);
    case 4: return new Base(x, y);
    case 5: return new Mid(x, y);
    case 6: return new Leaf(x, y);
    case 7: return new Other(x, y);
    case 8: return { x: x, y: y, weight() { return this.x * this.y; } };
    case 9: return { y: y, x: x, w: 2, weight() { return this.w * this.x; } };
    case 10: {
      const o = Object.create(protoA);
      o.x = x;
      o.y = y;
      return o;
    }
    case 11: {
      const o = Object.create(protoB);
      o.y = y;
      o.x = x;
      o.q = i;
      return o;
    }
    case 12: {
      // Grown incrementally with computed keys, then one key deleted.
      const o = { a: 1 };
      o["k" + (i % 4)] = i;
      o.x = x;
      o.y = y;
      delete o.a;
      o.weight = function () { return this.x + this.y + 1; };
      return o;
    }
    default: {
      const o = new PointXY(x, y);
      o["extra" + (i % 3)] = 1;      // shapes forked from PointXY
      return o;
    }
  }
}

function build(n) {
  const objs = [];
  for (let i = 0; i < n; i++) objs.push(makeObject(i, rand() % 100, rand() % 100));
  return objs;
}

// The hot loops: one call site / property read site, many shapes.
function sumX(objs) {
  let s = 0;
  for (let i = 0; i < objs.length; i++) s += objs[i].x;
  return s;
}

function sumWeights(objs) {
  let s = 0;
  for (let i = 0; i < objs.length; i++) s += objs[i].weight();
  return s;
}

function sumKinds(objs) {
  let s = 0;
  for (const o of objs) {
    if ("kind" in o) s += o.kind();
    if (o.z !== undefined) s += o.z;
    if (o.hasOwnProperty("extra1")) s += 11;
  }
  return s;
}

function mutate(objs, round) {
  for (let i = 0; i < objs.length; i++) {
    const o = objs[i];
    o.x = (o.x + round + i) % 101;
    if ((i + round) % 9 === 0) o.y = o.y + 1;
  }
}

const FIELD_NAMES = ["alpha", "beta", "gamma", "delta", "eps", "zeta", "eta", "theta"];

// Dictionary-style objects: keys chosen at run time, read back through
// computed member access.
function dictWork(round) {
  const d = {};
  let s = 0;
  for (let i = 0; i < 400; i++) {
    const k = FIELD_NAMES[(i * 7 + round) % FIELD_NAMES.length] + (i % 25);
    d[k] = (d[k] === undefined ? 0 : d[k]) + i;
  }
  const keys = Object.keys(d);
  for (const k of keys) s += d[k] * k.length;
  return s + keys.length;
}

let check = 0;
const objs = build(N_OBJECTS);
for (let r = 0; r < ROUNDS; r++) {
  const a = sumX(objs);
  const b = sumWeights(objs);
  const c = sumKinds(objs);
  const d = dictWork(r);
  mutate(objs, r);
  check = (check * 7 + a + b * 3 + c * 5 + d) | 0;
}
console.log("properties", ROUNDS, N_OBJECTS, check);
console.log("final", sumX(objs), sumWeights(objs), sumKinds(objs));
