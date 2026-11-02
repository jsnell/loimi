; (error msg irritant ...) prints the message and irritants.
(display "partial output is flushed")
(newline)
; expect: partial output is flushed
(error "Something bad:" 42 (list 1 "two"))
; expect-exit: 1
; expect-stderr: error: line 5: Something bad: 42 (1 "two")
