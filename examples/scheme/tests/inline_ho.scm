; Inlining of procedures that take procedures: map / for-each with one
; list, and small user-defined higher-order procedures called with a
; lambda or a known procedure. The result must be as if called.

(define (square x) (* x x))
(display (list (map square '(1 2 3)) (map (lambda (x) (+ x 1)) '(1 2 3)) (map car '()) (map square '())))
(newline)
; expect: ((1 4 9) (2 3 4) () ())
; for-each runs in order, and the lambda can assign captured variables.
(define (sum-list l) (let ((s 0)) (for-each (lambda (x) (set! s (+ s x))) l) s))
(define order '())
(for-each (lambda (x) (set! order (cons x order))) '(1 2 3))
(display (list (sum-list '(1 2 3 4)) order))
(newline)
; expect: (10 (3 2 1))
; Nested maps; the inner lambda captures the outer one's parameter.
(display (map (lambda (x) (map (lambda (y) (list x y)) '(a b))) '(1 2)))
(newline)
; expect: (((1 a) (1 b)) ((2 a) (2 b)))
; A user-defined fold called with a lambda, and with a known procedure.
(define (fold f acc l) (if (null? l) acc (fold f (f (car l) acc) (cdr l))))
(define (plus a b) (+ a b))
(display (list (fold (lambda (x acc) (cons x acc)) '() '(1 2 3)) (fold plus 0 '(1 2 3 4))))
(newline)
; expect: ((3 2 1) 10)
(define (twice f x) (f (f x)))
(define (twice-helper x) (+ x 100))
(display (list (twice (lambda (x) (* x 3)) 2) (twice square 3) (twice twice-helper 1)))
(newline)
; expect: (18 81 201)
; A lambda that escapes from the inlined body still works.
(define (compose f g) (lambda (x) (f (g x))))
(display ((compose (lambda (x) (* 2 x)) square) 5))
(newline)
; expect: 50
; Each map call makes fresh closures over fresh bindings.
(define (adders l) (map (lambda (n) (lambda (x) (+ x n))) l))
(display (map (lambda (f) (f 10)) (adders '(1 2 3))))
(newline)
; expect: (11 12 13)
; map with several lists and a redefined map are unaffected.
(display (map + '(1 2) '(10 20)))
(newline)
; expect: (11 22)
; Errors inside an inlined map report the caller's line.
(define (bad l) (map (lambda (x) (car x)) l))
(bad '((1) 2))
; expect-exit: 1
; expect-stderr: error: line 46: car: expected pair, got 2
