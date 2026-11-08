; Ahead-of-time traces (docs/design/trace-tier.md section 13.7): loops
; that the scheme_traces test records, profiles and traces. Each must
; print the same with traces, with guards that fail and with blacklisting
; at every failure.

; a named let over fixnums that turn into flonums halfway (the trace's
; arithmetic guards fail in the second half)
(define (mixed n)
  (let loop ((i 0) (acc 0))
    (if (= i n)
        acc
        (loop (+ i 1) (if (< i (quotient n 2)) (+ acc i) (+ acc (* i 0.5)))))))
(display (mixed 4000))
(newline)
; expect: 4998500.0

; nested loops over a vector, with set! of a captured variable
(define (sieve n)
  (let ((v (make-vector (+ n 1) #t)) (count 0))
    (let outer ((i 2))
      (when (<= i n)
        (when (vector-ref v i)
          (set! count (+ count 1))
          (let inner ((j (* i i)))
            (when (<= j n)
              (vector-set! v j #f)
              (inner (+ j i)))))
        (outer (+ i 1))))
    count))
(display (sieve 30000))
(newline)
; expect: 3245

; a loop over a list whose elements change type, and a call of a small
; procedure (inlined into the trace) that allocates
(define (pair-up x) (cons x x))
(define (walk l)
  (let loop ((l l) (n 0) (s 0))
    (cond ((null? l) (list n s))
          ((number? (car l)) (loop (cdr l) (+ n 1) (+ s (car (pair-up (car l))))))
          (else (loop (cdr l) n s)))))
(define big (let mk ((i 0) (acc '()))
              (if (= i 3000) acc (mk (+ i 1) (cons (if (= (remainder i 7) 0) 'sym i) acc)))))
(display (walk big))
(newline)
; expect: (2571 3855858)

; recursion (CALLSELF: a jump to the same code with a frame of its own is
; not a path inside a loop) next to a self tail call (a loop)
(define (fib n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
(define (count-down n acc) (if (= n 0) acc (count-down (- n 1) (+ acc (fib 5)))))
(display (list (fib 20) (count-down 3000 0)))
(newline)
; expect: (6765 15000)

; a dispatch loop with a case: side traces for its arms
(define (run prog steps)
  (let loop ((pc 0) (acc 0) (k 0))
    (if (= k steps)
        acc
        (let ((op (vector-ref prog pc)))
          (loop (remainder (+ pc 1) (vector-length prog))
                (case op
                  ((0) (+ acc 1))
                  ((1) (* acc 2))
                  ((2) (remainder acc 1000))
                  (else (- acc 3)))
                (+ k 1))))))
(display (run (vector 0 0 1 3 2 0 1 2 3 0 0 1) 20000))
(newline)
; expect: 180

; a string built in a loop (allocation and collections inside the trace)
(define (build n)
  (let loop ((i 0) (acc '()))
    (if (= i n)
        (string-length (apply string-append acc))
        (loop (+ i 1) (cons (number->string i) acc)))))
(display (build 3000))
(newline)
; expect: 10890

; floats in a tight loop (a guard on the flonum tag)
(define (integrate n)
  (let loop ((i 0) (x 0.0) (s 0.0))
    (if (= i n) s (loop (+ i 1) (+ x 0.001) (+ s (* x x 0.001))))))
(display (round (* 1000 (integrate 3000))))
(newline)
; expect: 8996.0
