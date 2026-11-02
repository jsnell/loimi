; Unsupported forms are reported, not silently miscompiled.
(display 1)
(dynamic-wind (lambda () 0) (lambda () 1) (lambda () 2))
; expect-exit: 1
; expect-stderr: error: line 3: dynamic-wind is not supported
