; Contification: letrec / named-let procedures whose every use is a tail
; call from the same frame are compiled inline as jumps. These check that
; the result is delivered to the right continuation, that tail calls stay
; tail calls, and that set! and closures see the right variables.

; A named let in non-tail position (its value goes to a register).
(define (sum-to n) (+ 1000 (let loop ((i 0) (acc 0)) (if (> i n) acc (loop (+ i 1) (+ acc i))))))
(display (sum-to 10)) (newline)
; expect: 1055
; In effect position, with a side effect each iteration.
(define counter 0)
(define (bump n) (let loop ((i 0)) (when (< i n) (set! counter (+ counter 1)) (loop (+ i 1)))) 'done)
(display (list (bump 5) counter)) (newline)
; expect: (done 5)
; As an argument, in a condition, in and/or.
(define (f n) (list (let l ((i n)) (if (= i 0) 'z (l (- i 1))))
                    (if (let l ((i n)) (if (= i 0) #f (l (- i 1)))) 'yes 'no)
                    (and #t (let l ((i n)) (if (= i 0) 'and-end (l (- i 1)))))
                    (or (let l ((i n)) (if (= i 0) #f (l (- i 1)))) 'or-end)))
(display (f 3)) (newline)
; expect: (z no and-end or-end)
; Loops nested in loops, the inner one in non-tail position.
(define (table n)
  (let rows ((i 1) (acc '()))
    (if (> i n)
        (reverse acc)
        (rows (+ i 1)
              (cons (let cols ((j 1) (row '())) (if (> j i) (reverse row) (cols (+ j 1) (cons (* i j) row))))
                    acc)))))
(display (table 3)) (newline)
; expect: ((1) (2 4) (3 6 9))
; Mutually tail-recursive local procedures (a state machine), long enough
; that anything but jumps would overflow the frame stack.
(define (parity n)
  (define (ev k) (if (= k 0) 'even (od (- k 1))))
  (define (od k) (if (= k 0) 'odd (ev (- k 1))))
  (ev n))
(display (list (parity 1000001) (parity 1000000))) (newline)
; expect: (odd even)
; The same in non-tail position.
(define (parity2 n)
  (define (ev k) (if (= k 0) 'even (od (- k 1))))
  (define (od k) (if (= k 0) 'odd (ev (- k 1))))
  (list 'p (ev n)))
(display (parity2 300001)) (newline)
; expect: (p odd)
; A tail call out of a contified body to another procedure stays a tail
; call (3000000 frames would overflow the frame stack).
(define (down n) (if (= n 0) 'down-done (let l ((k n)) (if (> k 0) (down (- k 1)) (l k)))))
(display (down 3000000)) (newline)
; expect: down-done
; A contified loop inside a procedure that loops on itself.
(define (outer n acc)
  (if (= n 0)
      acc
      (let inner ((k 3) (a acc)) (if (= k 0) (outer (- n 1) a) (inner (- k 1) (+ a 1))))))
(display (outer 1000000 0)) (newline)
; expect: 3000000
; Closures made in each iteration capture that iteration's variables.
(define (thunks n)
  (let loop ((i 0) (acc '()))
    (if (= i n) (map (lambda (t) (t)) acc) (loop (+ i 1) (cons (lambda () i) acc)))))
(display (thunks 4)) (newline)
; expect: (3 2 1 0)
; ... also when the closure assigns the loop variable (a fresh box each time).
(define (boxes n)
  (let loop ((i 0) (acc '()))
    (if (= i n)
        (map (lambda (p) (p)) acc)
        (loop (+ i 1) (cons (lambda () (set! i (* i 10)) i) acc)))))
(display (boxes 3)) (newline)
; expect: (20 10 0)
; set! of a loop variable inside the loop, and of a variable of the
; enclosing procedure.
(define (setter n)
  (let ((total 0))
    (let loop ((i 0))
      (when (< i n)
        (set! total (+ total i))
        (set! i (+ i 1))
        (loop i)))
    total))
(display (setter 5)) (newline)
; expect: 10
; Arguments that swap parameters.
(define (swap n)
  (let loop ((a 1) (b 2) (k n)) (if (= k 0) (list a b) (loop b a (- k 1)))))
(display (list (swap 3) (swap 4))) (newline)
; expect: ((2 1) (1 2))
; A loop used as a value is a closure (not contified), and still works.
(define (escape n)
  (let loop ((i 0)) (if (< i n) (loop (+ i 1)) loop)))
(display (procedure? (escape 3))) (newline)
; expect: #t
; A loop called in non-tail position from its own body (recursion) is a closure.
(define (depth l) (let walk ((x l)) (if (pair? x) (+ 1 (walk (cdr x))) 0)))
(display (depth '(1 2 3 4))) (newline)
; expect: 4
; A loop called from a closure inside it is a closure.
(define (via-closure n)
  (let loop ((i n)) (if (= i 0) 'vc ((lambda () (loop (- i 1)))))))
(display (via-closure 5)) (newline)
; expect: vc
; Loops in a top-level expression, and the empty case.
(display (let loop ((i 0) (s '())) (if (= i 3) s (loop (+ i 1) (cons i s))))) (newline)
; expect: (2 1 0)
(display (do ((i 0 (+ i 1))) ((= i 0) 'none))) (newline)
; expect: none
; Errors inside a contified body report its line.
(define (bad n) (let loop ((i n)) (if (= i 0) (car i) (loop (- i 1)))))
(bad 2)
; expect-exit: 1
; expect-stderr: error: line 110: car: expected pair, got 0
