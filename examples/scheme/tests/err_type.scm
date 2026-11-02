; A run-time type error reports the source line and exits with status 1.
(display "before")
(newline)
; expect: before
(define (f x) (car x))
(f 5)
(display "not reached")
; expect-exit: 1
; expect-stderr: error: line 5: car: expected pair, got 5
