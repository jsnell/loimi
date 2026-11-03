% Atoms and text: atom_codes, atom_chars, char_code, atom_length,
% atom_concat, sub_atom, number_codes, atom_number, upcase_atom,
% atomic_list_concat.
:- set_prolog_flag(double_quotes, codes).

main :-
    atom_codes(abc, C1), write(C1), nl,
    atom_codes(A1, [0'h, 0'i]), write(A1), nl,
    atom_chars(hello, Ch), write(Ch), nl,
    atom_chars(A2, [w, o, r, l, d]), write(A2), nl,
    char_code(Ch1, 0'z), char_code(a, Co), write(Ch1/Co), nl,
    atom_length(hello, L1), atom_length('', L2), atom_length(12345, L3), write([L1, L2, L3]), nl,
    atom_concat(foo, bar, A3), write(A3), nl,
    atom_concat(X1, bar, foobar), write(X1), nl,
    findall(P-S, atom_concat(P, S, abc), L4), write(L4), nl,
    findall(B-L-A, sub_atom(abc, B, L, A, b), L5), write(L5), nl,
    findall(Sub, sub_atom(abcd, _, 2, _, Sub), L6), write(L6), nl,
    ( sub_atom(hello_world, Bef, _, 0, world) -> write(Bef) ; write(no) ), nl,
    number_codes(N1, "42"), N2 is N1 + 1, write(N2), nl,
    number_codes(N3, " 7"), write(N3), nl,
    number_codes(3.5, C3), atom_codes(A4, C3), write(A4), nl,
    atom_number('12', N4), atom_number('1.5e3', N5), write(N4/N5), nl,
    ( atom_number(abc, _) -> write(yes) ; write(no) ), nl,
    atom_number(A5, 99), write(A5), nl,
    upcase_atom('hello World', U), write(U), nl,
    atomic_list_concat([a, b, 1, 2.5], A6), write(A6), nl,
    atomic_list_concat([x, y, z], '-', A7), write(A7), nl,
    atomic_list_concat(Parts, ',', 'a,b,,c'), write(Parts), nl,
    atom_string_test,
    X = "abc", write(X), nl,
    atom_codes(A8, X), write(A8), nl,
    0'a =:= 97, write(char_code_ok), nl,
    write(0' ), nl, write(0'''), nl, write(0'\n), nl,
    atom_chars(X9, ['1', '2']), atom(X9), write(X9), nl,
    number_chars(N9, ['1', '2']), integer(N9), write(N9), nl,
    char_type(a, alpha), char_type('A', upper(Lw)), write(Lw), nl,
    ( char_type('1', digit(W)) -> write(W) ; write(nodigit) ), nl,
    'hello world' = Q, writeq(Q), nl.

atom_string_test :- atom_length(abc, 3), write(ok), nl.
:- initialization(main).
