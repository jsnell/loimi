// Reads of absent properties, `in` and stores at sites that see many
// shapes (polymorphic and megamorphic caches), with prototype changes.
function P() {}
P.prototype.kind = function () { return "p"; };
const objs = [];
for (let i = 0; i < 20; i++) {
  const o = i % 2 === 0 ? new P() : {};
  o["f" + i] = i;
  o.x = i;
  objs.push(o);
}
function zs() {
  let s = "";
  for (const o of objs) s += o.z === undefined ? "." : o.z;
  return s;
}
function ins() {
  let s = "";
  for (const o of objs) s += ("z" in o ? "y" : "n") + ("kind" in o ? "k" : "-");
  return s;
}
console.log(zs(), ins());
P.prototype.z = "Z";
console.log(zs(), ins());
delete P.prototype.z;
Object.prototype.z = "O";
console.log(zs(), ins());
delete Object.prototype.z;
console.log(zs(), ins());
objs[3].z = 3;
console.log(zs(), ins());

function bump(round) {
  for (let i = 0; i < objs.length; i++) objs[i].x = objs[i].x + round;
}
for (let r = 0; r < 30; r++) bump(r);
console.log(objs.map(o => o.x).join(" "));
// A store site that adds properties to some shapes and overwrites others.
function setY(o, v) { o.y = v; }
for (let r = 0; r < 3; r++) for (const o of objs) setY(o, r);
console.log(objs.map(o => o.y + (o.hasOwnProperty("y") ? "" : "?")).join(" "));
