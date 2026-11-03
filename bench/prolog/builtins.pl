% List the builtins (predicates called but not defined in the file), the
% control constructs, and the arithmetic functions a program uses in the
% clauses reachable from top/0 and in its directives. Arithmetic in
% expressions built at run time (eval.pl) is not seen.
% Usage: swipl builtins.pl programs/*.pl
% Prints: NAME: preds [...] control [...] arith [...] dcg true/false
% (DCG rules are translated by SWI's dcg_translate_rule/2 first).
% SWI-Prolog only (a development aid, not part of the benchmark).

:- initialization(main, main).

main(Files) :- maplist(file, Files).

file(File) :-
    file_base_name(File, Base), file_name_extension(Name, _, Base),
    nb_setval(dcg, false),
    read_clauses(File, Clauses),
    nb_getval(dcg, DCG),
    findall(N/A, (member(C, Clauses), head(C, H), functor(H, N, A)), Defs0),
    sort(Defs0, Defs),
    findall(O-G, (member(C, Clauses), owner(C, O), body(C, B), goal(B, G)), Edges),
    reach([(:-)/0, top/0], Edges, [], Live),
    findall(G, (member(O-G, Edges), memberchk(O, Live)), Gs0),
    sort(Gs0, Gs),
    partition([X]>>(X = ctl(_)), Gs, Ctl0, Other),
    findall(P, member(ctl(P), Ctl0), Ctl),
    findall(P, (member(call(P), Other), \+ memberchk(P, Defs)), Preds),
    findall(F, member(arith(F), Other), Ar),
    format('~w: preds ~w control ~w arith ~w dcg ~w~n', [Name, Preds, Ctl, Ar, DCG]).

% Only clauses reachable from top/0 and the directives count.
owner((:- _), (:-)/0) :- !.
owner(C, N/A) :- head(C, H), functor(H, N, A).

reach([], _, Live, Live).
reach([P|Ps], Edges, Live0, Live) :-
    (   memberchk(P, Live0)
    ->  reach(Ps, Edges, Live0, Live)
    ;   findall(Q, member(P-call(Q), Edges), Qs),
        append(Ps, Qs, Ps1),
        reach(Ps1, Edges, [P|Live0], Live)
    ).

read_clauses(File, Cs) :-
    setup_call_cleanup(open(File, read, S), read_all(S, Cs), close(S)).
read_all(S, Cs) :-
    read_term(S, T, []),
    (   T == end_of_file -> Cs = []
    ;   ( T = (:- op(P, Ty, O)) -> op(P, Ty, O) ; true ),
        ( T = (_ --> _) -> dcg_translate_rule(T, T1), nb_setval(dcg, true) ; T1 = T ),
        Cs = [T1|Cs1], read_all(S, Cs1)
    ).

head((:- _), _) :- !, fail.
head((H :- _), H) :- !.
head(H, H).

body((:- B), B) :- !.
body((_ :- B), B).

control(','/2). control(';'/2). control('->'/2). control('\\+'/1).
control(call/1). control(findall/3). control(bagof/3). control(setof/3).
control('*->'/2). control(forall/2). control(catch/3). control(once/1).
control(!/0). control(true/0). control(fail/0).

meta(','(A,B), [A,B]). meta(';'(A,B), [A,B]). meta('->'(A,B), [A,B]).
meta('*->'(A,B), [A,B]). meta(\+ A, [A]). meta(call(A), [A]).
meta(once(A), [A]). meta(forall(A,B), [A,B]). meta(catch(A,_,B), [A,B]).
meta(findall(_,A,_), [A]). meta(bagof(_,A,_), [A]). meta(setof(_,A,_), [A]).
meta(_^A, [A]).

arith_pred(is/2). arith_pred((=:=)/2). arith_pred((=\=)/2). arith_pred((<)/2).
arith_pred((>)/2). arith_pred((=<)/2). arith_pred((>=)/2).

goal(V, call(call/1)) :- var(V), !.
goal(G, R) :-
    functor(G, N, A),
    (   G = _^_ -> true
    ;   control(N/A) -> R0 = ctl(N/A)
    ;   R0 = call(N/A)
    ),
    (   nonvar(R0), R = R0
    ;   meta(G, Args), member(X, Args), goal(X, R)
    ;   arith_pred(N/A), arg(I, G, E), between(1, 2, I), expr(E, R)
    ).

expr(E, _) :- var(E), !, fail.
expr(E, _) :- number(E), !, fail.
expr(E, arith(N/A)) :- compound(E), functor(E, N, A).
expr(E, arith(E)) :- atom(E).
expr(E, R) :- compound(E), arg(_, E, X), expr(X, R).
