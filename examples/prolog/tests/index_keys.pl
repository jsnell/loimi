% First-argument indexing on atomic keys that collide under some word
% encodings: 0 (the all-zero word of nanbox(int_high)), 0.0, -0.0, [],
% small and large integers, atoms, compound terms and lists. Each call
% must find exactly the clauses plain unification would.

k(0, int_zero).
k(0.0, float_zero).
k(-0.0, float_negzero).
k([], nil).
k(1, one).
k(-1, minus_one).
k(140737488355327, int_max).
k(-140737488355328, int_min).
k(140737488355328, big).
k(a, atom_a).
k(f(x), f1).
k(f(x, y), f2).
k([_|_], list).
k(32768, k32768).
k(2.5, two_and_a_half).

all(X) :- findall(N, k(X, N), L), write(X-L), nl.

:- initialization(main).
main :-
    all(0), all(0.0), all(-0.0), all([]), all(1), all(-1),
    all(140737488355327), all(-140737488355328), all(140737488355328),
    all(a), all(f(x)), all(f(x, y)), all([q]), all(32768), all(2.5),
    all(2), all(b),
    findall(K, k(K, _), Ks), length(Ks, Len), write(Len), nl,
    halt.
