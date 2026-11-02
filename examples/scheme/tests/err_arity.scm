; Calling a procedure with the wrong number of arguments.
(define (two a b) a)
(two 1)
; expect-exit: 1
; expect-stderr: error: line 3: two: wrong number of arguments (expected 2, got 1)
