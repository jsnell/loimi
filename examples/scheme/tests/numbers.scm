; Numbers: fixnums, flonums, overflow to bignum (see bignums.scm), arithmetic builtins.

(define (show x . xs) (display x) (for-each (lambda (x) (display " ") (display x)) xs) (newline))

(show (+ 1 2) (- 10 4) (* 6 7) (/ 12 4))
; expect: 3 6 42 3
(show (+) (*) (- 5) (+ 1 2 3 4) (* 1 2 3 4) (- 10 1 2 3))
; expect: 0 1 -5 10 24 4
(show (/ 1 2) (/ 7 2.0) (/ 6 -3) (exact->inexact 1))
; expect: 0.5 3.5 -2 1.0
(show 1.5 -0.25 3.0 1e21 (/ 1.0 3))
; expect: 1.5 -0.25 3.0 1e+21 0.3333333333333333
(show (+ 1 2.5) (* 2 1.5) (- 1.5 1) (exact->inexact 7))
; expect: 3.5 3.0 0.5 7.0
; Fixnums are 32-bit; overflow produces a bignum.
(show (+ 2147483647 1) (* 65536 65536) (- -2147483648 1))
; expect: 2147483648 4294967296 -2147483649
(show 2147483647 -2147483648 3000000000)
; expect: 2147483647 -2147483648 3000000000
(show (quotient 17 5) (remainder 17 5) (modulo 17 5))
; expect: 3 2 2
(show (quotient -17 5) (remainder -17 5) (modulo -17 5) (modulo 17 -5))
; expect: -3 -2 3 -3
(show (= 1 1) (= 1 1.0) (< 1 2) (< 2 1) (<= 2 2) (>= 1 2) (> 3 2 1) (< 1 3 2))
; expect: #t #t #t #f #t #f #t #f
(show (zero? 0) (zero? 1) (zero? 0.0) (positive? 3) (negative? -3) (even? 10) (odd? 10))
; expect: #t #f #t #t #t #t #f
(show (abs -5) (abs 5) (abs -2.5) (min 3 1 2) (max 3 1 2) (max 1 2.0))
; expect: 5 5 2.5 1 3 2.0
(show (sqrt 16) (sqrt 2) (expt 2 10) (expt 2 40) (expt 2.0 0.5))
; expect: 4 1.4142135623730951 1024 1099511627776 1.4142135623730951
(show (floor 2.5) (ceiling 2.5) (round 2.5) (round 3.5) (truncate -2.7) (floor 7))
; expect: 2.0 3.0 2.0 4.0 -2.0 7
(show (exact->inexact 1) (inexact->exact 4.0) (exact 2.0) (inexact 3))
; expect: 1.0 4 2 3.0
(show (number? 1) (number? 'a) (integer? 2.0) (integer? 2.5) (exact? 1) (inexact? 1.0))
; expect: #t #f #t #f #t #t
(show (number->string 42) (number->string 2.5) (string->number "17") (string->number "1e3") (string->number "x"))
; expect: 42 2.5 17 1000.0 #f
(show (gcd 12 18) (lcm 4 6) (square 9))
; expect: 6 12 81
(show (eqv? 2.0 2.0) (eq? 'a 'a) (eqv? 100 100) (equal? 1.0 1))
; expect: #t #t #t #f
(define x 10)
(set! x (+ x 0.5))
(show x (- x) (* x 2))
; expect: 10.5 -10.5 21.0
