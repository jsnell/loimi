function work() {
  try {
    console.log("in try");
    throw new Error("escapes");
  } finally {
    console.log("in finally");
  }
}
try {
  work();
} catch (e) {
  console.log("caught once:", e.message);
  throw e;
} finally {
  console.log("outer finally");
}
console.log("never");
