; Calling a non-procedure, and vector index checks.
(define v (vector 1 2))
(display (vector-ref v 1))
(newline)
; expect: 2
(vector-ref v 2)
; expect-exit: 1
; expect-stderr: error: line 6: index out of range: 2
