//! Generator of random Loimi programs for the GC rooting fuzzer
//! (`fuzz_gc_roots` in tests/fuzz.rs).
//!
//! Each program mixes traced locals (`Gc<Node>`, `?Gc<Node>`, a struct, a
//! tuple, an enum, a nanbox `value` and an array holding them) with every
//! binding form (`let`, `var`, `let .. else`, `if let` chains, `while
//! let`, `match` with guards that collect and or-patterns, `for`, `loop`
//! with `break` values, `defer`, closures) and with early exits (`return`,
//! `break`, `continue`, `next`, `raise`, `exit`; GC closures `new(heap,
//! [var st = ..] |i| ..)` whose state the body assigns), around calls that may
//! collect (`gc(c)`, which collects under GC stress, and generated
//! functions that call it), in plain functions and in a vm's op handlers,
//! `cold` blocks, vm functions and `on raise`.
//!
//! The program checks itself: it prints digests of the data it reaches
//! (`dig` reads through every reference, so an object freed while still
//! referenced reads back as the 0x01 bytes that `--gc-check-roots`
//! fills freed objects with, or as reused memory), and the driver
//! compares the output of runs without collections (the reference),
//! under GC stress and under generational stress with verify, in every
//! dispatch mode. Objects that only out-of-scope locals reference must
//! die: a probe allocates one in a block, takes a `Weak` to it, and
//! after the block a full collection must clear it (`dead`). The program
//! prints the number of survivors (`leaks`), which must be 0 (not
//! checked under the generational heap, whose collection may be minor).
//!
//! What the generator avoids, because the language does not promise it:
//! a traced temporary held across a call that may collect (the arguments
//! of a call, a struct literal's fields: docs/status.md, "temporaries are
//! not roots"), so calls that may collect appear only where no traced
//! temporary is pending: as a statement, the whole initializer of a
//! `let`, the right side of an assignment, a digest's only argument, a
//! condition, a guard, a `match` scrutinee, a `break` or `return` value.

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03)
    }
    pub fn next(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    pub fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len() as u64) as usize]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum T {
    I,
    N,
    O,
    P,
    Tu,
    E,
    V,
    A,
}

const TRACED: [T; 7] = [T::N, T::O, T::P, T::Tu, T::E, T::V, T::A];
const RET: [T; 4] = [T::I, T::N, T::E, T::V];

fn tname(t: T) -> &'static str {
    match t {
        T::I => "i64",
        T::N => "Gc<Node>",
        T::O => "?Gc<Node>",
        T::P => "Pair",
        T::Tu => "(Gc<Node>, i64)",
        T::E => "E",
        T::V => "V",
        T::A => "[?Gc<Node>; 2]",
    }
}

/// The digest function of a type (an `i64` from what a value reaches).
fn digest(t: T, e: &str) -> String {
    match t {
        T::I => format!("(({}) % MD + MD) % MD", e),
        T::N => format!("dn({})", e),
        T::O => format!("dig({}, 3)", e),
        T::P => format!("dp({})", e),
        T::Tu => format!("dt({})", e),
        T::E => format!("de({})", e),
        T::V => format!("dv({})", e),
        T::A => format!("da({})", e),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Plain,
    Handler,
    Cold,
    VmFn,
    Raise,
    Closure,
}

#[derive(Clone)]
struct Var {
    name: String,
    t: T,
    mutable: bool,
    depth: usize,
}

#[derive(Clone, Copy)]
enum Lp {
    /// `for`, `while let`: `break` and `continue`.
    Plain,
    /// `loop` with a value of this type: `break v`.
    Val(T),
}

#[derive(Clone)]
struct Fx {
    kind: Kind,
    ret: T,
    /// No stores into objects, the context or vm state; calls only to
    /// pure functions.
    pure: bool,
    scopes: Vec<Vec<Var>>,
    loops: Vec<Lp>,
    /// Inside a probe block opened at this scope depth: traced variables
    /// declared outside it are not assigned (the probed object must not
    /// escape the block).
    probe: Option<usize>,
    /// Index of the function (plain functions call only later ones), or
    /// of the vm function.
    index: usize,
    acc: String,
    kname: String,
    /// Statement nesting depth.
    depth: usize,
    /// Assignments only to variables declared at or below this depth
    /// (closures and cold blocks: captured locals stay as they are).
    own_from: usize,
    /// Inside a `defer` body: no control transfer.
    in_defer: bool,
}

impl Fx {
    fn vars(&self, t: T) -> Vec<&Var> {
        self.scopes.iter().flatten().filter(|v| v.t == t).collect()
    }
    fn assignable(&self, t: T) -> Vec<&Var> {
        self.scopes
            .iter()
            .flatten()
            .filter(|v| {
                v.t == t && v.mutable && v.depth >= self.own_from && self.probe.map_or(true, |p| v.t == T::I || v.depth > p)
            })
            .collect()
    }
    fn vm(&self) -> bool {
        matches!(self.kind, Kind::Handler | Kind::Cold | Kind::VmFn | Kind::Raise)
    }
}

pub struct Gen {
    r: Rng,
    nfns: usize,
    rets: Vec<T>,
    pure_fns: Vec<bool>,
    nvmfns: usize,
    nops: usize,
    tmp: usize,
}

const PRELUDE: &str = r#"// Generated by tests/fuzz/rootgen.rs.
import std.io;
import std.gc;
import std.os;

object Node { v: i64, l: ?Gc<Node>, r: ?Gc<Node> }
struct Pair { a: Gc<Node>, b: ?Gc<Node>, k: i64 }
enum E { Nil, One(Gc<Node>), Two(Gc<Node>, Gc<Node>), Pr(Pair) }
value V repr nanbox { Num(f64) unboxed, R(Gc<Node>), Nil }

struct Ctx { heap: gc.Heap, keep: ?Gc<Node>, sum: i64, fuel: i64, leaks: i64, gen: bool, gcs: i64 }

const MD: i64 = 1000003;

fn mix(a: i64, b: i64) -> i64 { (a * 31 + b) % MD }

fn scan(h: *gc.Heap, p: *u8) {
    let c = p as *Ctx;
    gc_mark(h, c.keep);
}

/// A safepoint: collects at every call under GC stress, then allocates
/// garbage that reuses what was freed.
fn gc(c: *Ctx) -> i64 uses gc {
    c.gcs += 1;
    gc.maybe_collect(&c.heap);
    for i in 0..3 { let _ = new(&c.heap, Node { v: 7, l: None, r: None }); }
    1
}
fn gcb(c: *Ctx) -> bool uses gc { gc(c); true }
fn gcf(c: *Ctx) -> bool uses gc { gc(c); false }

fn mk(c: *Ctx, v: i64, l: ?Gc<Node>, r: ?Gc<Node>) -> Gc<Node> {
    new(&c.heap, Node { v: v % 1000, l: l, r: r })
}

fn dig(n: ?Gc<Node>, d: i64) -> i64 {
    match n {
        None => 1,
        Some(x) => if d == 0 { x.v } else { (x.v * 3 + dig(x.l, d - 1) * 5 + dig(x.r, d - 1) * 7) % MD },
    }
}
fn dn(n: Gc<Node>) -> i64 { dig(Some(n), 3) }
fn dp(p: Pair) -> i64 { mix(mix(dn(p.a), dig(p.b, 3)), p.k % MD) }
fn de(e: E) -> i64 {
    match e {
        E.Nil => 2,
        E.One(a) => mix(3, dn(a)),
        E.Two(a, b) => mix(dn(a), dn(b)),
        E.Pr(p) => mix(5, dp(p)),
    }
}
fn dv(v: V) -> i64 {
    match v { R(n) => mix(7, dn(n)), Num(f) => (f * 4.0) as i64, Nil => 11 }
}
fn dt(t: (Gc<Node>, i64)) -> i64 { mix(dn(t.0), t.1 % MD) }
fn da(a: [?Gc<Node>; 2]) -> i64 { mix(dig(a[0], 3), dig(a[1], 3)) }

fn pick(a: Gc<Node>, b: Gc<Node>, k: i64) -> Gc<Node> { if k % 2 == 0 { a } else { b } }
fn none() -> ?Gc<Node> { None }

fn each(c: *Ctx, n: i64, f: block fn(i64) -> i64 uses gc) -> i64 uses gc {
    var t: i64 = 0;
    for i in 0..n { t = mix(t, f(i)); gc(c); }
    t
}

/// The same with an escaping closure in the GC heap: its environment (the
/// captures and the closure's own state) is reachable only through `f`.
fn eachg(c: *Ctx, n: i64, f: Gc<closure fn(i64) -> i64 uses gc>) -> i64 uses gc {
    var t: i64 = 0;
    for i in 0..n { gc(c); t = mix(t, f(i)); }
    t
}

/// 0; counts a leak if the target of `w` survives a full collection.
fn dead(c: *Ctx, w: ?Gc<gc.Weak<Node>>) -> i64 uses gc {
    gc.collect(&c.heap);
    if let Some(x) = w {
        if !c.gen and x.get().is_some() { c.leaks += 1; }
    }
    0
}
"#;

impl Gen {
    pub fn new(seed: u64) -> Gen {
        Gen { r: Rng::new(seed), nfns: 0, rets: vec![], pure_fns: vec![], nvmfns: 0, nops: 0, tmp: 0 }
    }

    fn fresh(&mut self, p: &str) -> String {
        self.tmp += 1;
        format!("{}{}", p, self.tmp)
    }

    fn ind(fx: &Fx) -> String {
        "    ".repeat(fx.depth + 1)
    }

    // ── expressions ─────────────────────────────────────────────────

    fn lit(&mut self) -> String {
        self.r.below(20).to_string()
    }

    /// An expression of type `t` that does not collect. `d` bounds nesting.
    fn ex(&mut self, fx: &Fx, t: T, d: u32) -> String {
        let vars: Vec<String> = fx.vars(t).iter().map(|v| v.name.clone()).collect();
        if !vars.is_empty() && self.r.chance(if d == 0 { 85 } else { 50 }) {
            return self.r.pick(&vars).clone();
        }
        let leaf = d == 0;
        match t {
            T::I => match self.r.below(if leaf { 3 } else { 6 }) {
                0 => self.lit(),
                1 => fx.kname.clone(),
                2 => format!("({} % 13)", fx.acc),
                3 => {
                    let n = self.ex(fx, T::N, d - 1);
                    format!("{}.v", n)
                }
                4 => {
                    let tt = *self.r.pick(&TRACED);
                    let e = self.ex(fx, tt, d - 1);
                    digest(tt, &e)
                }
                _ => {
                    let a = self.ex(fx, T::I, d - 1);
                    let b = self.ex(fx, T::I, d - 1);
                    format!("({} + {}) % 97", a, b)
                }
            },
            T::N => {
                if leaf {
                    let v = self.lit();
                    return format!("mk(c, {}, None, None)", v);
                }
                match self.r.below(7) {
                    0 | 1 => {
                        let v = self.ex(fx, T::I, d - 1);
                        let l = self.ex(fx, T::O, d - 1);
                        let r = self.ex(fx, T::O, d - 1);
                        format!("mk(c, {}, {}, {})", v, l, r)
                    }
                    2 => {
                        let a = self.ex(fx, T::N, d - 1);
                        let b = self.ex(fx, T::N, d - 1);
                        let k = self.ex(fx, T::I, d - 1);
                        format!("pick({}, {}, {})", a, b, k)
                    }
                    3 => {
                        let o = self.ex(fx, T::O, d - 1);
                        let n = self.ex(fx, T::N, d - 1);
                        format!("({} ?? {})", o, n)
                    }
                    4 => {
                        let p = self.ex(fx, T::P, d - 1);
                        format!("{}.a", p)
                    }
                    5 => {
                        let t = self.ex(fx, T::Tu, d - 1);
                        format!("{}.0", t)
                    }
                    _ => {
                        let v = self.lit();
                        format!("mk(c, {}, None, None)", v)
                    }
                }
            }
            T::O => {
                if leaf {
                    return if self.r.chance(30) { "none()".into() } else { format!("Some(mk(c, {}, None, None))", self.lit()) };
                }
                match self.r.below(7) {
                    0 => "none()".into(),
                    1 | 2 => format!("Some({})", self.ex(fx, T::N, d - 1)),
                    3 => format!("{}.l", self.ex(fx, T::N, d - 1)),
                    4 => "c.keep".into(),
                    5 if fx.vm() => "cur".into(),
                    5 => format!("{}.b", self.ex(fx, T::P, d - 1)),
                    _ => format!("{}[{}]", self.ex(fx, T::A, d - 1), self.r.below(2)),
                }
            }
            T::P => {
                let d1 = d.saturating_sub(1);
                let a = self.ex(fx, T::N, d1);
                let b = self.ex(fx, T::O, d1);
                let k = self.ex(fx, T::I, d1);
                format!("(Pair {{ a: {}, b: {}, k: {} }})", a, b, k)
            }
            T::Tu => {
                let d1 = d.saturating_sub(1);
                let a = self.ex(fx, T::N, d1);
                let k = self.ex(fx, T::I, d1);
                format!("({}, {})", a, k)
            }
            T::E => {
                let d1 = d.saturating_sub(1);
                match self.r.below(5) {
                    0 => "E.Nil".into(),
                    1 => format!("E.One({})", self.ex(fx, T::N, d1)),
                    2 => {
                        let a = self.ex(fx, T::N, d1);
                        let b = self.ex(fx, T::N, d1);
                        format!("E.Two({}, {})", a, b)
                    }
                    _ => format!("E.Pr({})", self.ex(fx, T::P, d1)),
                }
            }
            T::V => {
                let d1 = d.saturating_sub(1);
                match self.r.below(5) {
                    0 => "V.Nil".into(),
                    1 => "V.Num(2.5)".into(),
                    2 if fx.vm() => "tos".into(),
                    _ => format!("V.R({})", self.ex(fx, T::N, d1)),
                }
            }
            T::A => {
                let d1 = d.saturating_sub(1);
                let a = self.ex(fx, T::O, d1);
                let b = self.ex(fx, T::O, d1);
                format!("[{}, {}]", a, b)
            }
        }
    }

    /// A call that may collect, of type `t`, if there is one: a generated
    /// function, a vm function, `gc`, `each` with a closure.
    fn cex(&mut self, fx: &Fx, t: T) -> Option<String> {
        let mut opts: Vec<u32> = vec![];
        let callable: Vec<usize> = (0..self.nfns)
            .filter(|&j| self.rets[j] == t && (fx.vm() || j > fx.index || fx.kind == Kind::Closure && j > fx.index))
            .filter(|&j| !fx.pure || self.pure_fns[j])
            .filter(|&j| fx.kind != Kind::Plain && fx.kind != Kind::Closure || j > fx.index)
            .collect();
        if !callable.is_empty() {
            opts.push(0);
        }
        if t == T::I {
            opts.push(1);
            if fx.kind == Kind::Plain && fx.depth < 3 {
                opts.push(2);
            }
            if matches!(fx.kind, Kind::Handler | Kind::Cold | Kind::VmFn | Kind::Raise) && !fx.pure {
                let vf: Vec<usize> = (0..self.nvmfns).filter(|&j| fx.kind != Kind::VmFn || j > fx.index).collect();
                if !vf.is_empty() {
                    opts.push(3);
                }
            }
        }
        if opts.is_empty() {
            return None;
        }
        Some(match *self.r.pick(&opts) {
            0 => {
                let j = *self.r.pick(&callable);
                let k = self.ex(fx, T::I, 1);
                let a = self.ex(fx, T::N, 1);
                let b = self.ex(fx, T::O, 1);
                format!("f{}(c, {}, {}, {})", j, k, a, b)
            }
            1 => "gc(c)".into(),
            2 => self.closure_call(fx),
            _ => {
                let vf: Vec<usize> = (0..self.nvmfns).filter(|&j| fx.kind != Kind::VmFn || j > fx.index).collect();
                let j = *self.r.pick(&vf);
                let k = self.ex(fx, T::I, 1);
                let a = self.ex(fx, T::N, 1);
                format!("vf{}({}, {})", j, k, a)
            }
        })
    }

    /// An expression of type `t` that may collect (at the top only).
    fn tex(&mut self, fx: &Fx, t: T) -> String {
        if self.r.chance(40) {
            if let Some(e) = self.cex(fx, t) {
                return e;
            }
        }
        self.ex(fx, t, 2)
    }

    fn cond(&mut self, fx: &Fx) -> String {
        match self.r.below(6) {
            0 => format!("{} % 2 == 0", self.ex(fx, T::I, 1)),
            1 => format!("{}.v > {}", self.ex(fx, T::N, 1), self.r.below(10)),
            2 => format!("{} % 3 != 1", fx.kname),
            3 if !fx.in_defer => format!("{} % 2 == 1 and gcb(c)", fx.kname),
            4 if !fx.in_defer => format!("gcb(c) and {} % 3 == 0", fx.kname),
            _ => format!("({} + {}) % 4 < 2", fx.acc, self.ex(fx, T::I, 1)),
        }
    }

    fn closure_call(&mut self, fx: &Fx) -> String {
        let i = self.fresh("i");
        let ca = self.fresh("ca");
        let mut cfx = Fx {
            kind: Kind::Closure,
            ret: T::I,
            pure: fx.pure,
            scopes: fx.scopes.clone(),
            loops: vec![],
            probe: fx.probe,
            index: fx.index,
            acc: ca.clone(),
            kname: i.clone(),
            depth: fx.depth + 1,
            own_from: fx.scopes.len() + 1,
            in_defer: false,
        };
        cfx.scopes.push(vec![
            Var { name: i.clone(), t: T::I, mutable: false, depth: fx.scopes.len() },
            Var { name: ca.clone(), t: T::I, mutable: true, depth: fx.scopes.len() + 1 },
        ]);
        let n = 1 + self.r.below(3);
        let ind = "    ".repeat(fx.depth + 2);
        if self.r.chance(40) {
            // An escaping GC closure with traced state of its own, which
            // the body may assign (through the write barrier).
            let st = self.fresh("st");
            let tt = *self.r.pick(&TRACED);
            let init = self.ex(fx, tt, 2);
            cfx.scopes.last_mut().unwrap().push(Var { name: st.clone(), t: tt, mutable: true, depth: fx.scopes.len() + 1 });
            let mut body = String::new();
            if self.r.chance(50) {
                let e = self.tex(&cfx, tt);
                body.push_str(&format!("{}{} = {};\n", ind, st, e));
            }
            body.push_str(&self.stmts_n(&mut cfx, 1, 3));
            if self.r.chance(70) {
                let e = self.tex(&cfx, tt);
                body.push_str(&format!("{}{} = {};\n", ind, st, e));
            }
            let res = digest(tt, &st);
            return format!(
                "eachg(c, {}, new(&c.heap, [var {}: {} = {}] |{}| {{\n{}var {}: i64 = {};\n{}{}mix({}, {})\n{}}}))",
                n, st, tname(tt), init, i, ind, ca, i, body, ind, ca, res, "    ".repeat(fx.depth + 1)
            );
        }
        let body = self.stmts_n(&mut cfx, 1, 3);
        format!("each(c, {}, |{}| {{\n{}var {}: i64 = {};\n{}{}{}\n{}}})", n, i, ind, ca, i, body, ind, ca, "    ".repeat(fx.depth + 1))
    }

    // ── statements ──────────────────────────────────────────────────

    fn declare(&mut self, fx: &mut Fx, name: &str, t: T, mutable: bool) {
        let depth = fx.scopes.len();
        fx.scopes.last_mut().unwrap().push(Var { name: name.into(), t, mutable, depth });
    }

    fn stmts(&mut self, fx: &mut Fx, n: usize) -> String {
        let mut out = String::new();
        for _ in 0..n {
            out.push_str(&self.stmt(fx));
        }
        out
    }

    fn stmts_n(&mut self, fx: &mut Fx, lo: u64, span: u64) -> String {
        let n = lo + self.r.below(span);
        self.stmts(fx, n as usize)
    }

    fn block_n(&mut self, fx: &mut Fx, lo: u64, span: u64) -> String {
        let n = lo + self.r.below(span);
        self.block(fx, n as usize)
    }

    /// A block's statements in a new scope.
    fn block(&mut self, fx: &mut Fx, n: usize) -> String {
        fx.scopes.push(vec![]);
        fx.depth += 1;
        let s = self.stmts(fx, n);
        fx.depth -= 1;
        fx.scopes.pop();
        s
    }

    /// The ways out of the current code (for a `let .. else` block, an
    /// early exit): each ends the statement list.
    fn diverge(&mut self, fx: &mut Fx) -> Option<String> {
        if fx.in_defer {
            return None;
        }
        let mut opts: Vec<u32> = vec![];
        match fx.kind {
            Kind::Plain | Kind::VmFn => opts.push(0),
            Kind::Handler | Kind::Cold => {
                opts.push(1);
                opts.push(1);
                opts.push(2);
            }
            Kind::Raise => opts.push(3),
            Kind::Closure => {}
        }
        if !fx.loops.is_empty() {
            opts.push(4);
            opts.push(5);
        }
        if opts.is_empty() {
            return None;
        }
        Some(match *self.r.pick(&opts) {
            0 => {
                let v = self.tex(fx, fx.ret);
                format!("return {};", v)
            }
            1 => "next".into(),
            2 => format!("raise {}", self.ex(fx, T::I, 1)),
            3 => format!("exit ({} + e) % MD", fx.acc),
            4 => match *fx.loops.last().unwrap() {
                Lp::Plain => "break;".into(),
                Lp::Val(t) => format!("break {};", self.tex(fx, t)),
            },
            _ => match *fx.loops.last().unwrap() {
                Lp::Plain => "continue;".into(),
                Lp::Val(t) => format!("break {};", self.tex(fx, t)),
            },
        })
    }

    fn stmt(&mut self, fx: &mut Fx) -> String {
        let ind = Self::ind(fx);
        let deep = fx.depth >= 4;
        loop {
            let k = self.r.below(100);
            let s = match k {
                // let / var
                0..=17 => {
                    let t = if self.r.chance(15) { T::I } else { *self.r.pick(&TRACED) };
                    let mutable = self.r.chance(30);
                    let e = self.tex(fx, t);
                    let name = self.fresh("x");
                    self.declare(fx, &name, t, mutable);
                    Some(format!("{}{} {}: {} = {};\n", ind, if mutable { "var" } else { "let" }, name, tname(t), e))
                }
                // fold into the accumulator
                18..=26 => {
                    let t = *self.r.pick(&TRACED);
                    let e = self.tex(fx, t);
                    Some(format!("{}{} = mix({}, {});\n", ind, fx.acc, fx.acc, digest(t, &e)))
                }
                27..=32 if !fx.in_defer || true => Some(format!("{}{} += gc(c);\n", ind, fx.acc)),
                // assignment to a variable
                33..=37 => {
                    let t = *self.r.pick(&TRACED);
                    let vs: Vec<String> = fx.assignable(t).iter().map(|v| v.name.clone()).collect();
                    if vs.is_empty() {
                        None
                    } else {
                        let v = self.r.pick(&vs).clone();
                        let e = self.tex(fx, t);
                        Some(format!("{}{} = {};\n", ind, v, e))
                    }
                }
                // stores into objects, the context, vm state
                38..=41 if !fx.pure => match self.r.below(4) {
                    0 | 1 => {
                        let n = self.ex(fx, T::N, 1);
                        let o = self.ex(fx, T::O, 1);
                        Some(format!("{}{}.{} = {};\n", ind, n, if self.r.chance(50) { "l" } else { "r" }, o))
                    }
                    2 => {
                        let o = self.ex(fx, T::O, 1);
                        Some(format!("{}c.keep = {};\n", ind, o))
                    }
                    _ if fx.vm() => {
                        if self.r.chance(50) {
                            let o = self.tex(fx, T::O);
                            Some(format!("{}cur = {};\n", ind, o))
                        } else {
                            let v = self.tex(fx, T::V);
                            Some(format!("{}tos = {};\n", ind, v))
                        }
                    }
                    _ => None,
                },
                42..=47 if !deep => {
                    let c = self.cond(fx);
                    let a = self.block_n(fx, 1, 3);
                    let b = self.block_n(fx, 0, 3);
                    Some(format!("{}if {} {{\n{}{}}} else {{\n{}{}}}\n", ind, c, a, ind, b, ind))
                }
                // if let (chains)
                48..=53 if !deep => {
                    fx.scopes.push(vec![]);
                    let (p1, e1) = self.refutable(fx, true);
                    let mut head = format!("if let {} = {}", p1, e1);
                    if self.r.chance(35) {
                        let c = self.cond(fx);
                        head.push_str(&format!(" and {}", c));
                    }
                    if self.r.chance(30) {
                        let (p2, e2) = self.refutable(fx, false);
                        head.push_str(&format!(" and let {} = {}", p2, e2));
                    }
                    fx.depth += 1;
                    let binds: Vec<(String, T)> = fx.scopes.last().unwrap().iter().map(|v| (v.name.clone(), v.t)).collect();
                    let mut a = self.use_binds(fx, &binds);
                    a.push_str(&self.stmts_n(fx, 1, 3));
                    fx.depth -= 1;
                    fx.scopes.pop();
                    let b = self.block_n(fx, 0, 2);
                    Some(format!("{}{} {{\n{}{}}} else {{\n{}{}}}\n", ind, head, a, ind, b, ind))
                }
                // let .. else
                54..=61 => {
                    if self.diverge(fx).is_none() {
                        continue;
                    }
                    // In a handler, sometimes `else cold { .. }`: no
                    // assignments to the handler's locals there, no loops
                    // to leave.
                    let cold = fx.kind == Kind::Handler && self.r.chance(30);
                    let save = (fx.kind, fx.own_from, fx.loops.clone());
                    if cold {
                        fx.kind = Kind::Cold;
                        fx.own_from = fx.scopes.len() + 1;
                        fx.loops.clear();
                    }
                    fx.scopes.push(vec![]);
                    fx.depth += 1;
                    let s = self.stmts_n(fx, 0, 3);
                    let dv = self.diverge(fx);
                    let ind2 = Self::ind(fx);
                    fx.depth -= 1;
                    fx.scopes.pop();
                    (fx.kind, fx.own_from, fx.loops) = save;
                    let Some(dv) = dv else { continue };
                    let n = fx.scopes.last().unwrap().len();
                    let (p, e) = self.refutable(fx, true);
                    let binds: Vec<(String, T)> = fx.scopes.last().unwrap()[n..].iter().map(|v| (v.name.clone(), v.t)).collect();
                    let u = self.use_binds(fx, &binds);
                    Some(format!("{}let {} = {} else {}{{\n{}{}{}\n{}}};\n{}", ind, p, e, if cold { "cold " } else { "" }, s, ind2, dv, ind, u))
                }
                // irrefutable destructuring
                62..=63 => {
                    let tu = self.tex(fx, T::Tu);
                    let a = self.fresh("x");
                    let b = self.fresh("x");
                    self.declare(fx, &a, T::N, false);
                    self.declare(fx, &b, T::I, false);
                    Some(format!("{}let ({}, {}) = {};\n", ind, a, b, tu))
                }
                64..=71 if !deep => Some(self.match_stmt(fx)),
                // while let
                72..=75 if !deep => {
                    let w = self.fresh("w");
                    let s = self.fresh("s");
                    let o = self.tex(fx, T::O);
                    let x = self.fresh("x");
                    fx.scopes.push(vec![]);
                    self.declare(fx, &x, T::N, false);
                    fx.loops.push(Lp::Plain);
                    fx.depth += 1;
                    let body = self.stmts_n(fx, 1, 3);
                    let ind2 = Self::ind(fx);
                    fx.depth -= 1;
                    fx.loops.pop();
                    fx.scopes.pop();
                    Some(format!(
                        "{ind}var {w}: ?Gc<Node> = {o};\n{ind}var {s}: i64 = 0;\n{ind}while let Some({x}) = {w} {{\n{ind2}{s} += 1;\n{ind2}if {s} > 3 {{ break; }}\n{ind2}{w} = {x}.l;\n{body}{ind}}}\n"
                    ))
                }
                // for
                76..=79 if !deep => {
                    let i = self.fresh("i");
                    let n = 1 + self.r.below(3);
                    fx.scopes.push(vec![]);
                    self.declare(fx, &i, T::I, false);
                    fx.loops.push(Lp::Plain);
                    fx.depth += 1;
                    let body = self.stmts_n(fx, 1, 3);
                    fx.depth -= 1;
                    fx.loops.pop();
                    fx.scopes.pop();
                    Some(format!("{}for {} in 0..{} {{\n{}{}}}\n", ind, i, n, body, ind))
                }
                // loop with a break value
                80..=82 if !deep => {
                    let t = *self.r.pick(&TRACED);
                    let s = self.fresh("s");
                    let x = self.fresh("x");
                    fx.scopes.push(vec![]);
                    fx.loops.push(Lp::Val(t));
                    fx.depth += 1;
                    let ind2 = Self::ind(fx);
                    let first = self.tex(fx, t);
                    let body = self.stmts_n(fx, 1, 3);
                    let c = self.cond(fx);
                    let last = self.tex(fx, t);
                    fx.depth -= 1;
                    fx.loops.pop();
                    fx.scopes.pop();
                    self.declare(fx, &x, t, false);
                    Some(format!(
                        "{ind}var {s}: i64 = 0;\n{ind}let {x}: {tn} = loop {{\n{ind2}{s} += 1;\n{ind2}if {s} > 2 {{ break {first}; }}\n{body}{ind2}if {c} {{ break {last}; }}\n{ind}}};\n",
                        tn = tname(t)
                    ))
                }
                // defer
                83..=85 if !fx.in_defer && fx.kind != Kind::Cold => {
                    let save = fx.in_defer;
                    fx.in_defer = true;
                    let t = *self.r.pick(&TRACED);
                    let e = self.ex(fx, t, 1);
                    let g = if self.r.chance(60) { " gc(c);" } else { "" };
                    fx.in_defer = save;
                    Some(format!("{}defer {{ c.sum = mix(c.sum, {});{} }}\n", ind, digest(t, &e), g))
                }
                // early exit
                86..=88 => {
                    let c = self.cond(fx);
                    let Some(d) = self.diverge(fx) else { continue };
                    Some(format!("{}if {} {{ {} }}\n", ind, c, d))
                }
                // a nested block
                89..=90 if !deep => {
                    let b = self.block_n(fx, 1, 3);
                    Some(format!("{}{{\n{}{}}}\n", ind, b, ind))
                }
                // a probe: an object that only the block's locals reach must die
                91..=94 if !deep && fx.probe.is_none() && !fx.in_defer => {
                    let w = self.fresh("wk");
                    let t = self.fresh("t");
                    let save = (fx.pure, fx.probe);
                    fx.pure = true;
                    fx.scopes.push(vec![]);
                    fx.probe = Some(fx.scopes.len() - 1);
                    let v = self.lit();
                    self.declare(fx, &t, T::N, false);
                    fx.depth += 1;
                    let body = self.stmts_n(fx, 1, 4);
                    let ind2 = Self::ind(fx);
                    fx.depth -= 1;
                    fx.scopes.pop();
                    (fx.pure, fx.probe) = save;
                    Some(format!(
                        "{ind}var {w}: ?Gc<gc.Weak<Node>> = None;\n{ind}{{\n{ind2}let {t} = mk(c, {v}, None, None);\n{body}{ind2}{w} = Some(c.heap.new_weak({t}));\n{ind}}}\n{ind}{acc} += dead(c, {w});\n",
                        acc = fx.acc
                    ))
                }
                // a cold block in a handler
                95..=97 if fx.kind == Kind::Handler && !deep && fx.loops.is_empty() && !fx.in_defer => {
                    let c = self.cond(fx);
                    let save = (fx.kind, fx.own_from);
                    fx.kind = Kind::Cold;
                    fx.own_from = fx.scopes.len() + 1;
                    let b = self.block_n(fx, 1, 3);
                    (fx.kind, fx.own_from) = save;
                    let ind2 = "    ".repeat(fx.depth + 2);
                    Some(format!("{}if {} {{ cold {{\n{}{}next\n{}}} }}\n", ind, c, b, ind2, ind))
                }
                _ => None,
            };
            if let Some(s) = s {
                return s;
            }
        }
    }

    /// A refutable pattern binding fresh variables (declared in the current
    /// scope) and a scrutinee. The scrutinee may collect when `may_collect`.
    fn refutable(&mut self, fx: &mut Fx, may_collect: bool) -> (String, String) {
        let x = self.fresh("x");
        let y = self.fresh("x");
        let kind = self.r.below(7);
        let (pat, t, binds): (String, T, Vec<(String, T)>) = match kind {
            0 | 1 => (format!("Some({})", x), T::O, vec![(x.clone(), T::N)]),
            2 => (format!("E.One({})", x), T::E, vec![(x.clone(), T::N)]),
            3 => (format!("E.Two({}, {})", x, y), T::E, vec![(x.clone(), T::N), (y.clone(), T::N)]),
            4 => (format!("E.Pr({})", x), T::E, vec![(x.clone(), T::P)]),
            5 => (format!("R({})", x), T::V, vec![(x.clone(), T::N)]),
            _ => {
                // A tuple: an enum (which may collect) and an option.
                let e = if may_collect { self.tex(fx, T::E) } else { self.ex(fx, T::E, 2) };
                let o = self.ex(fx, T::O, 1);
                self.declare(fx, &x, T::N, false);
                self.declare(fx, &y, T::N, false);
                return (format!("(E.One({}), Some({}))", x, y), format!("({}, {})", e, o));
            }
        };
        let e = if may_collect { self.tex(fx, t) } else { self.ex(fx, t, 2) };
        for (n, t) in binds {
            self.declare(fx, &n, t, false);
        }
        (pat, e)
    }

    /// A scrutinee: often a fresh value (a call's result, a new object),
    /// which only the bindings keep alive.
    fn scrut(&mut self, fx: &Fx, t: T) -> String {
        match self.r.below(3) {
            0 => self.cex(fx, t).unwrap_or_else(|| self.ex(fx, t, 2)),
            1 => self.ex(&Fx { scopes: vec![], ..fx.clone() }, t, 2),
            _ => self.tex(fx, t),
        }
    }

    /// Often: a collection, then a read through the bindings.
    fn use_binds(&mut self, fx: &Fx, binds: &[(String, T)]) -> String {
        if binds.is_empty() || !self.r.chance(60) {
            return String::new();
        }
        let ind = "    ".repeat(fx.depth + 2);
        let mut s = String::new();
        if !fx.in_defer {
            s.push_str(&format!("{}{} += gc(c);\n", ind, fx.acc));
        }
        for (b, t) in binds {
            s.push_str(&format!("{}{} = mix({}, {});\n", ind, fx.acc, fx.acc, digest(*t, b)));
        }
        s
    }

    fn match_stmt(&mut self, fx: &mut Fx) -> String {
        let ind = Self::ind(fx);
        let ind2 = "    ".repeat(fx.depth + 2);
        // Scrutinee: an enum, an option, a value, or a tuple of an enum and an option.
        let st = self.r.below(4);
        let (scrut, arms): (String, Vec<(String, Vec<(String, T)>)>) = match st {
            0 => {
                let e = self.scrut(fx, T::E);
                let (a, b) = (self.fresh("x"), self.fresh("x"));
                (
                    e,
                    vec![
                        (format!("E.One({})", a), vec![(a.clone(), T::N)]),
                        (format!("E.Two({}, {})", a, b), vec![(a.clone(), T::N), (b.clone(), T::N)]),
                        (format!("E.Two(_, {})", b), vec![(b.clone(), T::N)]),
                        (format!("E.Pr({})", a), vec![(a.clone(), T::P)]),
                        ("E.Nil | E.One(_)".into(), vec![]),
                        ("E.Nil".into(), vec![]),
                    ],
                )
            }
            1 => {
                let e = self.scrut(fx, T::O);
                let a = self.fresh("x");
                (e, vec![(format!("Some({})", a), vec![(a.clone(), T::N)]), ("None".into(), vec![])])
            }
            2 => {
                let e = self.scrut(fx, T::V);
                let a = self.fresh("x");
                (e, vec![(format!("R({})", a), vec![(a.clone(), T::N)]), ("Num(_) | Nil".into(), vec![]), ("Nil".into(), vec![])])
            }
            _ => {
                let e = self.scrut(fx, T::E);
                let o = self.ex(fx, T::O, 1);
                let (a, b) = (self.fresh("x"), self.fresh("x"));
                (
                    format!("({}, {})", e, o),
                    vec![
                        (format!("(E.One({}), Some({}))", a, b), vec![(a.clone(), T::N), (b.clone(), T::N)]),
                        (format!("(E.Two({}, _), _)", a), vec![(a.clone(), T::N)]),
                        (format!("(_, Some({}))", b), vec![(b.clone(), T::N)]),
                        (format!("(E.Pr({}), None)", a), vec![(a.clone(), T::P)]),
                    ],
                )
            }
        };
        let value = self.r.chance(40);
        let mut out = String::new();
        let n = 1 + self.r.below(arms.len() as u64) as usize;
        let mut chosen: Vec<usize> = (0..arms.len()).collect();
        // A random subset of the arms, in random order, then `_`.
        for i in (1..chosen.len()).rev() {
            let j = self.r.below(i as u64 + 1) as usize;
            chosen.swap(i, j);
        }
        chosen.truncate(n);
        // Often first: an arm whose guard collects and fails, before the
        // arms that bind from the scrutinee.
        if !fx.in_defer && self.r.chance(30) {
            let tail = if value { format!(" {}", self.lit()) } else { String::new() };
            out.push_str(&format!("{}_ if gcf(c) => {{{} }}\n", ind2, tail));
        }
        for &ai in &chosen {
            let (pat, binds) = &arms[ai];
            fx.scopes.push(vec![]);
            for (b, t) in binds {
                self.declare(fx, b, *t, false);
            }
            let guard = match self.r.below(4) {
                0 if !fx.in_defer => (if self.r.chance(50) { " if gcf(c)" } else { " if gcb(c)" }).to_string(),
                1 => format!(" if {}", self.cond(fx)),
                2 if !binds.is_empty() && binds[0].1 == T::N && !fx.in_defer => format!(" if {}.v % 2 == 0 and gcb(c)", binds[0].0),
                _ => String::new(),
            };
            fx.depth += 1;
            let mut body = self.stmts_n(fx, 0, 3);
            body.push_str(&self.use_binds(fx, binds));
            let tail = if value { format!("{}{}\n", ind2, self.ex(fx, T::I, 1)) } else { String::new() };
            fx.depth -= 1;
            fx.scopes.pop();
            out.push_str(&format!("{}{}{} => {{\n{}{}{}}}\n", ind2, pat, guard, body, tail, ind2));
        }
        fx.depth += 1;
        let body = self.block_n(fx, 0, 2);
        let tail = if value { format!("{}{}\n", ind2, self.lit()) } else { String::new() };
        fx.depth -= 1;
        out.push_str(&format!("{}_ => {{\n{}{}{}}}\n", ind2, body, tail, ind2));
        if value {
            format!("{ind}{acc} = mix({acc}, match {scrut} {{\n{out}{ind}}});\n", acc = fx.acc)
        } else {
            format!("{ind}match {scrut} {{\n{out}{ind}}}\n")
        }
    }

    // ── functions and the vm ────────────────────────────────────────

    fn base_fx(&self, kind: Kind, index: usize, ret: T, pure: bool, kname: &str) -> Fx {
        Fx {
            kind,
            ret,
            pure,
            scopes: vec![vec![]],
            loops: vec![],
            probe: None,
            index,
            acc: "acc".into(),
            kname: kname.into(),
            depth: 0,
            own_from: 0,
            in_defer: false,
        }
    }

    fn function(&mut self, j: usize) -> String {
        let ret = self.rets[j];
        let pure = self.pure_fns[j];
        let mut fx = self.base_fx(Kind::Plain, j, ret, pure, "k");
        for (n, t, m) in [("k", T::I, false), ("a", T::N, false), ("b", T::O, false), ("acc", T::I, true)] {
            fx.scopes[0].push(Var { name: n.into(), t, mutable: m, depth: 0 });
        }
        let n = 4 + self.r.below(8) as usize;
        let body = self.stmts(&mut fx, n);
        let tail = match ret {
            T::I => format!("mix(acc, {})", {
                let t = *self.r.pick(&TRACED);
                let e = self.ex(&fx, t, 2);
                digest(t, &e)
            }),
            t => self.ex(&fx, t, 2),
        };
        let dflt = match ret {
            T::I => "k",
            T::N => "a",
            T::E => "E.Nil",
            _ => "V.Nil",
        };
        format!(
            "fn f{j}(c: *Ctx, k: i64, a: Gc<Node>, b: ?Gc<Node>) -> {rt} uses gc {{\n    if c.fuel <= 0 {{ return {dflt}; }}\n    c.fuel -= 1;\n    var acc: i64 = k;\n{body}    {tail}\n}}\n\n",
            rt = tname(ret)
        )
    }

    fn vm_fn(&mut self, j: usize) -> String {
        let mut fx = self.base_fx(Kind::VmFn, j, T::I, false, "k");
        fx.index = j;
        fx.scopes[0].push(Var { name: "k".into(), t: T::I, mutable: false, depth: 0 });
        fx.scopes[0].push(Var { name: "a".into(), t: T::N, mutable: false, depth: 0 });
        fx.depth = 1;
        let body = self.stmts_n(&mut fx, 2, 5);
        let tail = {
            let t = *self.r.pick(&TRACED);
            let e = self.ex(&fx, t, 2);
            digest(t, &e)
        };
        format!("    fn vf{j}(k: i64, a: Gc<Node>) -> i64 {{\n{body}        mix(acc, {tail})\n    }}\n")
    }

    fn handler(&mut self, j: usize) -> String {
        let mut fx = self.base_fx(Kind::Handler, usize::MAX, T::I, false, "kk");
        fx.scopes[0].push(Var { name: "kk".into(), t: T::I, mutable: false, depth: 0 });
        fx.depth = 1;
        let body = self.stmts_n(&mut fx, 2, 6);
        format!("    op OP{j}(k: imm) : N {{\n        let kk: i64 = k as i64;\n{body}        next\n    }}\n")
    }

    fn on_raise(&mut self) -> String {
        let mut fx = self.base_fx(Kind::Raise, usize::MAX, T::I, false, "e");
        fx.scopes[0].push(Var { name: "e".into(), t: T::I, mutable: false, depth: 0 });
        fx.depth = 1;
        let body = self.stmts_n(&mut fx, 1, 4);
        format!("    on raise(e: i64) {{\n{body}        exit (acc + e) % MD\n    }}\n")
    }

    pub fn program(&mut self) -> String {
        self.nfns = 4 + self.r.below(6) as usize;
        self.rets = (0..self.nfns).map(|_| *self.r.pick(&RET)).collect();
        self.pure_fns = (0..self.nfns).map(|_| self.r.chance(40)).collect();
        self.nvmfns = 1 + self.r.below(3) as usize;
        self.nops = 2 + self.r.below(5) as usize;
        let mut out = String::from(PRELUDE);
        out.push('\n');
        // Functions in reverse order of index, callers last (any order works).
        for j in 0..self.nfns {
            let f = self.function(j);
            out.push_str(&f);
        }
        out.push_str("vm M {\n    context c: *Ctx;\n    state { pc: *u32, acc: i64, cur: ?Gc<Node>, tos: V }\n    encoding fixed(u32) { format N { op: 8, x: 24 } }\n\n");
        for j in 0..self.nvmfns {
            let f = self.vm_fn(j);
            out.push_str(&f);
        }
        for j in 0..self.nops {
            let h = self.handler(j);
            out.push_str(&h);
        }
        out.push_str("    op HALT() : N { exit (acc + dig(cur, 3) + dv(tos)) % MD }\n\n    operand imm = it;\n\n");
        let r = self.on_raise();
        out.push_str(&r);
        out.push_str("\n    entry run(cx: *Ctx, code: *u32) -> i64 uses gc { c = cx; pc = code; acc = 0; cur = None; tos = V.Nil; jump code }\n}\n\n");
        // main
        out.push_str("fn main() uses gc {\n    let mode = os.arg_count();\n    var c = Ctx { heap: gc.Heap.new(), keep: None, sum: 0, fuel: 0, leaks: 0, gen: false, gcs: 0 };\n    c.heap.add_scanner(&c as *u8, scan);\n    if mode == 2 { c.heap.set_stress(true); }\n    if mode == 3 { c.heap.set_generational(true); c.heap.set_stress(true); c.heap.set_verify(true); c.gen = true; }\n");
        for j in 0..self.nfns {
            for k in 0..2 {
                let kv = self.r.below(20);
                let dg = digest(self.rets[j], &format!("f{}(&c, {}, mk(&c, {}, None, None), {})", j, kv, k + 1, if k == 0 { "None" } else { "Some(mk(&c, 4, None, None))" }));
                out.push_str(&format!("    c.fuel = 40;\n    io.print_int({});\n    io.println(\" f{}\");\n", dg, j));
            }
        }
        for p in 0..3 {
            let len = 2 + self.r.below(6) as usize;
            let mut code: Vec<String> = (0..len).map(|_| format!("M.OP{}({})", self.r.below(self.nops as u64), self.r.below(20))).collect();
            code.push("M.HALT()".into());
            out.push_str(&format!(
                "    {{\n        var code: [u32; {}] = [{}];\n        for i in 0..2 {{\n            c.fuel = 40;\n            io.print_int(M.run(&c, &code[0]));\n            io.println(\" vm{}\");\n        }}\n    }}\n",
                code.len(),
                code.join(", "),
                p
            ));
        }
        out.push_str("    io.print_int(c.sum);\n    io.println(\" sum\");\n    io.print(\"leaks \");\n    io.print_int(if c.gen { 0 } else { c.leaks });\n    io.println(\"\");\n    c.heap.free();\n}\n");
        out
    }
}

/// The program for `seed`.
pub fn generate(seed: u64) -> String {
    Gen::new(seed).program()
}
