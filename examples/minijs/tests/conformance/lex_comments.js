// Line comment at the start.
/* Block comment */ console.log("one"); // trailing comment
/* multi
   line
   comment with // inside and * stars ** */
console.log(/* inline */ "two" /* after */);
var x = 1 /* between */ + /* tokens */ 2;
console.log(x);
console.log("// not a comment", '/* nor this */');
/**/console.log("empty block comment");
	console.log("tab-indented line");
var y = 10 // comment right before newline
  ;
console.log(y);
// final comment without trailing newline handled below
console.log("crlf line");
console.log("after crlf");
// last line comment no newline