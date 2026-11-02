; tak: Takeuchi function (Gabriel benchmarks "TAK", r7rs-benchmarks "tak").
(define (tak x y z)
  (if (not (< y x))
      z
      (tak (tak (- x 1) y z)
           (tak (- y 1) z x)
           (tak (- z 1) x y))))

(define (run n)
  (let loop ((i n) (r 0))
    (if (= i 0) r (loop (- i 1) (tak 18 12 6)))))

(display (run 400))
(newline)
(display (tak 32 16 8))
(newline)
; expect: 7
; expect: 9
