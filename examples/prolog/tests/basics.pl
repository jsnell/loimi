app([], L, L).
app([H|T], L, [H|R]) :- app(T, L, R).
nrev([], []).
nrev([H|T], R) :- nrev(T, RT), app(RT, [H], R).

main :-
    write(hello), nl,
    X = f(a, _, 'x y'), print(X), nl,
    app([1,2], [3], L), write(L), nl,
    nrev([1,2,3,4,5], R), write(R), nl,
    Y is 3 + 4 * 2, write(Y), nl,
    ( member(Z, [a,b,c]), write(Z), nl, fail ; true ),
    findall(Q, member(Q, [x,y]), Qs), write(Qs), nl,
    catch(throw(oops), E, (write(caught(E)), nl)),
    ( 1 > 2 -> write(yes) ; write(no) ), nl.
:- initialization(main).
