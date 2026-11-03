% Benchmark driver, appended (after an `iterations(K).` fact) to each
% programs/NAME.pl by run.py. Runs top/0 K times; `\+ \+` undoes each
% run's bindings, as SWI-Prolog's own harness does (run.pl's
% not_not_top/1). Prints `done` if every run succeeded.
bench(N) :- N =< 0, !.
bench(N) :- \+ \+ top, N1 is N - 1, bench(N1).

main :- iterations(K), ( bench(K) -> write(done) ; write(failed) ), nl.
