% Integers beyond 48 bits: arithmetic, comparison, unification, output,
% reading, and the switch back to small integers.

fact(0, 1) :- !.
fact(N, F) :- N1 is N - 1, fact(N1, F1), F is N * F1.

big(123456789012345678901234567890).
big(-98765432109876543210).

main :-
    fact(25, F25), write(F25), nl,
    fact(30, F30), Q is F30 // F25, write(Q), nl,
    X is 2 ** 100, write(X), nl,
    Y is X - X + 5, write(Y), nl, ( integer(Y) -> write(small) ; write(other) ), nl,
    Z is -(2 ** 70), write(Z), nl, A is abs(Z), write(A), nl,
    M is X mod 1000007, write(M), nl,
    D is (2 ** 80) // 3, R is (2 ** 80) rem 3, write(D/R), nl,
    N is -(2 ** 80) // 3, write(N), nl, N2 is -(2 ** 80) div 3, write(N2), nl,
    S is 1 << 60, write(S), nl, S2 is S >> 58, write(S2), nl,
    ( X > 1 -> write(gt) ; write(le) ), nl,
    ( Z < 0 -> write(neg) ; write(pos) ), nl,
    ( 2 ** 64 =:= 18446744073709551616 -> write(eq) ; write(ne) ), nl,
    ( 2 ** 64 == 18446744073709551616 -> write(same) ; write(diff) ), nl,
    ( f(2 ** 64) = f(_) -> write(unified) ; write(no) ), nl,
    big(B1), write(B1), nl,
    findall(B, big(B), Bs), write(Bs), nl,
    ( big(123456789012345678901234567890) -> write(found) ; write(missing) ), nl,
    msort([X, 1, Z, 3.0, A], Sorted), write(Sorted), nl,
    Fl is float(2 ** 100), write(Fl), nl,
    T is truncate(1.0e20), write(T), nl,
    G is gcd(2 ** 90, 6 ** 40), write(G), nl,
    number_codes(NC, [0'9,0'9,0'9,0'9,0'9,0'9,0'9,0'9,0'9,0'9,0'9,0'9,0'9,0'9,0'9,0'9,0'9,0'9,0'9,0'9]), NC2 is NC + 1, write(NC2), nl,
    atom_number('-123456789012345678901', AN), write(AN), nl,
    atom_length(X, L), write(L), nl,
    format("~d~n", [X]),
    max_tagged_test.

max_tagged_test :- X is 140737488355327 + 1, write(X), nl, Y is X - 1, write(Y), nl.
:- initialization(main).
