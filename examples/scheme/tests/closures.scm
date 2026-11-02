; Closures: captured variables, set! on captured variables (boxes),
; letrec of lambdas, rest arguments.

(define (show x . xs) (display x) (for-each (lambda (x) (display " ") (display x)) xs) (newline))

(define (make-counter)
  (let ((n 0))
    (lambda () (set! n (+ n 1)) n)))
(define c1 (make-counter))
(define c2 (make-counter))
(c1) (c1) (c2)
(show (c1) (c2))
; expect: 3 2
(define (make-account balance)
  (define (withdraw amt) (set! balance (- balance amt)) balance)
  (define (deposit amt) (set! balance (+ balance amt)) balance)
  (lambda (m amt) (if (eq? m 'w) (withdraw amt) (deposit amt))))
(define acc (make-account 100))
(acc 'w 30)
(show (acc 'd 5))
; expect: 75
(define (adder n) (lambda (x) (+ x n)))
(show ((adder 3) 4) (map (adder 10) '(1 2 3)))
; expect: 7 (11 12 13)
; Each iteration of a named let gets fresh bindings.
(define procs
  (let loop ((i 0) (acc '()))
    (if (= i 3) (reverse acc) (loop (+ i 1) (cons (lambda () i) acc)))))
(show (map (lambda (p) (p)) procs))
; expect: (0 1 2)
; ... and so does do.
(define procs2 '())
(do ((i 0 (+ i 1))) ((= i 3)) (set! procs2 (cons (lambda () (* i 10)) procs2)))
(show (map (lambda (p) (p)) procs2))
; expect: (20 10 0)
; A parameter that is captured and assigned.
(define (box-param x)
  (let ((get (lambda () x)))
    (set! x (* x 2))
    (get)))
(show (box-param 21))
; expect: 42
; Deeply nested capture.
(define (f1 a) (lambda (b) (lambda (c) (lambda (d) (list a b c d)))))
(show ((((f1 1) 2) 3) 4))
; expect: (1 2 3 4)
; A closure capturing a variable assigned in another closure.
(define (pair-of-closures)
  (let ((x 0))
    (cons (lambda () (set! x (+ x 1))) (lambda () x))))
(define pc (pair-of-closures))
((car pc)) ((car pc))
(show ((cdr pc)))
; expect: 2
; Rest arguments.
(define (r0 . xs) xs)
(define (r2 a b . xs) (list a b xs))
(show (r0) (r0 1) (r2 1 2) (r2 1 2 3 4))
; expect: () (1) (1 2 ()) (1 2 (3 4))
(show ((lambda args (length args)) 1 2 3))
; expect: 3
(show (apply r2 '(5 6 7)))
; expect: (5 6 (7))
; letrec with a closure captured before a later variable is initialized.
(define (early)
  (define get-y (lambda () y))
  (define y 42)
  (get-y))
(show (early))
; expect: 42
(define (early2)
  (define fs (list (lambda () z)))
  (define z 'zed)
  ((car fs)))
(show (early2))
; expect: zed
; Mutually recursive internal procedures.
(define (parity n)
  (define (e? n) (if (= n 0) 'even (o? (- n 1))))
  (define (o? n) (if (= n 0) 'odd (e? (- n 1))))
  (e? n))
(show (parity 7) (parity 10))
; expect: odd even
; Procedures as values.
(show (procedure? car) (procedure? show) (procedure? 'car) (procedure? (lambda () 1)))
; expect: #t #t #f #t
(define compose (lambda (f g) (lambda (x) (f (g x)))))
(show ((compose car cdr) '(1 2 3)))
; expect: 2
(define (curry f . args) (lambda more (apply f (append args more))))
(show ((curry + 1 2) 3 4))
; expect: 10
; Redefining a builtin globally disables its inline compilation.
(define (use-car x) (car x))
(define (car x) 'my-car)
(show (use-car '(1 2)) (map car '(1 2)))
; expect: my-car (my-car my-car)
