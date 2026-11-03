% Arithmetic with integers and floats, comparison, and float output.

show(E) :- X is E, write(X), nl.

main :-
    show(1 + 2 * 3 - 4),
    show(7 // 2), show(-7 // 2), show(7 mod 3), show(-7 mod 3), show(7 rem -3),
    show(-7 rem 3), show(7 div 2), show(-7 div 2),
    show(7 / 2), show(8 / 2), show(1 / 3), show(2.0 * 3),
    show(1.5 + 1.5), show(0.1 + 0.2), show(1.0e10), show(1.0e15), show(1.0e16),
    show(123456789.0 * 10), show(1.0e-4), show(1.0e-5), show(-2.5), show(100.0),
    show(abs(-3)), show(abs(-3.5)), show(sign(-2)), show(sign(2.5)),
    show(min(3, 2.0)), show(max(3, 2)), show(max(1, 1.0)),
    show(sqrt(16)), show(2 ** 10), show(2 ** -1), show(2.0 ** 3), show(2 ^ 10), show(9 ^ 0),
    show(truncate(3.7)), show(truncate(-3.7)), show(round(2.5)), show(round(-2.5)),
    show(ceiling(2.1)), show(floor(-2.1)), show(integer(2.5)), show(float(7)),
    show(float_integer_part(3.75)), show(float_fractional_part(3.75)),
    show(5 /\ 3), show(5 \/ 3), show(5 xor 3), show(\ 5), show(1 << 10), show(1024 >> 3), show(-16 >> 2),
    show(gcd(12, 18)), show(msb(1000)),
    show(pi), show(e), show(cos(0)), show(atan2(1, 1)), show(exp(1)), show(log(100)),

    show(100000 * 100000),
    show(3 * (4 + 5)), show(- (3)), show(- (-3)), show(+(4)),
    show(10 - 3 - 2), show(2 ^ 3 ^ 2),
    ( 1 < 2, 2 =< 2, 3 > 2, 3 >= 3, 1 =:= 1.0, 1 =\= 2, 1.5 < 2 -> write(cmp_ok) ; write(cmp_bad) ), nl,
    ( 0.1 + 0.2 =:= 0.3 -> write(eq) ; write(neq) ), nl,
    X = 5, Y is X * X + 1, write(Y), nl,
    sum(100, S), write(S), nl,
    fact(15, F), write(F), nl,
    fib(20, Fb), write(Fb), nl,
    Z is 10.0, ( integer(Z) -> write(int) ; write(float) ), nl,
    catch((W is 2 + a, write(W)), error(type_error(T, C), _), (write(type_error(T, C)), nl)),
    catch((W2 is 1 mod 0, write(W2)), error(Err, _), (write(Err), nl)),
    catch((W3 is foo(1), write(W3)), error(Err3, _), (write(Err3), nl)),
    succ(3, S4), succ(P4, 4), write(S4/P4), nl,
    plus(2, Q4, 7), write(Q4), nl,
    between(1, 3, B), write(B), nl, B >= 3,
    findall(Bx, between(1, 5, Bx), Bs), write(Bs), nl,
    ( between(1, 3, 5) -> write(in) ; write(out) ), nl,
    numlist(1, 5, NL), sum_list(NL, NS), max_list(NL, NM), min_list(NL, Nm), write(NS/NM/Nm), nl.

sum(0, 0) :- !.
sum(N, S) :- N1 is N - 1, sum(N1, S1), S is S1 + N.
fact(0, 1) :- !.
fact(N, F) :- N1 is N - 1, fact(N1, F1), F is N * F1.
fib(0, 0).
fib(1, 1).
fib(N, F) :- N > 1, A is N - 1, B is N - 2, fib(A, FA), fib(B, FB), F is FA + FB.
:- initialization(main).
