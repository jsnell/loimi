; define-record-type, at top level and in a body.
(define-record-type point (make-point x y) point? (x point-x set-point-x!) (y point-y))
(define p (make-point 1 2))
(display (list (point? p) (point? 5) (vector? p) (point-x p) (point-y p)))
(newline)
(set-point-x! p 10)
(display (point-x p))
(newline)
(write p)
(newline)
(define (f)
  (define-record-type node (make-node val) node? (val node-val) (next node-next set-node-next!))
  (let ((n (make-node 7)))
    (set-node-next! n 'end)
    (list (node-val n) (node-next n) (node? p))))
(display (f))
(newline)
(display (equal? (make-point 1 2) (make-point 1 2)))
(newline)
(point-x 5)   ; reported at the line of the definition (the accessor's code)
; expect: (#t #f #f 1 2)
; expect: 10
; expect: #<record point>
; expect: (7 end #f)
; expect: #f
; expect-exit: 1
; expect-stderr: error: line 2: record accessor: expected a record of type point, got 5
