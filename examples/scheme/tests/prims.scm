; Builtins called directly (PRIMCALL) and type predicates compiled to ops.

(define (types x) (list (symbol? x) (number? x) (exact-integer? x) (string? x) (vector? x)))
(display (map types (list 'a 1 1.5 "s" (vector 1) '() #f)))
(newline)
; expect: ((#t #f #f #f #f) (#f #t #t #f #f) (#f #t #f #f #f) (#f #f #f #t #f) (#f #f #f #f #t) (#f #f #f #f #f) (#f #f #f #f #f))
; A record is not a vector.
(define-record-type point (make-point x y) point? (x point-x) (y point-y))
(display (list (vector? (make-point 1 2)) (point? (make-point 1 2)) (point-x (make-point 3 4))))
(newline)
; expect: (#f #t 3)
; Direct builtin calls: values, tail position, a builtin as a value.
(define (rev-tail l) (reverse l))
(display (list (assq 'b '((a 1) (b 2))) (memq 'c '(a b c d)) (length '(1 2 3)) (rev-tail '(1 2 3))
               (append '(1) '(2) '(3)) (apply + 1 2 '(3 4)) (map car '((1) (2)))))
(newline)
; expect: ((b 2) (c d) 3 (3 2 1) (1 2 3) 10 (1 2))
; A builtin in tail position of a long loop does not grow the frame stack.
(define (spin n) (if (= n 0) (list 'spun) (begin (length '(1 2)) (spin (- n 1)))))
(display (spin 1000000))
(newline)
; expect: (spun)
; A program that redefines a builtin gets its own definition everywhere.
(define (length l) 'mine)
(display (list (length '(1 2 3)) (rev-tail '(4 5))))
(newline)
; expect: (mine (5 4))
; Errors from directly called builtins report the line.
(define (f x) (vector-length x))
(f 'nope)
; expect-exit: 1
; expect-stderr: error: line 29: vector-length: expected vector, got nope
