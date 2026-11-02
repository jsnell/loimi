function f(x) { return String(x); }
console.log(Math.floor(1.7), Math.floor(-1.7), Math.ceil(1.2), Math.ceil(-1.2), Math.trunc(2.9), Math.trunc(-2.9));
console.log(Math.round(2.5), Math.round(-2.5), Math.round(0.49999999999999994), Math.round(-0.6), Math.round(1.4999), Math.round(3));
console.log(Math.abs(-5), Math.abs(5), Math.abs(-0.25), Math.abs(-Infinity), f(Math.abs(NaN)));
console.log(Math.sqrt(16), Math.sqrt(2), f(Math.sqrt(-1)), Math.sqrt(0), Math.sqrt(Infinity));
console.log(Math.sign(-3), Math.sign(3), Math.sign(0), f(Math.sign(NaN)), Math.sign(-Infinity));
console.log(Math.min(3, 1, 2), Math.max(3, 1, 2), Math.min(), Math.max(), f(Math.min(1, NaN)), Math.max(-1, -5), Math.min(5), Math.max(1, 2, 3, 4, 5, 6, 7));
console.log(Math.pow(2, 8), Math.pow(9, 0.5), Math.pow(-2, 3), Math.pow(0.5, 3), f(Math.pow(NaN, 0)));
console.log(Math.PI, Math.E, Math.PI.toFixed(10), (Math.E * 2).toFixed(6));
console.log(Math.sin(0), Math.cos(0), Math.tan(0), Math.atan(0), Math.atan2(0, 1), Math.exp(0), Math.log(1));
console.log(Math.sin(Math.PI / 2), Math.cos(Math.PI), (Math.atan(1) * 4).toFixed(12), (Math.atan2(1, 1) * 4).toFixed(12), Math.atan2(-1, -1).toFixed(12));
console.log(Math.exp(1).toFixed(12), Math.log(Math.E).toFixed(12), Math.log(0), f(Math.log(-1)), Math.exp(-Infinity), Math.exp(Infinity));
console.log(Math.sin(1).toFixed(12), Math.cos(1).toFixed(12), Math.tan(1).toFixed(12), Math.exp(2).toFixed(10), Math.log(10).toFixed(12));
console.log(Math.atan2(1, 0).toFixed(12), Math.atan2(0, -1).toFixed(12), Math.atan2(-1, 0).toFixed(12), Math.atan(Infinity).toFixed(12));
console.log(Math.floor(1e300), Math.ceil(-1e-300) === 0, Math.trunc(4294967296.5), Math.round(4294967296.5), Math.floor(-2147483648.5));
var total = 0;
for (var i = 0; i < 100; i++) { total += Math.floor(Math.sqrt(i)) + Math.round(Math.sin(i) * 100); }
console.log(total);
console.log(Math.max(Math.min(15, 10), 0), Math.floor(7 / 2), Math.ceil(7 / 2), Math.abs(Math.floor(-0.5)));
console.log(typeof Math, typeof Math.PI, Math.round(-0.5) === 0, 1 / Math.round(-0.5));
