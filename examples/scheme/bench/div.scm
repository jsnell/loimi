; div: dividing lists by 2, iteratively and recursively (Gabriel benchmarks
; "DIV", r7rs-benchmarks "diviter" and "divrec").
(define (create-n n)
  (do ((n n (- n 1))
       (a '() (cons '() a)))
      ((= n 0) a)))

(define *ll* (create-n 200))

(define (iterative-div2 l)
  (do ((l l (cddr l))
       (a '() (cons (car l) a)))
      ((null? l) a)))

(define (recursive-div2 l)
  (cond ((null? l) '())
        (else (cons (car l) (recursive-div2 (cddr l))))))

(define (run n)
  (let loop ((i n) (r 0))
    (if (= i 0)
        r
        (loop (- i 1)
              (+ (length (iterative-div2 *ll*))
                 (length (recursive-div2 *ll*)))))))

(display (run 100000))
(newline)
; expect: 200
