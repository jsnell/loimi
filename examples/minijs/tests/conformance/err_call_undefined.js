var o = { present() { return "here"; } };
console.log(o.present());
o.absent();
console.log("never");
