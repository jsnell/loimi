; A known top-level procedure calling itself in tail position compiles to
; a jump to its start. It must behave exactly like the call.

; Parameters that swap, and a long loop (the frame stack holds 131072 frames).
(define (swap a b n) (if (= n 0) (list a b) (swap b a (- n 1))))
(display (list (swap 1 2 3) (swap 1 2 1000001)))
(newline)
; expect: ((2 1) (2 1))
; Each iteration gets a fresh binding for a captured, assigned parameter.
(define (collect i acc)
  (if (= i 3)
      (map (lambda (p) (p)) acc)
      (collect (+ i 1) (cons (lambda () (set! i (+ i 100)) i) acc))))
(display (collect 0 '()))
(newline)
; expect: (102 101 100)
; A procedure that is set! later is called through the global.
(define (hop n) (if (= n 0) 'original (hop (- n 1))))
(define saved hop)
(set! hop (lambda (n) 'replaced))
(display (list (saved 3) (hop 3)))
(newline)
; expect: (replaced replaced)
; A procedure defined twice: the call goes through the global.
(define (twice n) (if (= n 0) 'first (twice (- n 1))))
(define first-twice twice)
(define (twice n) 'second)
(display (first-twice 2))
(newline)
; expect: second
; Wrong argument counts still fail as calls do.
(define (two a b) (if (= a 0) b (two (- a 1))))
(two 1 2)
; expect-exit: 1
; expect-stderr: two: wrong number of arguments (expected 2, got 1)
