var s = "Hello, World";
console.log(s.charCodeAt(0), s.charCodeAt(4), String(s.charCodeAt(99)), String(s.charCodeAt(-1)));
console.log(s.charAt(0), "[" + s.charAt(99) + "]", "[" + s.charAt(-1) + "]", s.charAt(7));
console.log(s.indexOf("o"), s.indexOf("o", 5), s.indexOf("o", 9), s.indexOf("xyz"), s.indexOf(""), s.indexOf("", 5), s.indexOf("Hello"), s.indexOf("o", -5));
console.log(s.lastIndexOf("o"), s.lastIndexOf("l"), s.lastIndexOf("H"), s.lastIndexOf("z"), s.lastIndexOf("World"));
console.log(s.slice(7), s.slice(0, 5), s.slice(-5), s.slice(-5, -1), s.slice(3, 2) + "|", s.slice(100) + "|", s.slice(-100, 2), s.slice());
console.log(s.substring(7), s.substring(0, 5), s.substring(5, 0), s.substring(-3, 2), s.substring(3, 100), s.substring(4, 4) + "|");
console.log(s.toUpperCase(), s.toLowerCase(), "MiXeD 123 _!".toLowerCase(), "abc-xyz".toUpperCase());
console.log("[" + "  pad  ".trim() + "]", "[" + "\t\n x y \r\n".trim() + "]", "[" + "".trim() + "]", "[" + "   ".trim() + "]");
console.log("ab".repeat(3), "[" + "x".repeat(0) + "]", "-".repeat(10), "".repeat(5).length);
console.log(s.startsWith("Hell"), s.startsWith("World"), s.startsWith(""), s.endsWith("World"), s.endsWith("Hello"), s.endsWith(""));
console.log(s.includes("lo, W"), s.includes("low"), s.includes(""), "".includes(""), "".includes("a"));
console.log(String.fromCharCode(72, 105), String.fromCharCode(), String.fromCharCode(65), String.fromCharCode(48 + 9) + String.fromCharCode(10).length);
var codes = [];
for (var i = 0; i < 5; i++) { codes.push(s.charCodeAt(i)); }
console.log(codes.join(" "));
var built = "";
for (var c = 32; c < 127; c++) { built += String.fromCharCode(c); }
console.log(built.length, built.slice(1, 48), built.slice(48));
var lit = "abc";
console.log(lit.toUpperCase(), lit, "Done".length);
