% Enough allocation for many collections: long lists, deep recursion,
% backtracking over large structures, findall results, exceptions carrying
% terms, asserted clauses, first-argument indexing on large predicates.
:- dynamic(store/2).

numl(N, N, [N]) :- !.
numl(I, N, [I|T]) :- I1 is I + 1, numl(I1, N, T).

len([], N, N).
len([_|T], N0, N) :- N1 is N0 + 1, len(T, N1, N).

sum([], S, S).
sum([X|Xs], S0, S) :- S1 is S0 + X, sum(Xs, S1, S).

% non-tail recursion: a deep environment stack
build(0, []) :- !.
build(N, [f(N, g(N))|T]) :- N1 is N - 1, build(N1, T), true.

nrev([], []).
nrev([H|T], R) :- nrev(T, RT), app(RT, [H], R).
app([], L, L).
app([H|T], L, [H|R]) :- app(T, L, R).

loop(0) :- !.
loop(N) :- numl(1, 300, L), nrev(L, R), R = [300|_], N1 is N - 1, loop(N1).

tree(0, leaf) :- !.
tree(D, node(L, D, R)) :- D1 is D - 1, tree(D1, L), tree(D1, R).
count(leaf, 0).
count(node(L, _, R), N) :- count(L, NL), count(R, NR), N is NL + NR + 1.

% a predicate with many clauses, indexed on the first argument
fact(N, F) :- N mod 3 =:= 0, !, F = fizz.
fact(_, other).
:- forall(between(1, 2000, I), (J is I * I, assertz(store(I, J)))).

pick(N, X) :- member(X, [a, b, c, d, e]), N > 0.

main :-
    numl(1, 100000, L), len(L, 0, N), write(N), nl,
    sum(L, 0, S), write(S), nl,
    build(50000, B), len(B, 0, NB), write(NB), nl, B = [First|_], write(First), nl,
    loop(200), write(loop_done), nl,
    tree(14, T), count(T, NT), write(NT), nl,
    findall(X-Y, (between(1, 20000, X), Y = f(X)), Ps), len(Ps, 0, NP), write(NP), nl,
    findall(Z, (member(Z0, L), Z0 mod 1000 =:= 0, Z = Z0), Zs), len(Zs, 0, NZ), write(NZ), nl,
    catch((numl(1, 5000, Big), throw(big(Big))), big(BB), (len(BB, 0, NBB), write(caught(NBB)), nl)),
    store(1500, Sq), write(Sq), nl,
    findall(K, (store(K, V), V > 3999000), Ks), write(Ks), nl,
    aggregate_all(count, (between(1, 30000, I), fact(I, fizz)), Fz), write(Fz), nl,
    findall(P, (between(1, 2000, I2), pick(I2, P)), Pk), len(Pk, 0, NPk), write(NPk), nl,
    copy_term(T, T2), count(T2, NT2), write(NT2), nl,
    atom_codes(A, [0'a, 0'b, 0'c]), write(A), nl,
    garbage_collect, write(done), nl.
:- initialization(main).
