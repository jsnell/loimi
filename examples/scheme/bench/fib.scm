; fib: doubly recursive Fibonacci (r7rs-benchmarks "fib").
(define (fib n)
  (if (< n 2)
      n
      (+ (fib (- n 1)) (fib (- n 2)))))

(display (fib 32))
(newline)
; expect: 2178309
