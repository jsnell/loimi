console.log("single");
console.log("a", "b", "c");
console.log(1, 2.5, -3, 1e21, 1e-7);
console.log(true, false, null, undefined);
console.log();
console.log("");
console.log("", "");
console.log("x", "", "y");
console.log("  leading and trailing spaces  ");
console.log("mixed", 1, true, null, undefined, "end");
console.log(String([1, 2]), [3, 4].join(";"), String({}));
console.log("tab\tinside", "newline\ninside");
console.log(0.1 + 0.2, 100, 1 / 3);
console.log(NaN, Infinity, -Infinity);
var many = [];
for (var i = 0; i < 20; i++) { many.push(i); }
console.log(many.join(" "));
console.log("quotes ' and \" are printed raw");
console.log("backslash \\ printed once");
