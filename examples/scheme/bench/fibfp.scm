; fibfp: Fibonacci on flonums (r7rs-benchmarks "fibfp").
(define (fibfp n)
  (if (< n 2.)
      n
      (+ (fibfp (- n 1.)) (fibfp (- n 2.)))))

(display (fibfp 30.))
(newline)
; expect: 832040.0
