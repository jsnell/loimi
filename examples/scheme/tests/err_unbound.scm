; An unbound variable.
(define (f) (+ 1 undefined-var))
(f)
; expect-exit: 1
; expect-stderr: error: line 2: unbound variable: undefined-var
