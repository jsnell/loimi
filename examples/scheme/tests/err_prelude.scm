; Errors inside builtins written in Scheme (map) report the caller's line.
(define l (list 1 2 (quote x)))
(map (lambda (x) (+ x 1))
     l)
; expect-exit: 1
; expect-stderr: error: line 3: +: expected number, got x
