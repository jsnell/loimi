; Escape-only call/cc, values / call-with-values, (import ...), 4-level cXr.
(import (scheme base) (scheme write))
(define (show x) (display x) (newline))
(show (call-with-current-continuation (lambda (k) (+ 1 (k 42)))))
(show (call/cc (lambda (k) 5)))
(define (find-first pred l)
  (call/cc (lambda (return)
    (for-each (lambda (x) (if (pred x) (return x))) l)
    #f)))
(show (list (find-first even? '(1 3 4 5)) (find-first even? '(1 3))))
; nested escapes past inner continuations
(show (call/cc (lambda (outer) (call/cc (lambda (inner) (outer 'out))) 'not-here)))
; escape from deep non-tail recursion
(define (walk n k) (if (= n 0) (k 'bottom) (+ 1 (walk (- n 1) k))))
(show (call/cc (lambda (k) (walk 10000 k))))
; tail-called call/cc
(define (tc) (call/cc (lambda (k) (k 'tail))))
(show (tc))
(show (call-with-values (lambda () (values 1 2 3)) list))
(show (call-with-values (lambda () 7) (lambda (x) (* x 2))))
(show (call-with-values (lambda () (values)) (lambda () 'none)))
(show (caddar '((1 2 3))))
(show (procedure? call/cc))
(define saved #f)
(show (+ 1 (call/cc (lambda (k) (set! saved k) 1))))
(saved 5)
; expect: 42
; expect: 5
; expect: (4 #f)
; expect: out
; expect: bottom
; expect: tail
; expect: (1 2 3)
; expect: 14
; expect: none
; expect: 3
; expect: #t
; expect: 2
; expect-exit: 1
; expect-stderr: error: line 26: continuation invoked after its extent ended (only escaping continuations are supported)
