; Unsupported forms are reported, not silently miscompiled.
(display 1)
(call/cc (lambda (k) (k 1)))
; expect-exit: 1
; expect-stderr: error: line 3: call/cc is not supported
