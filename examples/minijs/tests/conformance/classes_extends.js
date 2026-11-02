class Shape {
  constructor(name) { this.name = name; Shape.created = (Shape.created || 0) + 1; }
  area() { return 0; }
  describe() { return this.name + " area=" + this.area().toFixed(2); }
  kind() { return "shape"; }
  static make(n) { return new Shape(n); }
  static label() { return "Shape:" + this.tag(); }
  static tag() { return "base"; }
}
class Circle extends Shape {
  constructor(r) { super("circle"); this.r = r; }
  area() { return Math.PI * this.r * this.r; }
  kind() { return "round " + super.kind(); }
}
class Square extends Shape {
  constructor(s) { super("square"); this.s = s; }
  area() { return this.s * this.s; }
  static tag() { return "sq"; }
}
class Cube extends Square {
  constructor(s) { super(s); this.name = "cube"; }
  area() { return 6 * super.area(); }
  describe() { return "3D " + super.describe(); }
  kind() { return "boxy " + super.kind(); }
}
var shapes = [new Circle(1), new Square(2), new Cube(3), Shape.make("plain")];
for (const sh of shapes) { console.log(sh.describe(), sh.kind()); }
console.log(Shape.created, Square.label(), Cube.label(), Shape.label(), Cube.make("viaCube").name);
var cu = shapes[2];
console.log(Object.getPrototypeOf(cu) === Cube.prototype, Object.getPrototypeOf(Cube.prototype) === Square.prototype, Object.getPrototypeOf(Cube) === Square);
console.log(Object.keys(cu).join(","), cu.hasOwnProperty("area"));
class NoCtorChild extends Circle { extra() { return "child r=" + this.r; } }
var n = new NoCtorChild(5);
console.log(n.extra(), n.name, n.area().toFixed(3));
class Base2 { constructor() { this.order = ["base"]; } }
class Mid extends Base2 { constructor() { super(); this.order.push("mid"); } }
class Leaf extends Mid { constructor() { var before = "pre"; super(); this.order.push("leaf"); this.order.unshift(before); } }
console.log(new Leaf().order.join(">"));
class Acc { constructor() { this.total = 0; } add(v) { this.total += v; return this; } }
class LoudAcc extends Acc { add(v) { super.add(v * 2); return this; } }
console.log(new LoudAcc().add(1).add(2).total);
function OldStyle(v) { this.v = v; }
OldStyle.prototype.get = function () { return this.v; };
class NewStyle extends OldStyle { constructor() { super(41); } get2() { return super.get() + 1; } }
console.log(new NewStyle().get2(), new NewStyle().get());
class Arrowy extends Acc {
  constructor() { super(); this.fns = [1, 2, 3].map(x => () => super.add(x)); }
}
var ar = new Arrowy();
for (const f of ar.fns) { f(); }
console.log(ar.total);
