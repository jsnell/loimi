; Syntax errors are reported with their line.
(display 1)
(define (f x)
  (+ x 1)
; expect-exit: 1
; expect-stderr: error: line 3: unterminated list
