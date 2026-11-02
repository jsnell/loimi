; Strings, characters, symbols, vectors and output.

(define (show x . xs) (display x) (for-each (lambda (x) (display " ") (display x)) xs) (newline))

(define s "hello")
(show (string-length s) (string-ref s 1) (substring s 1 3) (substring s 2))
; expect: 5 e el llo
(show (string-append "foo" "bar" "" "baz") (string-length (string-append)))
; expect: foobarbaz 0
(show (string=? "ab" "ab") (string=? "ab" "abc") (string<? "abc" "abd") (string<? "ab" "a") (string>? "b" "a"))
; expect: #t #f #t #f #t
(write (list "quote\"d" "back\\slash" "tab\there" #\a #\space #\newline #\x41 #\()) (newline)
; expect: ("quote\"d" "back\\slash" "tab\there" #\a #\space #\newline #\A #\()
(display (list "plain" #\b)) (newline)
; expect: (plain b)
(show (char->integer #\a) (integer->char 66) (char-upcase #\a) (char-downcase #\A))
; expect: 97 B A a
(show (char-alphabetic? #\a) (char-numeric? #\5) (char-whitespace? #\space) (char=? #\a #\a) (char<? #\a #\b))
; expect: #t #t #t #t #t
(show (symbol->string 'abc) (string->symbol "xyz") (eq? 'abc (string->symbol "abc")))
; expect: abc xyz #t
(show (symbol? 'a) (symbol? "a") (string? "a") (char? #\a) (boolean? #f) (boolean? '()))
; expect: #t #f #t #t #t #f
(show (number->string 255) (string->list "abc") (list->string (list #\x #\y)))
; expect: 255 (a b c) xy
(define ms (make-string 3 #\z))
(string-set! ms 1 #\-)
(show ms (string-copy "copy") (string #\a #\b))
; expect: z-z copy ab
(show "multi\
       line" "esc\x41;")
; expect: multiline escA
; Vectors.
(define v (vector 1 'two "three" #\4))
(write v) (newline)
; expect: #(1 two "three" #\4)
(show (vector-length v) (vector-ref v 1) (vector? v) (vector? '(1)))
; expect: 4 two #t #f
(vector-set! v 0 'one)
(show (vector->list v) (list->vector '(1 2)) (make-vector 2 'x) #(1 #(2)))
; expect: (one two three 4) #(1 2) #(x x) #(1 #(2))
(define v2 (make-vector 3 0))
(vector-fill! v2 7)
(show v2 (vector-map (lambda (x) (* x 2)) #(1 2 3)))
; expect: #(7 7 7) #(2 4 6)
(vector-for-each (lambda (x) (display x)) #(a b c)) (newline)
; expect: abc
; write vs display, and special values.
(write 'sym) (write "str") (write #\c) (write 1.5) (write '()) (write #t) (newline)
; expect: sym"str"#\c1.5()#t
(write (if #f #f)) (newline)
; expect:
(write (list car (lambda (x) x) show)) (newline)
; expect: (#<procedure> #<procedure lambda> #<procedure show>)
(write-string "ws") (write-char #\!) (newline)
; expect: ws!
