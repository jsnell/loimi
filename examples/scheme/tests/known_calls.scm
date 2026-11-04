; Calls of known top-level procedures (CALLK: the closure is a constant):
; fixed and rest parameters, tail calls, aliases, and calls that may run
; before the definition.

(define (h a . r) (list a r))
(define (k) (h 1 2 3))
(define (k0) (h 1))
(display (list (k) (k0) (h 5 6)))
(newline)
; expect: ((1 (2 3)) (1 ()) (5 (6)))
(define (lp n acc) (if (= n 0) acc (lp (- n 1) (h n acc))))
(display (lp 3 0))
(newline)
; expect: (1 ((2 ((3 (0))))))
; Tail calls of a procedure with a rest parameter.
(define (count-rest . r) (length r))
(define (tail-rest x) (count-rest x x x))
(display (tail-rest 0))
(newline)
; expect: 3
; The constant closure is the value of the global.
(define (same) h)
(display (eq? (same) h))
(newline)
; expect: #t
; An alias of a procedure too big to inline.
(define (big x)
  (let loop ((i 0) (acc 0))
    (if (< i x) (loop (+ i 1) (+ acc (* i i) (quotient i 3) (remainder i 7) (* 2 i))) acc)))
(define big2 big)
(define (use) (big2 10))
(display (use))
(newline)
; expect: 411
; A procedure that a form runs before a later definition, and one whose
; closure escapes (red-edges in conform): both still find later
; definitions bound when they run.
(define (make-getter) (lambda (x) (later-def x)))
(define getter (make-getter))
(define (later-def x) (* x 10))
(display (getter 4))
(newline)
; expect: 40
(define (g) (f 1))
(display (g))
(define (f x) x)
; expect-exit: 1
; expect-stderr: error: line 44: unbound variable: f
