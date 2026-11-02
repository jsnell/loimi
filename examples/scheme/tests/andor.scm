; and / or / not compiled as branches: values in tail, value and effect
; positions, with boolean and non-boolean operands.

(define (t-and x y) (and (pair? x) (null? y)))
(define (t-and-val x) (and (pair? x) (car x)))
(define (t-or x y) (or (null? x) (pair? y)))
(define (t-or-val x) (or (null? x) (and (pair? x) (car x)) 'none))
(define (t-or-mixed x) (or (pair? x) x (null? x)))
(define (t-not-or x y) (not (or (null? x) (null? y))))
(display (list (t-and '(1) '()) (t-and '(1) '(2)) (t-and 3 '())
               (t-and-val '(7)) (t-and-val 3)
               (t-or '() 1) (t-or 1 '(2)) (t-or 1 2)
               (t-or-val '()) (t-or-val '(8)) (t-or-val '(#f)) (t-or-val 4)
               (t-or-mixed '(1)) (t-or-mixed 5) (t-or-mixed #f)
               (t-not-or '(1) '(2)) (t-not-or '() '(2))))
(newline)
; expect: (#t #f #f 7 #f #t #t #f #t 8 none none #t 5 #f #t #f)
; In value position and in if tests.
(define (v x) (list (and (pair? x) (car x)) (or (null? x) (car x)) (if (or (null? x) (eq? (car x) 'a)) 'yes 'no)
                    (if (and (pair? x) (not (null? (cdr x)))) 'long 'short)))
(display (list (v '()) (v '(a)) (v '(b c))))
(newline)
; expect: ((#f #t yes short) (a a yes short) (b b no long))
; Constants and nesting.
(define (c x) (list (and #t x) (or #f x) (and x #f) (or (and x #t) 'no) (if (or #f (and #t x)) 1 2)))
(display (list (c 5) (c #f)))
(newline)
; expect: ((5 5 #f #t 1) (#f #f #f no 2))
; In effect position with side effects (only evaluated as far as needed).
(define log '())
(define (note x r) (set! log (cons x log)) r)
(define (eff) (and (note 1 #t) (note 2 #f) (note 3 #t)) (or (note 4 #f) (note 5 5) (note 6 6)) 'done)
(display (list (eff) (reverse log)))
(newline)
; expect: (done (1 2 4 5))
; Tail calls through and/or stay tail calls.
(define (walk n) (or (= n 0) (and (> n 0) (walk (- n 1)))))
(display (walk 1000000))
(newline)
; expect: #t
