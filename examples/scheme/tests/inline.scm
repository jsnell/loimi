; Inlining of small known procedures: the result must be as if called.

; Arguments are evaluated exactly once, even when the parameter is unused
; or used twice.
(define n 0)
(define (next!) (set! n (+ n 1)) n)
(define (ignore x) 'ignored)
(define (twice x) (+ x x))
(display (list (ignore (next!)) (twice (next!)) n))
(newline)
; expect: (ignored 4 2)
; A parameter assigned in the body does not change the caller's variable.
(define (inc-local x) (set! x (+ x 1)) x)
(define (caller) (let ((y 5)) (list (inc-local y) y)))
(display (caller))
(newline)
; expect: (6 5)
; An argument variable assigned by the caller after the call.
(define (id x) x)
(define (later) (let ((y 1)) (let ((r (id y))) (set! y 2) (list r y))))
(display (later))
(newline)
; expect: (1 2)
; Nested inlined calls (inc twice) with the same body: distinct variables.
(define (add1 x) (let ((t (+ x 1))) t))
(display (add1 (add1 (add1 1))))
(newline)
; expect: 4
; Tail calls in an inlined body stay tail calls: a loop of 3000000
; iterations through a small helper in tail position.
(define (helper k) (if (> k 0) (count (- k 1)) 'counted))
(define (count k) (helper k))
(display (count 3000000))
(newline)
; expect: counted
; A procedure that is set! is not inlined.
(define (greet) 'hello)
(define (use-greet) (greet))
(set! greet (lambda () 'changed))
(display (use-greet))
(newline)
; expect: changed
; A procedure defined twice is not inlined.
(define (ver) 1)
(define (use-ver) (ver))
(define (ver) 2)
(display (use-ver))
(newline)
; expect: 2
; Local procedures are inlined too, with their free variables.
(define (scale-all l k)
  (define (scale x) (* x k))
  (map (lambda (x) (scale x)) l))
(display (scale-all '(1 2 3) 10))
(newline)
; expect: (10 20 30)
; A local procedure that sets a variable of its enclosing procedure.
(define (counter-sum l)
  (let ((total 0))
    (define (add! x) (set! total (+ total x)))
    (for-each (lambda (x) (add! x)) l)
    (add! 100)
    total))
(display (counter-sum '(1 2 3)))
(newline)
; expect: 106
; A call that runs before the definition is still an unbound variable.
(define (too-early) (not-yet 1))
(too-early)
(define (not-yet x) x)
; expect-exit: 1
; expect-stderr: error: line 68: unbound variable: not-yet
