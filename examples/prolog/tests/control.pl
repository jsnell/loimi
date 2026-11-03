% Cut, if-then-else, negation, disjunction, call/N, and their interaction.

t(1). t(2). t(3).

first(X) :- t(X), !.
deep(X) :- t(X), X > 1, !.
nocut(X) :- t(X).

ite(X, R) :- ( X > 1 -> R = big ; X =:= 1 -> R = one ; R = small ).
ite_cut(X, R) :- ( t(X), ! ; X = none ), R = X.
cut_in_then(R) :- ( true -> t(R), ! ; R = no ).
cut_in_else(R) :- ( fail -> R = no ; t(R), ! ).
cut_in_cond(L) :- findall(X, ( ( t(X), ! -> true ; true ) ), L).
neg(X) :- \+ t(X).
call_cut(L) :- findall(X, call((t(X), !)), L).
call_cut2(L) :- findall(X, (call(t(X)), !), L).
or_cut(L) :- findall(X, (t(X) ; X = 4), L).
soft(X) :- ( t(X) *-> true ; X = none ).

p(a). p(b). p(c).
q(b). q(c).
both(X) :- p(X), q(X).

count(G, N) :- findall(x, G, L), length(L, N).

nested(R) :- ( t(X), ( X > 1 -> Y = X ; fail ), Y < 3 -> R = Y ; R = none ).

once_t(L) :- findall(X, once(t(X)), L).
ignore_t :- ignore(fail).
forall_t :- forall(t(X), X > 0).

callN(L) :- findall(Y, (member(X, [1,2,3]), call(plus(10), X, Y)), L).
callN2(R) :- G = format("~w-~w~n"), call(G, [a, b]), R = ok.

main :-
    first(A), write(first(A)), nl,
    deep(B), write(deep(B)), nl,
    findall(X, nocut(X), L1), write(L1), nl,
    ite(5, R1), ite(1, R2), ite(0, R3), write([R1, R2, R3]), nl,
    ite_cut(Y, R4), write(Y-R4), nl,
    cut_in_then(C1), write(then(C1)), nl,
    cut_in_else(C2), write(else(C2)), nl,
    cut_in_cond(L2), write(cond(L2)), nl,
    ( neg(4) -> write(neg4_yes) ; write(neg4_no) ), nl,
    ( neg(1) -> write(neg1_yes) ; write(neg1_no) ), nl,
    call_cut(L3), write(call_cut(L3)), nl,
    call_cut2(L4), write(call_cut2(L4)), nl,
    or_cut(L5), write(or(L5)), nl,
    findall(S, soft(S), L6), write(soft(L6)), nl,
    findall(Z, both(Z), L7), write(both(L7)), nl,
    count(t(_), N), write(count(N)), nl,
    nested(R5), write(nested(R5)), nl,
    once_t(L8), write(once(L8)), nl,
    ( ignore_t -> write(ignore_ok) ; write(ignore_fail) ), nl,
    ( forall_t -> write(forall_ok) ; write(forall_fail) ), nl,
    callN(L9), write(callN(L9)), nl,
    callN2(R6), write(R6), nl,
    ( \+ \+ (X1 = 1, X1 == 1) -> write(dneg_ok) ; write(dneg_fail) ), nl,
    ( var(X1) -> write(still_var) ; write(bound) ), nl,
    G2 = (member(M, [x, y, z]), M \== x), findall(M, G2, L10), write(L10), nl,
    ( member(Q, [1, 2, 3]), Q > 1 -> write(q(Q)) ; write(noq) ), nl,
    ( fail ; write(second_branch) ), nl,
    \+ fail, write(done), nl.
:- initialization(main).
