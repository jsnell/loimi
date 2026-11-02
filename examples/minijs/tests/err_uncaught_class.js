// An uncaught exception from a method deep in a call chain ends the
// program after the output so far is flushed.
class Account {
  constructor(n) { this.balance = n; }
  withdraw(x) {
    if (x > this.balance) throw new Error("insufficient funds");
    this.balance -= x;
    return this.balance;
  }
}
const a = new Account(10);
console.log(a.withdraw(3));
try { a.withdraw(100); } catch (e) { console.log("caught:", e.message); }
[1, 2, 3].forEach(function (v) { console.log("step", v, a.withdraw(v * 3)); });
console.log("not reached");
