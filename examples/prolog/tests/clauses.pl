% Clause selection without choicepoints: guard chains, clauses whose
% first comparisons exclude each other, and heads that bind before a
% neck cut; each must behave as plain backtracking would.

% Exclusive first comparisons (no cut).
tak(X, Y, Z, A) :- X =< Y, Z = A.
tak(X, Y, Z, A) :- X > Y,
    X1 is X - 1, tak(X1, Y, Z, A1),
    Y1 is Y - 1, tak(Y1, Z, X, A2),
    Z1 is Z - 1, tak(Z1, X, Y, A3),
    tak(A1, A2, A3, A).

% Swapped operands: Y < X is X > Y.
sw(X, Y, le) :- X =< Y.
sw(X, Y, gt) :- Y < X.

% Overlapping: both hold when X =:= Y.
ov(X, Y, a) :- X =< Y.
ov(X, Y, b) :- X >= Y.

% Equality against inequality, with an integer operand.
eq(X, zero) :- X =:= 0.
eq(X, other) :- X =\= 0.

% Three clauses, all exclusive.
sign(X, neg) :- X < 0.
sign(X, zero) :- X =:= 0.
sign(X, pos) :- X > 0.

% The second clause's first goal is not a comparison: no commit.
nc(X, small) :- X < 5.
nc(_, any).

% Non-integers and errors.
cmp(X, Y, lt) :- X < Y.
cmp(X, Y, ge) :- X >= Y.

% Heads binding before a neck cut, with the other clauses as fallback.
d(U+V, X, DU+DV) :- !, d(U, X, DU), d(V, X, DV).
d(U*V, X, DU*V+U*DV) :- !, d(U, X, DU), d(V, X, DV).
d(log(U), X, DU/U) :- !, d(U, X, DU).
d(X, X, 1) :- !.
d(_, _, 0).

part([X|L], Y, [X|L1], L2) :- X =< Y, !, part(L, Y, L1, L2).
part([X|L], Y, L1, [X|L2]) :- part(L, Y, L1, L2).
part([], _, [], []).

% The head binds an argument, then a later argument fails: the binding
% must be undone for the next clause.
hb(f(A), b, A).
hb(f(x), _, second).
hb(_, _, third).

hc(X, Y) :- X = g(Y), Y > 1, !.
hc(_, none).

% arg/3 into a new variable (temporary and permanent).
ga(T, A, B) :- arg(1, T, A), arg(2, T, B).
gp(T, L) :- arg(2, T, A), findall(x, true, _), arg(1, T, B), L = [A, B].
gl(T, A) :- arg(1, T, A).
gn(T, N, A) :- ( arg(N, T, A) -> true ; A = none ).

% functor/3 inline: reading a term, building one.
fr(T, F, N) :- functor(T, F0, N0), F = F0, N = N0.
fp(T, L) :- functor(T, F, N), findall(x, true, _), L = F/N.
fw(F, N, T) :- functor(T0, F, N), T = T0.
fwp(F, N, L) :- functor(T, F, N), findall(x, true, _), L = [T].
fe(T, F, N) :- catch(fw(F, N, T), error(E, _), (T = E)).

% compare/3 into a new variable.
co(A, B, O) :- compare(O0, A, B), O = O0.
cop(A, B, L) :- compare(O, A, B), findall(x, true, _), L = [O].

t_compare :-
    findall(O, (member(A-B, [1-2, 2-1, 3-3, a-b, f(x)-f(x), 1.0-1, _-a, "ab"-"ab", f(a)-g(a), [1]-[1,2]]), co(A, B, O)), L1),
    write(L1), nl,
    cop(z, y, L2), write(L2), nl,
    ( compare(<, 1, 2) -> write(yes) ; write(no) ), nl,
    catch(compare(foo, 1, 2), error(E, _), (write(E), nl)).

t_functor :-
    findall(F/N, (member(T, [f(a, b), [x], abc, 3, 2.5, g(_)]), fr(T, F, N)), L1), write(L1), nl,
    fp(h(1, 2, 3), P1), write(P1), nl,
    fw(foo, 3, W1), functor(W1, WF, WN), write(WF/WN), nl,
    fw('[|]', 2, W2), ( W2 = [_|_] -> write(list) ; write(nolist) ), nl,
    fw(abc, 0, W3), write(W3), nl, fw(7, 0, W4), write(W4), nl,
    fwp(bar, 2, [W5]), arg(1, W5, A5), ( var(A5) -> write(var) ; write(nonvar) ), nl,
    fe(E1, _, 2), write(E1), nl, fe(E2, foo, _), write(E2), nl, fe(E3, foo, -1), write(E3), nl,
    fe(E4, foo(a), 1), write(E4), nl, fe(E5, foo, a), write(E5), nl,
    catch(fr(_, _, _), error(E6, _), true), write(E6), nl.

t_arg :-
    ga(f(a, g(X0)), A1, B1), write(A1), nl, X0 = 1, write(B1), nl,
    gp(h(p, q, r), L0), write(L0), nl,
    gl([h|t], H0), write(H0), nl,
    gn(f(a), 2, N0), gn(f(a), 0, N1), gn(f(a, b), 2, N2), write([N0, N1, N2]), nl,
    catch(gl(_, _), error(E0, _), (write(E0), nl)),
    catch(gn(f(a), x, _), error(E1, _), (write(E1), nl)),
    ( ga(f(Y0, Y0), 1, 2) -> write(bad) ; write(ok) ), nl.

main :-
    t_functor, t_arg, t_compare,
    tak(18, 12, 6, T), write(tak(T)), nl,
    findall(R, sw(1, 2, R), L1), findall(R, sw(2, 1, R), L2), findall(R, sw(1, 1, R), L3),
    write(sw(L1, L2, L3)), nl,
    findall(R, ov(1, 1, R), L4), findall(R, ov(1, 2, R), L5), findall(R, ov(2, 1, R), L6),
    write(ov(L4, L5, L6)), nl,
    findall(R, eq(0, R), L7), findall(R, eq(3, R), L8), findall(R, eq(0.0, R), L9),
    write(eq(L7, L8, L9)), nl,
    findall(S, (member(X, [-2, 0, 3, 1.5, -0.5]), sign(X, S)), L10), write(sign(L10)), nl,
    findall(R, nc(1, R), L11), findall(R, nc(7, R), L12), write(nc(L11, L12)), nl,
    findall(R, cmp(1, 2.0, R), L13), findall(R, cmp(2.5, 2, R), L14), findall(R, cmp(1+1, 2, R), L15),
    write(cmp(L13, L14, L15)), nl,
    catch(cmp(_, 1, _), error(E, _), (write(caught(E)), nl)),
    findall(R, sw(1, 1.0, R), L16), write(sw_mixed(L16)), nl,
    d(log(log(x)) + x * x, x, D), write(D), nl,
    d(y, x, D0), write(D0), nl,
    findall(D1, d(x, x, D1), L17), write(L17), nl,
    findall(D2, d(log(x), x, D2), L18), write(L18), nl,
    findall(yes, d(log(x), x, 1/x), L19), write(L19), nl,
    findall(D4, (Z = log(x), d(Z, x, D4)), L20), write(L20), nl,
    part([3, 1, 4, 1, 5, 9, 2, 6], 4, Lo, Hi), write(Lo-Hi), nl,
    findall(Lo2-Hi2, part([3, 1, 4], 3, Lo2, Hi2), L21), write(L21), nl,
    findall(R, part([1, 5], 3, [1], R), L22), write(L22), nl,
    findall(B-R, (hb(f(A), c, R), (var(A) -> B = var ; B = A)), L23), write(L23), nl,
    findall(B, (hb(f(A), b, R), (var(A) -> (R == A -> B = same ; B = R) ; B = A-R)), L24), write(L24), nl,
    findall(R, hc(g(5), R), L26), findall(R, hc(g(0), R), L27), findall(R, hc(h, R), L28),
    write(L26-L27-L28), nl.
:- initialization(main).
