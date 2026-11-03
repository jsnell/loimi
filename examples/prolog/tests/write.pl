% write/1, writeq/1, print/1, write_canonical/1 and format/2: operators,
% spacing, quoting, lists, curly terms.

w(T) :- write(T), nl.
q(T) :- writeq(T), nl.

main :-
    w(1 + 2), w(1 + 2 * 3), w((1 + 2) * 3), w(1 - (2 - 3)), w(1 - 2 - 3),
    w(2 ** 3), w(- 1), w(-(1)), w(-(-(1))), w(- a), w(-(-(a))), w(1 - -1), w(1 + -2),
    w(a = b), w(a :- b), w((a :- b, c)), w((a, b)), w(f((a, b))), w(f(a :- b)),
    w((a ; b)), w((a -> b ; c)), w(\+ a), w(\+ (a, b)), w(f(-)), w(f(+, -)),
    w([a, b, c]), w([a | b]), w([[1, 2], [3]]), w('[]'), w([]), w({a, b}), w('{}'(x)),
    w(f(x, y) = g), w(a:b:c), w(- (1 + 2)), w(-(3) + 4), w(3 - (-3)),
    w(1 mod 2), w(a is b), w(f(a mod b)), w(- - - a), w(\ a), w(a rem b rem c),
    w(1.0), w(-0.0), w(3.25e100), w(1.0e-10),
    q(hello), q('Hello'), q('hello world'), q([]), q(''), q('it''s'), q(a+'B'),
    q(f('A', b, c)), q('\n'), q(+), q(f(+)), q(- (1)), q(-(-(1))), q([a, 'B' | c]),
    q('/*'), q(;), q('!'), q(','), q('|'), q(f(',', '|')), q({x}), q(-(a)), q(1 - a),
    q(f(;, '|', [])),
    print(foo('X', y)), nl,
    write_canonical(f(x, 'A', [1, 2], 1 + 2)), nl,
    write_term(f('A', 1 + 2), [quoted(true)]), nl,
    write_term([1, 2], [ignore_ops(true)]), nl,
    format("~w and ~q~n", [hello, 'World']),
    format("~a|~d|~2d|~s|~c~n", [abc, 42, 314, "xyz", 65]),
    format("~e ~4f ~g~n", [1.5, 2.0, 3.0]),
    format("~p~n", ['q q']),
    format("no args~n"),
    format("~w~30|~w~n", [left, right]),
    format("~a~t~20|~a~n", [name, value]),
    format("~t~w~10|~n", [right]),
    format("~D~n", [1234567]),
    format("~8|abc~n", []),
    format("~~ tilde~n", []),
    format("~w", hello), nl,
    atom_to_term('foo(X, Y, X)', T, B), length(B, NB), write(NB), nl, T = foo(1, 2, Z), write(Z), nl,
    term_to_atom(f(x, 'Y'), At), write(At), nl,
    X = 1, write(X), nl.
:- initialization(main).
