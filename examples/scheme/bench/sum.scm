; sum: a tight counting loop (r7rs-benchmarks "sum").
(define (sum n)
  (let loop ((i n) (sum 0))
    (if (< i 0)
        sum
        (loop (- i 1) (+ i sum)))))

(define (run n)
  (let loop ((i n) (r 0))
    (if (= i 0) r (loop (- i 1) (sum 10000)))))

(display (run 10000))
(newline)
; expect: 50005000
