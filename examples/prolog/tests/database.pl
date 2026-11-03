% assert, asserta, retract, retractall, clause/2, the logical update
% view, dynamic declarations, and global counters.
:- dynamic(fact/1).
:- dynamic counter/1, edge/2.

counter(0).

incr :- retract(counter(N)), N1 is N + 1, assertz(counter(N1)).

main :-
    assertz(fact(1)), assertz(fact(2)), asserta(fact(0)),
    findall(X, fact(X), L1), write(L1), nl,
    retract(fact(1)), findall(X, fact(X), L2), write(L2), nl,
    incr, incr, incr, counter(C), write(C), nl,
    % logical update view: the clauses seen are those at call time
    findall(X, (fact(X), X < 10, Y is X + 10, assertz(fact(Y))), L3), write(L3), nl,
    findall(X, fact(X), L4), write(L4), nl,
    retractall(fact(_)), findall(X, fact(X), L5), write(L5), nl,
    ( fact(_) -> write(yes) ; write(no) ), nl,
    assertz((double(X, Y2) :- Y2 is X * 2)), double(21, D), write(D), nl,
    clause(double(_, _), Body), Body = (_ is _ * K), write(K), nl,
    assertz(edge(a, b)), assertz(edge(b, c)), assertz(edge(c, d)), assertz(edge(a, e)),
    findall(T, path(a, T), L6), write(L6), nl,
    findall(F-T2, retract(edge(F, T2)), L7), write(L7), nl,
    findall(F, edge(F, _), L8), write(L8), nl,
    ( catch(retract(undefined_dyn(_)), E, (write(E), nl)) -> true ; write(retract_failed), nl ),
    retractall(newpred(_)), ( newpred(_) -> true ; write(newpred_empty) ), nl,
    forall(between(1, 1000, I), assertz(num(I))),
    aggregate_all(count, num(_), Cnt), write(Cnt), nl,
    aggregate_all(sum(I2), num(I2), Sum), write(Sum), nl,
    retract(num(500)), ( num(500) -> write(still) ; write(gone) ), nl,
    aggregate_all(count, num(_), Cnt2), write(Cnt2), nl,
    nb_setval(key, f(1, [a])), nb_getval(key, V), write(V), nl,
    abolish_test.

path(X, Y) :- edge(X, Y).
path(X, Y) :- edge(X, Z), path(Z, Y).

abolish_test :- write(end), nl.
:- initialization(main).
