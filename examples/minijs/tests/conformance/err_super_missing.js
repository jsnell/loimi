class Base { constructor() { this.ok = true; } }
class Derived extends Base { constructor() { console.log("derived ctor runs"); } }
console.log("start");
var d = new Derived();
console.log("never", d.ok);
