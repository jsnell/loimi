var $ = 1, _ = 2, $_$ = 3, a1 = 4, _private = 5, camelCase = 6, ALLCAPS = 7;
console.log($, _, $_$, a1, _private, camelCase, ALLCAPS);
var of = "of-as-name";
console.log(of);
var static_ = 1;
var constructor = "constructor-as-var";
console.log(constructor);
var static2 = "s2";
console.log(static2);
function $fn(x) { return x + 1; }
console.log($fn(1));
var obj = { of: 5, constructor2: 6, static: 7 };
console.log(obj.of, obj.constructor2, obj.static);
var longIdentifierNameWithManyCharacters_0123456789 = "long";
console.log(longIdentifierNameWithManyCharacters_0123456789);
var undefinedLike = undefined;
console.log(undefinedLike, typeof undefined);
