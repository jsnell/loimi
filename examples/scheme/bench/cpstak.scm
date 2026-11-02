; cpstak: TAK in continuation-passing style (r7rs-benchmarks "cpstak").
; Every call is a tail call; the continuations are closures.
(define (cpstak x y z)
  (define (tak x y z k)
    (if (not (< y x))
        (k z)
        (tak (- x 1)
             y
             z
             (lambda (v1)
               (tak (- y 1)
                    z
                    x
                    (lambda (v2)
                      (tak (- z 1)
                           x
                           y
                           (lambda (v3)
                             (tak v1 v2 v3 k)))))))))
  (tak x y z (lambda (a) a)))

(define (run n)
  (let loop ((i n) (r 0))
    (if (= i 0) r (loop (- i 1) (cpstak 18 12 6)))))

(display (run 300))
(newline)
; expect: 7
