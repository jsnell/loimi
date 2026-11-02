; Allocation-heavy code that must survive collections (run with GC stress
; by the test harness): trees, closures, boxes, vectors, strings, rest lists.

(define (make-tree d)
  (if (= d 0)
      (cons '() '())
      (cons (make-tree (- d 1)) (make-tree (- d 1)))))
(define (check t) (if (null? (car t)) 1 (+ 1 (check (car t)) (check (cdr t)))))
(define keep (make-tree 6))
(define (churn n acc)
  (if (= n 0) acc (churn (- n 1) (+ acc (check (make-tree 4))))))
(display (churn 20 0)) (newline)
; expect: 620
(display (check keep)) (newline)
; expect: 127
; Closures and boxes survive.
(define (make-gen)
  (let ((n 0) (hist '()))
    (lambda ()
      (set! n (+ n 1))
      (set! hist (cons (number->string n) hist))
      hist)))
(define g (make-gen))
(define (run k) (if (> k 0) (begin (g) (make-vector 10 k) (run (- k 1)))))
(run 50)
(display (length (g))) (display " ") (display (car (g))) (newline)
; expect: 51 52
; Strings built in a loop.
(define (stars n) (if (= n 0) "" (string-append "*" (stars (- n 1)))))
(display (stars 30)) (newline)
; expect: ******************************
; Vectors of lists.
(define vv (make-vector 20 '()))
(do ((i 0 (+ i 1))) ((= i 200))
  (let ((j (modulo i 20)))
    (vector-set! vv j (cons i (vector-ref vv j)))))
(display (vector-ref vv 7)) (newline)
; expect: (187 167 147 127 107 87 67 47 27 7)
; Rest lists and apply under allocation.
(define (sum . xs) (if (null? xs) 0 (+ (car xs) (apply sum (cdr xs)))))
(display (sum 1 2 3 4 5 6 7 8 9 10)) (newline)
; expect: 55
(define (assoc-build n) (if (= n 0) '() (cons (cons n (list->string (list #\k))) (assoc-build (- n 1)))))
(display (assv 3 (assoc-build 10))) (newline)
; expect: (3 . k)
(display (map (lambda (x) (list x (* x x))) '(1 2 3))) (newline)
; expect: ((1 1) (2 4) (3 9))
(display (let loop ((i 0) (acc '())) (if (= i 1000) (length acc) (loop (+ i 1) (cons (vector i) acc))))) (newline)
; expect: 1000
