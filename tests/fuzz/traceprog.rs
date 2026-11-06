//! Random Mini programs for the trace tier's differential test
//! (`mini_traces_fuzz` in golden.rs): loops of arithmetic that changes
//! type with the input (ints, floats), table reads and writes, string
//! building, closures over loop variables, calls, `break`s and nested
//! loops. The input is a string constant (`tonumber("..")`): constant
//! strings are loaded from the constant pool, not baked into traces, so
//! one program's code with two inputs is the same code object, and a
//! trace made from one input runs on the other.

/// xorshift64*
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len() as u64) as usize]
    }
}

struct Gen {
    r: Rng,
    out: String,
    depth: usize,
    nfun: usize,
    /// In a function body: leaves are its parameters, not the locals.
    in_fn: bool,
}

const VARS: [&str; 4] = ["a", "b", "c", "d"];

impl Gen {
    fn line(&mut self, s: &str) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }

    /// A numeric expression over the locals, the loop variable `i` and
    /// the inputs.
    fn expr(&mut self, lv: &str, d: u32) -> String {
        if d == 0 || self.r.below(3) == 0 {
            return match self.r.below(7) {
                0 => lv.to_string(),
                1 => format!("{}", self.r.below(9) + 1),
                2 => format!("{}.5", self.r.below(4)),
                3 => self.r.pick(&["N", "M", "F"]).to_string(),
                _ if self.in_fn => self.r.pick(&["x", "y"]).to_string(),
                _ => self.r.pick(&VARS).to_string(),
            };
        }
        let a = self.expr(lv, d - 1);
        let b = self.expr(lv, d - 1);
        match self.r.below(6) {
            0 => format!("({} + {})", a, b),
            1 => format!("({} - {})", a, b),
            2 => format!("({} * {})", a, b),
            3 => format!("(({}) % 97)", a),
            4 => format!("({} / ({} * {} + 1))", a, b, b),
            _ => format!("math.floor({})", a),
        }
    }

    /// Keeps a local small and finite.
    fn clamp(&mut self, v: &str) {
        let k = self.r.below(7);
        self.line(&format!("if {v} > 100000 or {v} < -100000 or {v} ~= {v} then {v} = {} end", k));
    }

    fn stmt(&mut self, lv: &str, budget: &mut u32) {
        *budget = budget.saturating_sub(1);
        let v = self.r.pick(&VARS);
        // calls and closures end traces (calls are exits): rarer than the rest
        let c = match self.r.below(20) {
            10..=17 => 0,
            18 => 10,
            19 => 11,
            k => k,
        };
        match c {
            0..=3 => {
                let e = self.expr(lv, 3);
                self.line(&format!("{} = {}", v, e));
                self.clamp(v);
            }
            4 => {
                let k = self.r.below(5) + 2;
                let e = self.expr(lv, 2);
                self.line(&format!("if {} % {} == 0 then {} = {} + 0.25 else {} = {} + 1 end", lv, k, v, e, v, v));
                self.clamp(v);
            }
            5 => {
                let e = self.expr(lv, 2);
                self.line(&format!("t[({} % 50) + 1] = {}", lv, e));
            }
            6 => {
                self.line(&format!("{} = {} + (t[({} % 50) + 1] or 0)", v, v, lv));
                self.clamp(v);
            }
            7 if self.depth < 4 && *budget > 4 => {
                let iv = format!("i{}", self.depth);
                let hi = self.r.pick(&["M", "5", "N % 7 + 1", "3"]);
                self.line(&format!("for {} = 1, {} do", iv, hi));
                self.depth += 1;
                let n = self.r.below(4) + 1;
                for _ in 0..n {
                    self.stmt(&iv, budget);
                }
                self.depth -= 1;
                self.line("end");
            }
            8 => {
                let k = self.r.below(3) + 2;
                self.line(&format!("if {} > N * {} then break end", lv, k));
            }
            9 => {
                self.line(&format!("if {} % 13 == 0 then s = s .. tostring({}) .. \",\" end", lv, v));
            }
            10 if self.nfun > 0 => {
                let f = self.r.below(self.nfun as u64);
                let w = self.r.pick(&VARS);
                self.line(&format!("{} = {} + f{}({}, {})", v, v, f, lv, w));
                self.clamp(v);
            }
            _ => {
                // a closure over the loop variable, called at once
                let e = self.expr("x", 2);
                self.line(&format!("local g = function(x) return {} + {} end", e, lv));
                let w = self.r.pick(&VARS);
                self.line(&format!("{} = g({})", v, w));
                self.clamp(v);
            }
        }
    }

    fn func(&mut self, i: usize) {
        self.line(&format!("local function f{}(x, y)", i));
        self.depth += 1;
        self.in_fn = true;
        let e = self.expr("x", 2);
        self.in_fn = false;
        self.line(&format!("local r = {}", e));
        self.line("if r ~= r or r > 100000 or r < -100000 then r = 1 end");
        self.line("return r");
        self.depth -= 1;
        self.line("end");
    }

    fn loop_(&mut self, budget: &mut u32) {
        match self.r.below(3) {
            0 => {
                self.line("for i = 1, N do");
                self.depth += 1;
                while *budget > 0 && self.r.below(6) != 0 {
                    self.stmt("i", budget);
                }
                self.depth -= 1;
                self.line("end");
            }
            1 => {
                self.line("local i = 0");
                self.line("while i < N do");
                self.depth += 1;
                self.line("i = i + 1");
                while *budget > 0 && self.r.below(6) != 0 {
                    self.stmt("i", budget);
                }
                self.depth -= 1;
                self.line("end");
            }
            _ => {
                self.line("local i = 0");
                self.line("repeat");
                self.depth += 1;
                self.line("i = i + 1");
                while *budget > 0 && self.r.below(6) != 0 {
                    self.stmt("i", budget);
                }
                self.depth -= 1;
                self.line("until i >= N");
            }
        }
        self.line("sum = sum + a + b + c + d");
        self.line("if sum > 1000000000 or sum < -1000000000 or sum ~= sum then sum = 0 end");
    }
}

/// The program of `seed` with the inputs `(n, m, f)` (strings: an int,
/// an int and a number).
pub fn generate(seed: u64, input: (&str, &str, &str)) -> String {
    let mut g = Gen { r: Rng::new(seed), out: String::new(), depth: 0, nfun: 0, in_fn: false };
    g.line(&format!("-- trace fuzz program, seed {}", seed));
    g.line(&format!("local N = tonumber(\"{}\")", input.0));
    g.line(&format!("local M = tonumber(\"{}\")", input.1));
    g.line(&format!("local F = tonumber(\"{}\")", input.2));
    g.line("local a, b, c, d = 1, 2, 3.5, 0");
    g.line("local t = {}");
    g.line("local s = \"\"");
    g.line("local sum = 0");
    let nf = g.r.below(3) as usize;
    for i in 0..nf {
        g.func(i);
        g.nfun += 1;
    }
    let mut budget = 30;
    let nl = g.r.below(3) + 1;
    for _ in 0..nl {
        g.loop_(&mut budget);
    }
    g.line("print(string.format(\"%.6g %.6g %.6g %.6g %.6g\", a, b, c, d, sum))");
    g.line("print(#s, string.sub(s, 1, 60))");
    g.out
}
