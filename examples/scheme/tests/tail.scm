; Proper tail calls (they must not grow the frame stack, which holds 131072
; frames: every loop below runs longer than that) and deep
; non-tail recursion.

(define (count-down n) (if (= n 0) 'done (count-down (- n 1))))
(display (count-down 3000000)) (newline)
; expect: done
; Mutual recursion through globals.
(define (my-even? n) (if (= n 0) #t (my-odd? (- n 1))))
(define (my-odd? n) (if (= n 0) #f (my-even? (- n 1))))
(display (my-even? 2000001)) (newline)
; expect: #f
; Tail calls in cond, and, or, when, case, let, begin.
(define (t-cond n) (cond ((= n 0) 'c) (else (t-cond (- n 1)))))
(define (t-and n) (and #t (if (= n 0) 'a (t-and (- n 1)))))
(define (t-or n) (or #f (if (= n 0) 'o (t-or (- n 1)))))
(define (t-when n) (when #t (if (= n 0) 'w (t-when (- n 1)))))
(define (t-case n) (case (if (= n 0) 0 1) ((0) 'k) (else (t-case (- n 1)))))
(define (t-let n) (let ((m (- n 1))) (if (< m 0) 'l (t-let m))))
(define (t-begin n) (begin 1 (if (= n 0) 'b (t-begin (- n 1)))))
(define (t-cond=> n) (cond ((= n 0) 'arrow) ((- n 1) => t-cond=>)))
(display (list (t-cond 1000000) (t-and 1000000) (t-or 1000000) (t-when 1000000)
               (t-case 1000000) (t-let 1000000) (t-begin 1000000) (t-cond=> 1000000)))
(newline)
; expect: (c a o w k l b arrow)
; Tail calls through apply and through a builtin in tail position.
(define (t-apply n) (if (= n 0) 'ap (apply t-apply (list (- n 1)))))
(display (t-apply 150000)) (newline)
; expect: ap
; Tail calls with rest arguments.
(define (t-rest n . xs) (if (= n 0) xs (t-rest (- n 1) n)))
(display (t-rest 150000)) (newline)
; expect: (1)
; A named-let loop with a closure created on each iteration.
(define (loop-closures n)
  (let loop ((i 0) (f (lambda () 0)))
    (if (= i n) (f) (loop (+ i 1) (lambda () i)))))
(display (loop-closures 150000)) (newline)
; expect: 149999
; Deep non-tail recursion (100000 deep).
(define (sum-to n) (if (= n 0) 0 (+ n (sum-to (- n 1)))))
(display (sum-to 100000)) (newline)
; expect: 5000050000.0
(define (build n) (if (= n 0) '() (cons n (build (- n 1)))))
(display (length (build 10000))) (newline)
; expect: 10000
