% catch/3 and throw/1: unwinding, bindings undone, rethrow, errors from
% builtins, catch inactive after its goal exits.

t(1). t(2). t(3).

e1 :- catch(throw(my(err)), my(X), (write(caught(X)), nl)).
e2 :- catch((X = 1, throw(ball(X))), ball(Y), (write(y(Y)), nl)), ( var(X) -> write(x_unbound) ; write(x_bound) ), nl.
e3 :- catch(catch(throw(inner), outer, write(wrong)), inner, write(right)), nl.
e4 :- catch(catch(throw(a), b, true), E, (write(rethrown(E)), nl)).
e5 :- catch(X is foo + 1, error(E, _), (write(E), nl)), var(X).
e6 :- catch(atom_length(X, _), error(E, _), (write(E), nl)), var(X).
e7 :- catch(undefined_pred_xyz, error(E, _), (write(E), nl)).
e8 :- catch(X is 1 / 0, error(E, _), (write(E), nl)), var(X).
e9 :- catch(atom_length(123456, L), _, true), write(len(L)), nl.
e10 :- catch(functor(_, foo, -1), error(E, _), (write(E), nl)).
e11 :- catch(arg(x, f(a), _), error(E, _), (write(E), nl)).
e12 :- catch(atom_codes(_, _), error(E, _), (write(E), nl)).
% A catch whose goal left choicepoints is not active for the continuation.
e13 :- catch(catch(t(X), _, (write(inner_caught), nl)), outer, (write(outer_caught(X)), nl)), X >= 2, throw(outer).
% Backtracking into the goal re-activates the catch.
e14 :- catch(( t(X), X >= 2, throw(found(X)) ), found(Y), (write(found(Y)), nl)).
e15 :- findall(X, catch(( t(X), X =:= 2, throw(skip) ), skip, X = skipped), L), write(L), nl.
e16 :- catch(( member(X, [1,2,3]), X > 1, throw(x(X)) ), x(V), true), write(v(V)), nl.
deep(0) :- throw(bottom).
deep(N) :- N1 is N - 1, deep(N1).
e17 :- catch(deep(1000), B, (write(B), nl)).
e18 :- catch(call(1), error(E, _), (write(E), nl)).
e19 :- catch(throw(f(_X, _Y)), f(A, B), (A = 1, B = 2, write(A-B), nl)).
e20 :- catch((between(1, 5, X), X > 3, throw(big(X))), big(N), (write(big(N)), nl)).
e21 :- catch(number_codes(N, "12x"), error(E, _), (write(syntax), nl)), var(N), (nonvar(E) -> true ; true).
e22 :- catch(succ(X, 0), _, write(err)), write(X), nl.
e23 :- catch(catch(throw(first), first, throw(second)), second, (write(second_caught), nl)).

run(G) :- ( catch(G, E, (write(uncaught(G, E)), nl)) -> true ; write(failed(G)), nl ).

main :-
    run(e1), run(e2), run(e3), run(e4), run(e5), run(e6), run(e7), run(e8),
    run(e9), run(e10), run(e11), run(e12), run(e13), run(e14), run(e15),
    run(e16), run(e17), run(e18), run(e19), run(e20), run(e21), run(e22), run(e23).
:- initialization(main).
