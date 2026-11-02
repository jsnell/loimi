; Special forms: if, define, let family, cond, case, and, or, when, unless, do, begin.

(define (show x . xs) (display x) (for-each (lambda (x) (display " ") (display x)) xs) (newline))

(show (if #t 1 2) (if #f 1 2) (if '() 'empty-is-true 'no) (if 0 'zero-is-true 'no))
; expect: 1 2 empty-is-true zero-is-true
(define v 5)
(define (sq x) (* x x))
(define sq2 (lambda (x) (* x x)))
(show v (sq 4) (sq2 5))
; expect: 5 16 25
(show (let ((a 1) (b 2)) (+ a b)) (let () 7))
; expect: 3 7
(define a 100)
(show (let ((a 1) (b a)) b) (let* ((a 1) (b (+ a 1))) b))
; expect: 100 2
(show (letrec ((ev? (lambda (n) (if (= n 0) #t (od? (- n 1))))) (od? (lambda (n) (if (= n 0) #f (ev? (- n 1)))))) (ev? 10)))
; expect: #t
(show (letrec* ((x 1) (y (+ x 1))) (list x y)))
; expect: (1 2)
(show (let loop ((i 0) (acc '())) (if (= i 5) (reverse acc) (loop (+ i 1) (cons i acc)))))
; expect: (0 1 2 3 4)
(define (classify n)
  (cond ((< n 0) 'negative)
        ((= n 0) 'zero)
        ((< n 10) 'small)
        (else 'large)))
(show (classify -5) (classify 0) (classify 3) (classify 42))
; expect: negative zero small large
(show (cond ((assv 'b '((a 1) (b 2))) => cadr) (else 'none)) (cond (#f 1)) (cond (5)))
; expect: 2  5
(define (kind x)
  (case x
    ((1 2 3) 'small)
    ((a b) 'letter)
    (("s") 'never-eqv)
    (else 'other)))
(show (kind 2) (kind 'b) (kind "s") (kind 99))
; expect: small letter other other
(show (case 5 ((5) => (lambda (x) (* x 10))) (else 0)) (case 'z ((a) 1) (else => (lambda (x) x))))
; expect: 50 z
(show (and) (and 1 2) (and 1 #f 3) (or) (or #f 2) (or #f #f))
; expect: #t 2 #f #f 2 #f
(show (when (> 1 0) 'a 'b) (unless (> 1 0) 'a 'b) (unless #f 'c))
; expect: b  c
(show (do ((i 0 (+ i 1)) (acc '() (cons i acc))) ((= i 4) acc)))
; expect: (3 2 1 0)
(define vec (make-vector 5 0))
(do ((i 0 (+ i 1))) ((= i 5)) (vector-set! vec i (* i i)))
(show vec)
; expect: #(0 1 4 9 16)
(show (begin 1 2 3))
; expect: 3
(begin (define b1 1) (define b2 2))
(show (+ b1 b2))
; expect: 3
(define (internal x)
  (define y (* x 2))
  (define (helper z) (+ y z))
  (helper 1))
(show (internal 5))
; expect: 11
(define (internal2)
  (define r '())
  (set! r (cons 1 r))
  (define s (cons 2 r))
  s)
(show (internal2))
; expect: (2 1)
; Shadowing: special-form names and builtins can be rebound locally.
(define (shadow list if) (list if))
(show (shadow (lambda (x) (* x 3)) 4))
; expect: 12
(show (let ((car cdr)) (car '(1 2 3))))
; expect: (2 3)
; Nested quasiquote and vectors.
(define n 3)
(show `(a ,n ,@(list 4 5) . end) `#(1 ,n) `(x `(y ,(z ,n))))
; expect: (a 3 4 5 . end) #(1 3) (x `(y ,(z 3)))
(show `(1 ,@'() 2) `,n `(,@(list 1 2)))
; expect: (1 2) 3 (1 2)
; Comments
#| block
   #| nested |# comment |#
(show 'after #;(ignored datum) 'comments) ; trailing
; expect: after comments
(show '[square brackets] #t #f #true #false)
; expect: (square brackets) #t #f #t #f
