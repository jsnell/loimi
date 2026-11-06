% Division by integer constants (the IDIVK, MODK, DIVK and REMK ops,
% which multiply by a std.math.Divisor), against SWI-Prolog: //, mod, div
% and rem by positive constants of every size, negative and zero
% constants (the generic ops), and dividends of both signs, the i48
% limits, and bignums.

d1(X) :- A is X // 1, B is X mod 1, C is X div 1, D is X rem 1, w([A, B, C, D]).
d3(X) :- A is X // 3, B is X mod 3, C is X div 3, D is X rem 3, w([A, B, C, D]).
d7(X) :- A is X // 7, B is X mod 7, C is X div 7, D is X rem 7, w([A, B, C, D]).
d10(X) :- A is X // 10, B is X mod 10, C is X div 10, D is X rem 10, w([A, B, C, D]).
d16(X) :- A is X // 16, B is X mod 16, C is X div 16, D is X rem 16, w([A, B, C, D]).
d127(X) :- A is X // 127, B is X mod 127, C is X div 127, D is X rem 127, w([A, B, C, D]).
d1000(X) :- A is (X * 3 + 1) // 1000, B is (X * 3 + 1) mod 1000, C is X div 1000, D is X rem 1000, w([A, B, C, D]).
d65537(X) :- A is X // 65537, B is X mod 65537, C is X div 65537, D is X rem 65537, w([A, B, C, D]).
dbig(X) :- A is X // 2147483647, B is X mod 2147483647, C is X div 2147483648, D is X rem 2147483648, w([A, B, C, D]).
dneg(X) :- A is X // -3, B is X mod -3, C is X div -7, D is X rem -10, w([A, B, C, D]).
dzero(X) :- catch((_ is X mod 0), error(E, _), (write(E), nl)).

w(L) :- write(L), nl.

xs([0, 1, -1, 2, -2, 3, -3, 6, -6, 7, -7, 9, -9, 10, -10, 11, -11, 15, -15, 16, -16, 17, -17,
    99, -99, 100, -101, 126, -127, 128, -128, 999, -1000, 65536, -65537, 65538,
    2147483646, 2147483647, -2147483648, 2147483648, -2147483649, 4294967295, 4294967296,
    140737488355327, -140737488355328, 140737488355328, -140737488355329,
    123456789012345, -98765432109876, 1180591620717411303424, -1180591620717411303425]).

t(X) :- d1(X), d3(X), d7(X), d10(X), d16(X), d127(X), d1000(X), d65537(X), dbig(X), dneg(X).

main :-
    xs(L), forall(member(X, L), t(X)),
    dzero(5),
    Y = 12345, Z is Y mod 10 + Y // 10 * 10, write(Z), nl,
    ( catch((_ is foo mod 10), error(E, _), (write(E), nl)) -> true ; true ).
:- initialization(main).
