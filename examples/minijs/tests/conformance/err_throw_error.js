console.log("before");
function fail(n) { if (n === 0) { throw new Error("uncaught failure"); } return fail(n - 1); }
console.log("still running");
fail(3);
console.log("never printed");
