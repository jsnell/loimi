var inputs = ["0", "42", "  42  ", "\t\n 7 \r\n", "-3.5", "1e3", "1E-2", "2.5e-3", ".5", "0x1f", "0xff", "0xABCDEF",
  "abc", "12abc", "1 2", "0.1", "1.7976931348623157e308", "1e400", "-0", "x12", "0x", "1e", "--1", "1.2.3"];
for (var i = 0; i < inputs.length; i++) {
  console.log("[" + inputs[i] + "]", String(Number(inputs[i])));
}
console.log(Number("12") + Number("30"), Number("3") * Number("0x10"));
console.log(1 / Number("-0"), Number(5), Number(-2.5));
