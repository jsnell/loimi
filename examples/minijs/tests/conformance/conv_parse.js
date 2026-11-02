console.log(parseInt("42"), parseInt("  42px"), parseInt("-17"), parseInt("+5"), parseInt("3.99"), parseInt("-3.99"));
console.log(String(parseInt("")), String(parseInt("abc")), String(parseInt("px42")), parseInt("0x1F"), parseInt("0xff"), parseInt("0x10", 16));
console.log(parseInt("ff", 16), parseInt("FF", 16), parseInt("777", 8), parseInt("101", 2), parseInt("z", 36), parseInt("Zz", 36));
console.log(parseInt("12", 10), parseInt("19", 8), String(parseInt("2", 2)), parseInt("1e3"), parseInt("123456789012345678901234"));
console.log(parseInt("  \t\n-0x1a"), parseInt("0010"), parseInt("9007199254740993"), parseInt("10", 36));
console.log(parseFloat("3.14"), parseFloat("  3.14abc"), parseFloat("-.5"), parseFloat("1e3"), parseFloat("1e-3x"), parseFloat("2.5E+2"));
console.log(String(parseFloat("")), String(parseFloat("abc")), parseFloat("0x10"), parseFloat("1.2.3"), parseFloat("Infinity"), parseFloat("-Infinityx"));
console.log(parseFloat("  42  "), parseFloat("007.5"), parseFloat("1e"), parseFloat(".e1"), parseFloat("5."), parseFloat("1e400"));
console.log(1 / parseFloat("-0"), parseFloat("0.1") + parseFloat("0.2"));
