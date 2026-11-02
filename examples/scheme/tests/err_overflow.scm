; Unbounded non-tail recursion fails cleanly.
(define (inf n) (+ 1 (inf n)))
(inf 0)
; expect-exit: 1
; expect-stderr: stack overflow
