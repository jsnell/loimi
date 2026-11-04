; Builtins compiled as ops with a fast path (length, odd?, even?,
; positive?, negative?, vector-length, reverse, append of two lists,
; record accessors): every other case is the native's.
(display (list (length (list 1 2 3)) (length '()) (odd? 3) (odd? -3) (even? 4)
               (odd? 3.0) (even? 100000000000000000000) (positive? 2)
               (positive? -2.5) (negative? -1) (negative? 0) (vector-length (vector 1 2))))
(newline)
; expect: (3 0 #t #t #t #t #t #t #f #t #f 2)
(define (f g) (map g (list 1 2 3)))
(display (list (f odd?) (f even?)))
(newline)
; expect: ((#t #f #t) (#f #t #f))
(display (list (reverse (list 1 2 3)) (reverse '()) (append (list 1 2) (list 3))
               (append '() 5) (append (list 1) 5) (append (list 1 2) '())))
(newline)
; expect: ((3 2 1) () (1 2 3) 5 (1 . 5) (1 2))
; Long lists run out of the allocator's current run of free memory.
(define (iota n) (let lp ((i 0) (l '())) (if (= i n) l (lp (+ i 1) (cons i l)))))
(define x (iota 100000))
(display (list (length (reverse x)) (length (append x x)) (car (append x x)) (car (reverse x))))
(newline)
; expect: (100000 200000 99999 0)
; append's result shares its last argument.
(define tail (list 7 8))
(display (eq? (cddr (append (list 1 2) tail)) tail))
(newline)
; expect: #t
(define (g h) (h (list 1 2)))
(display (g reverse))
(newline)
; expect: (2 1)
(display (call-with-current-continuation (lambda (k) (+ 1 (k 41)))))
(newline)
; expect: 41
(append (cons 1 2) (list 3))
; expect-exit: 1
; expect-stderr: error: line 35: append: expected list, got (1 . 2)
