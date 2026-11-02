; deriv: symbolic differentiation (Gabriel benchmarks "DERIV", r7rs-benchmarks "deriv").
(define (deriv a)
  (cond ((not (pair? a))
         (if (eq? a 'x) 1 0))
        ((eq? (car a) '+)
         (cons '+ (map deriv (cdr a))))
        ((eq? (car a) '-)
         (cons '- (map deriv (cdr a))))
        ((eq? (car a) '*)
         (list '*
               a
               (cons '+ (map (lambda (a) (list '/ (deriv a) a)) (cdr a)))))
        ((eq? (car a) '/)
         (list '-
               (list '/ (deriv (cadr a)) (caddr a))
               (list '/
                     (cadr a)
                     (list '* (caddr a) (caddr a) (deriv (caddr a))))))
        (else
         (error "No derivation method available" (car a)))))

(define (run n)
  (let loop ((i n) (r '()))
    (if (= i 0) r (loop (- i 1) (deriv '(+ (* 3 x x) (* a x x) (* b x) 5))))))

(display (run 400000))
(newline)
; expect: (+ (* (* 3 x x) (+ (/ 0 3) (/ 1 x) (/ 1 x))) (* (* a x x) (+ (/ 0 a) (/ 1 x) (/ 1 x))) (* (* b x) (+ (/ 0 b) (/ 1 x))) 0)
