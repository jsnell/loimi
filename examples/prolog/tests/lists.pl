% findall/bagof/setof, list library predicates, sorting, DCG.
:- set_prolog_flag(double_quotes, codes).

age(peter, 7). age(ann, 11). age(pat, 8). age(tom, 5). age(mike, 11).
cls(a, x). cls(b, y). cls(c, x). cls(d, z). cls(e, y).

greeting --> [hello], name.
name --> [world].
name --> [prolog].

digits([D|T]) --> digit(D), digits(T).
digits([D]) --> digit(D).
digit(D) --> [D], { code_type_digit(D) }.
code_type_digit(D) :- D >= 0'0, D =< 0'9.

expr(V) --> term(T), expr_rest(T, V).
expr_rest(Acc, V) --> "+", !, term(T), { A is Acc + T }, expr_rest(A, V).
expr_rest(Acc, V) --> "-", !, term(T), { A is Acc - T }, expr_rest(A, V).
expr_rest(V, V) --> [].
term(V) --> digits(Ds), { number_codes(V, Ds) }.

ab --> [].
ab --> [a], ab, [b].

main :-
    findall(N-A, age(N, A), L1), write(L1), nl,
    ( bagof(N, age(N, 11), L2) -> write(L2) ; write(none) ), nl,
    ( bagof(N, age(N, 99), L3) -> write(L3) ; write(none) ), nl,
    findall(A-Ns, bagof(N, age(N, A), Ns), L4), write(L4), nl,
    setof(A, N^age(N, A), L5), write(L5), nl,
    setof(N-A, age(N, A), L6), write(L6), nl,
    findall(C-Xs, setof(X, cls(X, C), Xs), L7), write(L7), nl,
    ( append(P, S, [1, 2]), write(P/S), nl, fail ; true ),
    findall(P-S, append(P, S, [1, 2, 3]), L8), length(L8, N8), write(N8), nl,
    append([[1], [2, 3], [], [4]], Fl), write(Fl), nl,
    reverse([1, 2, 3], R), write(R), nl,
    nth0(1, [a, b, c], E0), nth1(1, [a, b, c], E1), write(E0/E1), nl,
    findall(I-E, nth1(I, [x, y], E), L9), write(L9), nl,
    last([1, 2, 3], La), write(La), nl,
    msort([b, a, c, a], Ms), write(Ms), nl,
    sort([3, 1, 2, 1], So), write(So), nl,
    predsort(cmp3, [c, b, a, b], PS), write(PS), nl,
    select(b, [a, b, c], Sel), write(Sel), nl,
    selectchk(a, [a, b, a], Sel2), write(Sel2), nl,
    subtract([1, 2, 3, 4], [2, 4], Sub), write(Sub), nl,
    intersection([1, 2, 3], [2, 3, 4], Int), union([1, 2], [2, 3], Un), write(Int/Un), nl,
    delete([a, b, a, c], a, Del), write(Del), nl,
    exclude(gt2, [1, 2, 3, 4], Ex), write(Ex), nl,
    list_to_set([a, b, a, c, b], Set), write(Set), nl,
    findall(Pm, permutation([1, 2, 3], Pm), Pms), write(Pms), nl,
    flatten([1, [2, [3, 4]], [], [[5]]], Fl2), write(Fl2), nl,
    sum_list([1, 2, 3], Sm), max_member(Mx, [3, 1, 4]), min_member(Mn, [3, 1, 4]), write(Sm/Mx/Mn), nl,
    maplist(succ, [1, 2, 3], Ms2), write(Ms2), nl,
    foldl(plus, [1, 2, 3], 0, Fo), write(Fo), nl,
    include(integer, [a, 1, b, 2], Inc), write(Inc), nl,
    partition(atom, [a, 1, b, 2], PI, PE), write(PI/PE), nl,
    pairs_keys_values(PKV, [a, b], [1, 2]), write(PKV), nl,
    memberchk(b, [a, b, c]), write(memberchk_ok), nl,
    length(LL, 2), write(len2), nl, LL = [_, _],
    phrase(greeting, [hello, world]), write(dcg1), nl,
    findall(W, phrase(greeting, [hello, W]), Ws), write(Ws), nl,
    phrase(expr(V), "12+30-2"), write(V), nl,
    ( phrase(ab, [a, a, b, b]) -> write(ab_yes) ; write(ab_no) ), nl,
    ( phrase(ab, [a, b, b]) -> write(ab_yes) ; write(ab_no) ), nl,
    phrase(digits(Ds), "123", Rest), atom_codes(DA, Ds), write(DA-Rest), nl.
cmp3(O, A, B) :- compare(O, A, B).
gt2(X) :- X > 2.
:- initialization(main).
