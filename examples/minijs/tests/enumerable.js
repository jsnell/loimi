class C { constructor() { this.a = 1; } m() {} static s() {} }
C.prototype.extra = 2;
const o = new C();
const ks = []; for (const k in o) ks.push(k); console.log(ks.join(","));
function F() { this.x = 1; }
F.prototype.y = 3;
const f = new F(); const fk = []; for (const k in f) fk.push(k); console.log(fk.join(","));
Object.prototype.zz = 9;
const lit = { b: 1, 2: "two", 1: "one", a: 2 };
const lk = []; for (const k in lit) lk.push(k); console.log(lk.join(","), Object.keys(lit).join(","));
delete Object.prototype.zz;
const mk = []; for (const k in Math) mk.push(k); console.log(mk.length);
F.foo = 5; const sk = []; for (const k in F) sk.push(k); console.log(sk.join(","));
console.log(Object.keys(C.prototype).join(","), Object.keys(F.prototype).join(","));
