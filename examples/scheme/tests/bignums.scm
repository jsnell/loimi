; Exact integers beyond the fixnums (48-bit; the 32-bit edges too): promotion on overflow,
; demotion of results that fit, mixed exact/inexact comparisons, radix
; input and output. Expected output: Guile 3.0.11's, which Gauche 0.9.15
; matches except that it writes two of the flonums as 1.099511627776e12
; and 1.2345678901234e13.
(import (scheme base) (scheme write) (scheme inexact))

(define (show x . xs) (display x) (for-each (lambda (x) (display " ") (display x)) xs) (newline))

; Promotion at the fixnum boundaries (ops, natives, and inline constants).
(show (+ 2147483647 1) (- -2147483648 1) (* 46341 46341) (* -1 -2147483648))
; expect: 2147483648 -2147483649 2147488281 2147483648
(show (- -2147483648) (abs -2147483648) (quotient -2147483648 -1) (- 0 -2147483648))
; expect: 2147483648 2147483648 2147483648 2147483648
(define (add a b) (+ a b))
(show (add 2147483647 1) (apply + (list 2147483647 2147483647)) (* 65536 65536 65536 65536))
; expect: 2147483648 4294967294 18446744073709551616
(show (let loop ((i 0) (x 1)) (if (= i 100) x (loop (+ i 1) (* x 2)))))
; expect: 1267650600228229401496703205376
; Demotion: results that fit a fixnum are fixnums again (eq? to a literal).
(define big (+ 2147483647 1))
(show (eq? (- big 1) 2147483647) (eq? (quotient (* big 4) 8) 1073741824) (eq? (- big big) 0))
; expect: #t #t #t
(show (eq? (+ (- -2147483648 1) 1) -2147483648) (exact-integer? (- big 1)) (exact-integer? big))
; expect: #t #t #t
; The 48-bit fixnum edges: + and - promote, * of two fixnums whose i64
; product overflows, and expt.
(show (+ 140737488355327 1) (- -140737488355328 1) (* 140737488355327 140737488355327))
; expect: 140737488355328 -140737488355329 19807040628565802923409276929
(show (add 140737488355327 1) (* -1 -140737488355328) (expt 2 47) (expt -2 47) (expt 3 30))
; expect: 140737488355328 140737488355328 140737488355328 -140737488355328 205891132094649
(show (eq? (- (+ 140737488355327 1) 1) 140737488355327) (eq? (* 4294967296 4) 17179869184))
; expect: #t #t
(show (let loop ((i 0) (x 2147483600)) (if (= i 200) x (loop (+ i 1) (if (< i 100) (+ x 1) (- x 1))))))
; expect: 2147483600

; Literals and printing.
(show 123456789012345678901234567890 -98765432109876543210 #x-FFFFFFFFFFFFFFFFFFFF #b1000000000000000000000000000000000)
; expect: 123456789012345678901234567890 -98765432109876543210 -1208925819614629174706175 8589934592
(write (list (expt 2 64) (- (expt 2 64)))) (newline)
; expect: (18446744073709551616 -18446744073709551616)

; Arithmetic.
(define (fact n) (if (= n 0) 1 (* n (fact (- n 1)))))
(show (fact 30) (fact 50))
; expect: 265252859812191058636308480000000 30414093201713378043612608166064768844377641568960512000000000000
(show (- (fact 25) (fact 24)) (+ (fact 20) (- (fact 20))) (* (fact 20) -1))
; expect: 14890761641597746544640000 0 -2432902008176640000
(define (fib n) (let loop ((a 0) (b 1) (i 0)) (if (= i n) a (loop b (+ a b) (+ i 1)))))
(show (fib 100) (fib 200))
; expect: 354224848179261915075 280571172992510140037611932413038677189525
(show (square (expt 10 20)) (expt -3 101) (expt 2 0) (expt 0 0) (expt (expt 10 10) 5))
; expect: 10000000000000000000000000000000000000000 -1546132562196033993109383389296863818106322566003 1 1 100000000000000000000000000000000000000000000000000
(show (expt 1 (expt 10 30)) (expt -1 (+ (expt 10 30) 1)))
; expect: 1 -1
(show (expt -1 1999999999) (expt 1 2000000000) (expt 0 2000000000) (expt -2 31) (expt 2 31))
; expect: -1 1 0 -2147483648 2147483648
(show (/ (expt 10 30) (expt 10 28)) (/ (- (expt 2 70)) (expt 2 69)))
; expect: 100 -2

; Integer division, all sign combinations.
(define n (+ (expt 10 30) 7))
(define d (+ (expt 10 12) 3))
(show (quotient n d) (remainder n d) (modulo n d))
; expect: 999999999997000000 9000007 9000007
(show (quotient (- n) d) (remainder (- n) d) (modulo (- n) d))
; expect: -999999999997000000 -9000007 999990999996
(show (quotient n (- d)) (remainder n (- d)) (modulo n (- d)))
; expect: -999999999997000000 9000007 -999990999996
(show (quotient (- n) (- d)) (remainder (- n) (- d)) (modulo (- n) (- d)))
; expect: 999999999997000000 -9000007 -9000007
(show (quotient n 7) (remainder n 7) (modulo (- n) 7) (quotient 7 n) (modulo -7 n))
; expect: 142857142857142857142857142858 1 6 0 1000000000000000000000000000000
(show (call-with-values (lambda () (floor/ (- n) d)) list)
      (call-with-values (lambda () (truncate/ (- n) d)) list))
; expect: (-999999999997000001 999990999996) (-999999999997000000 -9000007)
(show (floor-quotient (- n) d) (floor-remainder (- n) d) (truncate-quotient (- n) d) (truncate-remainder (- n) d))
; expect: -999999999997000001 999990999996 -999999999997000000 -9000007
(show (call-with-values (lambda () (floor/ -7 2)) list) (call-with-values (lambda () (truncate/ -7 2)) list))
; expect: (-4 1) (-3 -1)
(show (call-with-values (lambda () (exact-integer-sqrt (expt 10 50))) list)
      (call-with-values (lambda () (exact-integer-sqrt (+ (expt 10 50) 1))) list)
      (call-with-values (lambda () (exact-integer-sqrt 17)) list))
; expect: (10000000000000000000000000 0) (10000000000000000000000000 1) (4 1)
(show (call-with-values (lambda () (exact-integer-sqrt (- (expt 2 128) 1))) list))
; expect: (18446744073709551615 36893488147419103230)
(show (sqrt (expt 10 40)) (sqrt (* (+ (expt 2 70) 1) (+ (expt 2 70) 1))))
; expect: 100000000000000000000 1180591620717411303425
(show (gcd (expt 2 80) (expt 6 40)) (gcd (- (expt 2 70)) 0) (gcd) (gcd 0 0) (gcd 12 -18))
; expect: 1099511627776 1180591620717411303424 0 0 6
(show (lcm (expt 2 40) (expt 3 30)) (lcm) (lcm (- (expt 10 20)) 6) (lcm 0 (expt 10 20)))
; expect: 226379693794030958489370624 1 300000000000000000000 0

; Predicates and selection.
(show (zero? big) (positive? big) (negative? big) (positive? (- big)) (negative? (- big)))
; expect: #f #t #f #f #t
(show (odd? (+ (expt 2 64) 1)) (even? (expt 2 64)) (odd? (- (expt 3 41))) (even? (- (expt 3 41))))
; expect: #t #t #t #f
(show (number? big) (integer? big) (exact? big) (inexact? big) (exact-integer? (expt 3 50)))
; expect: #t #t #t #f #t
(show (max 1 (expt 2 64) 3) (min 1 (- (expt 2 64)) 3) (abs (- (expt 2 64))) (abs (expt 2 64)))
; expect: 18446744073709551616 -18446744073709551616 18446744073709551616 18446744073709551616
(show (= big big) (= big (+ big 1)) (< big (+ big 1)) (> (- big) (- -1 big)) (<= 1 big big) (>= big 1 0))
; expect: #t #f #t #t #t #t
(show (= (expt 2 100) (expt 2 100) (expt 2 100)) (< 1 (expt 2 40) (expt 2 80)) (< 1 (expt 2 80) (expt 2 40)))
; expect: #t #t #f

; Exact and inexact.
(show (exact 1e20) (exact -1e19) (exact 4294967296.0) (inexact (expt 2 40)) (inexact 4294967296))
; expect: 100000000000000000000 -10000000000000000000 4294967296 1099511627776.0 4294967296.0
(show (exact (inexact (+ (expt 2 53) 1))) (exact (inexact (+ (expt 2 53) 3))))
; expect: 9007199254740992 9007199254740996
(show (inexact? (max (expt 2 100) 1.0)) (= (max (expt 2 100) 1.0) (inexact (expt 2 100))))
; expect: #t #t
(show (exact (floor 1e18)) (round (expt 10 20)) (truncate (- (expt 10 20))) (floor big) (ceiling big))
; expect: 1000000000000000000 100000000000000000000 -100000000000000000000 2147483648 2147483648

; Comparisons with flonums are exact, not after rounding the bignum.
(define p53 (expt 2 53))
(show (= p53 9007199254740992.0) (= (+ p53 1) 9007199254740992.0) (< (+ p53 1) 9007199254740992.0) (> (+ p53 1) 9007199254740992.0))
; expect: #t #f #f #t
(show (< 9007199254740992.0 (+ p53 1)) (= 9007199254740992.0 (+ p53 1)) (>= 9007199254740992.0 (+ p53 1)))
; expect: #t #f #f
(show (< (expt 10 400) +inf.0) (> (- (expt 10 400)) -inf.0) (= (expt 10 400) +inf.0) (> (expt 10 400) 1e308))
; expect: #t #t #f #t
(show (< 1e300 (expt 10 301)) (> 1e300 (expt 10 299)) (= (expt 10 22) 1e22) (= (expt 10 23) 1e23))
; expect: #t #t #t #f
(show (< (expt 2 64) 18446744073709551616.5) (> (+ (expt 2 64) 1) 18446744073709551616.0) (= (expt 2 64) 18446744073709551616.0))
; expect: #f #t #t
(show (= (expt 2 100) +nan.0) (< (expt 2 100) +nan.0) (> (expt 2 100) +nan.0))
; expect: #f #f #f
(show (< (- (expt 2 64)) -1.5) (> (expt 2 64) -1e300) (< 2.5 big) (> 2147483648.5 big))
; expect: #t #t #t #t

; Radix input and output.
(show (number->string (expt 2 100) 2))
; expect: 10000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
(show (number->string (expt 2 100) 8) (number->string (expt 2 100) 16) (number->string (- (expt 2 70)) 16))
; expect: 2000000000000000000000000000000000 10000000000000000000000000 -400000000000000000
(show (number->string 255 16) (number->string -255 2) (number->string 0 8) (number->string (expt 10 30) 10))
; expect: ff -11111111 0 1000000000000000000000000000000
(show (string->number "-1000000000000000000000000" 16) (string->number "777" 8) (string->number "abc" 16))
; expect: -79228162514264337593543950336 511 2748
(show (string->number "#xffffffffffffffffffff") (string->number "#b101") (string->number "#o-777") (string->number "#d99"))
; expect: 1208925819614629174706175 5 -511 99
(show (string->number "100000000000000000000") (string->number "-100000000000000000000") (string->number "1z" 16))
; expect: 100000000000000000000 -100000000000000000000 #f
(show (string->number "#e1e20") (string->number "#i12345678901234") (string->number "11111111111111111111111111111111111" 2))
; expect: 100000000000000000000 12345678901234.0 34359738367

; Equivalence.
(show (eqv? (expt 2 100) (expt 2 100)) (eqv? (expt 2 100) (+ (expt 2 100) 1)) (eqv? (expt 2 100) (inexact (expt 2 100))))
; expect: #t #f #f
(show (equal? (list (expt 3 50) "x") (list (expt 3 50) "x")) (equal? (vector (expt 2 64)) (vector (expt 2 64))))
; expect: #t #t
(show (memv (expt 2 64) (list 1 (expt 2 64) 2)) (assv (expt 2 64) (list (cons (expt 2 64) 'yes))))
; expect: (18446744073709551616 2) (18446744073709551616 . yes)
(show (case (* 65536 65536) ((4294967296) 'big) (else 'other)) (member (expt 2 70) (list (expt 2 70))))
; expect: big (1180591620717411303424)

; A loop of bignum arithmetic with no call or allocation of its own
; still collects (the arithmetic ops' cold paths are safepoints).
(show (let loop ((i 0) (x 1)) (if (= i 30000) (remainder x 1000000007) (loop (+ i 1) (* x 3)))))
; expect: 852098747
