// DeltaBlue: incremental one-way constraint solver (Maloney / Freeman-Benson,
// as in the V8/Octane suite), rewritten with ES classes and `extends`.
const ITERATIONS = 350;
const CHAIN_SIZE = 100;

class OrderedCollection {
  constructor() {
    this.elms = [];
  }
  add(elm) {
    this.elms.push(elm);
  }
  at(index) {
    return this.elms[index];
  }
  size() {
    return this.elms.length;
  }
  removeFirst() {
    return this.elms.pop();
  }
  remove(elm) {
    let index = 0;
    let skipped = 0;
    for (let i = 0; i < this.elms.length; i++) {
      const value = this.elms[i];
      if (value !== elm) {
        this.elms[index] = value;
        index++;
      } else {
        skipped++;
      }
    }
    for (let i = 0; i < skipped; i++) this.elms.pop();
  }
}

class Strength {
  constructor(strengthValue, name) {
    this.strengthValue = strengthValue;
    this.name = name;
  }
  static stronger(s1, s2) {
    return s1.strengthValue < s2.strengthValue;
  }
  static weaker(s1, s2) {
    return s1.strengthValue > s2.strengthValue;
  }
  static weakestOf(s1, s2) {
    return Strength.weaker(s1, s2) ? s1 : s2;
  }
  static strongest(s1, s2) {
    return Strength.stronger(s1, s2) ? s1 : s2;
  }
  nextWeaker() {
    switch (this.strengthValue) {
      case 0: return Strength.WEAKEST;
      case 1: return Strength.WEAK_DEFAULT;
      case 2: return Strength.NORMAL;
      case 3: return Strength.STRONG_DEFAULT;
      case 4: return Strength.PREFERRED;
      case 5: return Strength.REQUIRED;
    }
    throw new Error("bad strength");
  }
}

Strength.REQUIRED = new Strength(0, "required");
Strength.STRONG_PREFERRED = new Strength(1, "strongPreferred");
Strength.PREFERRED = new Strength(2, "preferred");
Strength.STRONG_DEFAULT = new Strength(3, "strongDefault");
Strength.NORMAL = new Strength(4, "normal");
Strength.WEAK_DEFAULT = new Strength(5, "weakDefault");
Strength.WEAKEST = new Strength(6, "weakest");

class Constraint {
  constructor(strength) {
    this.strength = strength;
  }
  addConstraint() {
    this.addToGraph();
    planner.incrementalAdd(this);
  }
  satisfy(mark) {
    this.chooseMethod(mark);
    if (!this.isSatisfied()) {
      if (this.strength === Strength.REQUIRED) {
        throw new Error("Could not satisfy a required constraint!");
      }
      return null;
    }
    this.markInputs(mark);
    const out = this.output();
    const overridden = out.determinedBy;
    if (overridden !== null) overridden.markUnsatisfied();
    out.determinedBy = this;
    if (!planner.addPropagate(this, mark)) {
      throw new Error("Cycle encountered");
    }
    out.mark = mark;
    return overridden;
  }
  destroyConstraint() {
    if (this.isSatisfied()) planner.incrementalRemove(this);
    else this.removeFromGraph();
  }
  isInput() {
    return false;
  }
}

class UnaryConstraint extends Constraint {
  constructor(v, strength) {
    super(strength);
    this.myOutput = v;
    this.satisfied = false;
    this.addConstraint();
  }
  addToGraph() {
    this.myOutput.addConstraint(this);
    this.satisfied = false;
  }
  chooseMethod(mark) {
    this.satisfied = (this.myOutput.mark !== mark) &&
      Strength.stronger(this.strength, this.myOutput.walkStrength);
  }
  isSatisfied() {
    return this.satisfied;
  }
  markInputs(mark) {
  }
  output() {
    return this.myOutput;
  }
  recalculate() {
    this.myOutput.walkStrength = this.strength;
    this.myOutput.stay = !this.isInput();
    if (this.myOutput.stay) this.execute();
  }
  markUnsatisfied() {
    this.satisfied = false;
  }
  inputsKnown() {
    return true;
  }
  removeFromGraph() {
    if (this.myOutput !== null) this.myOutput.removeConstraint(this);
    this.satisfied = false;
  }
}

class StayConstraint extends UnaryConstraint {
  constructor(v, str) {
    super(v, str);
  }
  execute() {
  }
}

class EditConstraint extends UnaryConstraint {
  constructor(v, str) {
    super(v, str);
  }
  isInput() {
    return true;
  }
  execute() {
  }
}

const Direction = { NONE: 0, FORWARD: 1, BACKWARD: -1 };

class BinaryConstraint extends Constraint {
  constructor(var1, var2, strength) {
    super(strength);
    this.v1 = var1;
    this.v2 = var2;
    this.direction = Direction.NONE;
    // Subclasses call addConstraint() once they are fully initialized.
  }
  chooseMethod(mark) {
    if (this.v1.mark === mark) {
      this.direction = (this.v2.mark !== mark &&
        Strength.stronger(this.strength, this.v2.walkStrength))
        ? Direction.FORWARD : Direction.NONE;
    }
    if (this.v2.mark === mark) {
      this.direction = (this.v1.mark !== mark &&
        Strength.stronger(this.strength, this.v1.walkStrength))
        ? Direction.BACKWARD : Direction.NONE;
    }
    if (Strength.weaker(this.v1.walkStrength, this.v2.walkStrength)) {
      this.direction = Strength.stronger(this.strength, this.v1.walkStrength)
        ? Direction.BACKWARD : Direction.NONE;
    } else {
      this.direction = Strength.stronger(this.strength, this.v2.walkStrength)
        ? Direction.FORWARD : Direction.BACKWARD;
    }
  }
  addToGraph() {
    this.v1.addConstraint(this);
    this.v2.addConstraint(this);
    this.direction = Direction.NONE;
  }
  isSatisfied() {
    return this.direction !== Direction.NONE;
  }
  markInputs(mark) {
    this.input().mark = mark;
  }
  input() {
    return (this.direction === Direction.FORWARD) ? this.v1 : this.v2;
  }
  output() {
    return (this.direction === Direction.FORWARD) ? this.v2 : this.v1;
  }
  recalculate() {
    const ihn = this.input();
    const out = this.output();
    out.walkStrength = Strength.weakestOf(this.strength, ihn.walkStrength);
    out.stay = ihn.stay;
    if (out.stay) this.execute();
  }
  markUnsatisfied() {
    this.direction = Direction.NONE;
  }
  inputsKnown(mark) {
    const i = this.input();
    return i.mark === mark || i.stay || i.determinedBy === null;
  }
  removeFromGraph() {
    if (this.v1 !== null) this.v1.removeConstraint(this);
    if (this.v2 !== null) this.v2.removeConstraint(this);
    this.direction = Direction.NONE;
  }
}

class ScaleConstraint extends BinaryConstraint {
  constructor(src, scale, offset, dest, strength) {
    super(src, dest, strength);
    this.scale = scale;
    this.offset = offset;
    this.addConstraint();
  }
  addToGraph() {
    super.addToGraph();
    this.scale.addConstraint(this);
    this.offset.addConstraint(this);
  }
  removeFromGraph() {
    super.removeFromGraph();
    if (this.scale !== null) this.scale.removeConstraint(this);
    if (this.offset !== null) this.offset.removeConstraint(this);
  }
  markInputs(mark) {
    super.markInputs(mark);
    this.scale.mark = this.offset.mark = mark;
  }
  execute() {
    if (this.direction === Direction.FORWARD) {
      this.v2.value = this.v1.value * this.scale.value + this.offset.value;
    } else {
      this.v1.value = (this.v2.value - this.offset.value) / this.scale.value;
    }
  }
  recalculate() {
    const ihn = this.input();
    const out = this.output();
    out.walkStrength = Strength.weakestOf(this.strength, ihn.walkStrength);
    out.stay = ihn.stay && this.scale.stay && this.offset.stay;
    if (out.stay) this.execute();
  }
}

class EqualityConstraint extends BinaryConstraint {
  constructor(var1, var2, strength) {
    super(var1, var2, strength);
    this.addConstraint();
  }
  execute() {
    this.output().value = this.input().value;
  }
}

class Variable {
  constructor(name, initialValue) {
    this.value = initialValue;
    this.constraints = new OrderedCollection();
    this.determinedBy = null;
    this.mark = 0;
    this.walkStrength = Strength.WEAKEST;
    this.stay = true;
    this.name = name;
  }
  addConstraint(c) {
    this.constraints.add(c);
  }
  removeConstraint(c) {
    this.constraints.remove(c);
    if (this.determinedBy === c) this.determinedBy = null;
  }
}

class Planner {
  constructor() {
    this.currentMark = 0;
  }
  incrementalAdd(c) {
    const mark = this.newMark();
    let overridden = c.satisfy(mark);
    while (overridden !== null) overridden = overridden.satisfy(mark);
  }
  incrementalRemove(c) {
    const out = c.output();
    c.markUnsatisfied();
    c.removeFromGraph();
    const unsatisfied = this.removePropagateFrom(out);
    let strength = Strength.REQUIRED;
    do {
      for (let i = 0; i < unsatisfied.size(); i++) {
        const u = unsatisfied.at(i);
        if (u.strength === strength) this.incrementalAdd(u);
      }
      strength = strength.nextWeaker();
    } while (strength !== Strength.WEAKEST);
  }
  newMark() {
    return ++this.currentMark;
  }
  makePlan(sources) {
    const mark = this.newMark();
    const plan = new Plan();
    const todo = sources;
    while (todo.size() > 0) {
      const c = todo.removeFirst();
      if (c.output().mark !== mark && c.inputsKnown(mark)) {
        plan.addConstraint(c);
        c.output().mark = mark;
        this.addConstraintsConsumingTo(c.output(), todo);
      }
    }
    return plan;
  }
  extractPlanFromConstraints(constraints) {
    const sources = new OrderedCollection();
    for (let i = 0; i < constraints.size(); i++) {
      const c = constraints.at(i);
      if (c.isInput() && c.isSatisfied()) sources.add(c);
    }
    return this.makePlan(sources);
  }
  addPropagate(c, mark) {
    const todo = new OrderedCollection();
    todo.add(c);
    while (todo.size() > 0) {
      const d = todo.removeFirst();
      if (d.output().mark === mark) {
        this.incrementalRemove(c);
        return false;
      }
      d.recalculate();
      this.addConstraintsConsumingTo(d.output(), todo);
    }
    return true;
  }
  removePropagateFrom(out) {
    out.determinedBy = null;
    out.walkStrength = Strength.WEAKEST;
    out.stay = true;
    const unsatisfied = new OrderedCollection();
    const todo = new OrderedCollection();
    todo.add(out);
    while (todo.size() > 0) {
      const v = todo.removeFirst();
      for (let i = 0; i < v.constraints.size(); i++) {
        const c = v.constraints.at(i);
        if (!c.isSatisfied()) unsatisfied.add(c);
      }
      const determining = v.determinedBy;
      for (let i = 0; i < v.constraints.size(); i++) {
        const next = v.constraints.at(i);
        if (next !== determining && next.isSatisfied()) {
          next.recalculate();
          todo.add(next.output());
        }
      }
    }
    return unsatisfied;
  }
  addConstraintsConsumingTo(v, coll) {
    const determining = v.determinedBy;
    const cc = v.constraints;
    for (let i = 0; i < cc.size(); i++) {
      const c = cc.at(i);
      if (c !== determining && c.isSatisfied()) coll.add(c);
    }
  }
}

class Plan {
  constructor() {
    this.v = new OrderedCollection();
  }
  addConstraint(c) {
    this.v.add(c);
  }
  size() {
    return this.v.size();
  }
  constraintAt(index) {
    return this.v.at(index);
  }
  execute() {
    for (let i = 0; i < this.size(); i++) {
      const c = this.constraintAt(i);
      c.execute();
    }
  }
}

let planner = null;
let checksum = 0;

function chainTest(n) {
  planner = new Planner();
  let prev = null;
  let first = null;
  let last = null;
  for (let i = 0; i <= n; i++) {
    const name = "v" + i;
    const v = new Variable(name, 0);
    if (prev !== null) new EqualityConstraint(prev, v, Strength.REQUIRED);
    if (i === 0) first = v;
    if (i === n) last = v;
    prev = v;
  }
  new StayConstraint(last, Strength.STRONG_DEFAULT);
  const edit = new EditConstraint(first, Strength.PREFERRED);
  const edits = new OrderedCollection();
  edits.add(edit);
  const plan = planner.extractPlanFromConstraints(edits);
  for (let i = 0; i < 100; i++) {
    first.value = i;
    plan.execute();
    if (last.value !== i) throw new Error("Chain test failed.");
    checksum = (checksum + last.value) | 0;
  }
}

function change(v, newValue) {
  const edit = new EditConstraint(v, Strength.PREFERRED);
  const edits = new OrderedCollection();
  edits.add(edit);
  const plan = planner.extractPlanFromConstraints(edits);
  for (let i = 0; i < 10; i++) {
    v.value = newValue;
    plan.execute();
  }
  edit.destroyConstraint();
}

function projectionTest(n) {
  planner = new Planner();
  const scale = new Variable("scale", 10);
  const offset = new Variable("offset", 1000);
  let src = null;
  let dst = null;
  const dests = new OrderedCollection();
  for (let i = 0; i < n; i++) {
    src = new Variable("src" + i, i);
    dst = new Variable("dst" + i, i);
    dests.add(dst);
    new StayConstraint(src, Strength.NORMAL);
    new ScaleConstraint(src, scale, offset, dst, Strength.REQUIRED);
  }
  change(src, 17);
  if (dst.value !== 1170) throw new Error("Projection 1 failed");
  change(dst, 1050);
  if (src.value !== 5) throw new Error("Projection 2 failed");
  change(scale, 5);
  for (let i = 0; i < n - 1; i++) {
    if (dests.at(i).value !== i * 5 + 1000) throw new Error("Projection 3 failed");
  }
  change(offset, 2000);
  for (let i = 0; i < n - 1; i++) {
    if (dests.at(i).value !== i * 5 + 2000) throw new Error("Projection 4 failed");
  }
  for (let i = 0; i < n; i++) checksum = (checksum + dests.at(i).value) | 0;
}

for (let i = 0; i < ITERATIONS; i++) {
  chainTest(CHAIN_SIZE);
  projectionTest(CHAIN_SIZE);
}
console.log("deltablue", ITERATIONS, checksum);
