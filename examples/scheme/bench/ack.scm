; ack: Ackermann's function (r7rs-benchmarks "ack"). Deep non-tail recursion.
(define (ack m n)
  (cond ((= m 0) (+ n 1))
        ((= n 0) (ack (- m 1) 1))
        (else (ack (- m 1) (ack m (- n 1))))))

(define (run n)
  (let loop ((i n) (r 0))
    (if (= i 0) r (loop (- i 1) (ack 3 9)))))

(display (run 20))
(newline)
(display (ack 3 10))
(newline)
; expect: 4093
; expect: 8189
