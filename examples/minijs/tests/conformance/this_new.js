function Point(x, y) { this.x = x; this.y = y; }
Point.prototype.norm2 = function () { return this.x * this.x + this.y * this.y; };
Point.prototype.add = function (o) { return new Point(this.x + o.x, this.y + o.y); };
var p = new Point(3, 4);
var q = p.add(new Point(1, 1));
console.log(p.norm2(), q.x, q.y, q.norm2(), Object.keys(p).join(","));
console.log(p.hasOwnProperty("x"), p.hasOwnProperty("norm2"), "norm2" in p);
function Factory() { this.ignored = true; return { made: "custom" }; }
var f = new Factory();
console.log(f.made, f.ignored, Object.getPrototypeOf(f) === Factory.prototype);
function Prim() { this.kept = 1; return 5; }
console.log(new Prim().kept);
function NoArgs() { this.v = "noargs"; }
console.log(new NoArgs().v, new NoArgs().v.length);
var obj = { name: "obj", who() { return this.name; }, nested: { name: "nested", who() { return this.name; } } };
console.log(obj.who(), obj.nested.who(), obj["who"](), obj.nested["who"]());
var detached = obj.who;
var other = { name: "other", who: detached };
console.log(other.who());
function Counter() {
  this.count = 0;
  var self = this;
  this.incLater = function () { return function () { self.count++; return self.count; }; };
  this.incArrow = () => { this.count += 10; return this.count; };
}
var c = new Counter();
var inc = c.incLater();
inc(); inc();
var ia = c.incArrow;
ia();
console.log(c.count);
var holder = {
  items: [1, 2, 3], factor: 3,
  scaled() { return this.items.map(x => x * this.factor); }
};
console.log(holder.scaled().join(","));
function Animal(name) { this.name = name; }
Animal.prototype.speak = function () { return this.name + " makes " + this.sound(); };
Animal.prototype.sound = function () { return "noise"; };
function Dog(name) { this.name = name; this.tricks = 0; }
Dog.prototype = Object.create(Animal.prototype);
Dog.prototype.sound = function () { return "woof"; };
var d = new Dog("rex");
console.log(d.speak(), new Animal("gen").speak(), Object.getPrototypeOf(Object.getPrototypeOf(d)) === Animal.prototype);
Point.prototype.label = "pt";
console.log(p.label, q.label);
var methods = { m1() { return 1; }, m2() { return 2; } };
var names = ["m1", "m2"];
var total = 0;
for (var i = 0; i < names.length; i++) { total += methods[names[i]](); }
console.log(total);
