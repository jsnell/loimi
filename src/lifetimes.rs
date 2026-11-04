//! Pointer stability (docs/design/lifetimes.md): a pointer or slice into
//! a container must not be used after a call that may move or free the
//! memory it points into, and the address of a local must not outlive it.
//! A check only: it never changes the typed tree, so no code changes.
//!
//! **Model.** Every pointer-like value is abstracted by the set of
//! *paths* it may point into: a root (a local of the function, a global,
//! a GC object held by a `let`, any other GC object) followed by field
//! steps and dereferences. `v.items()` points into `v.ptr.*`, `&x` into
//! `x`, `sm.file(i).text` into `sm.*.files.ptr.*.text.*`. A *restructure*
//! of path R (a call of `free`/`realloc`, or of a function that may make
//! one on memory reached through its parameters) invalidates every value
//! pointing into R itself, at any field offset, but not memory reached
//! through pointers *stored* in R (`realloc` of a `Vec<str>`'s buffer
//! does not free the strings' bytes).
//!
//! Per function, over the whole call graph (a least fixpoint, as
//! src/objref.rs does for slices of objects), a *summary*:
//!
//! - `restructs`: the paths, rooted at its parameters (or globals), that
//!   it may move or free (`Vec.push`: `v.*.ptr.*`);
//! - `ret`: the parameter paths its result may point into (`Vec.items`:
//!   `v.*.ptr.*`; a result pointing nowhere known is a fresh value);
//! - `pairs`: (i, j) when it uses argument j after moving or freeing
//!   memory through argument i, so a call must not pass in j a pointer
//!   into what it restructures through i (`b.push_str(b.as_str())`),
//!   unless `#[may_alias(j)]` says the function copes (`Vec.extend`);
//! - `keeps`: the parameters whose pointee it may store in memory that
//!   outlives the call (`Vec.push`'s `x`, `SourceMap.add`'s `text`).
//!
//! A call that may collect (`uses gc`, also `reenter`) invalidates
//! pointers into GC objects other than those held by a `let` local or
//! parameter (which the compiler roots while in scope: the collector does
//! not move objects). In vm code, a `reenter` call invalidates pointers
//! derived from the state that the vm's `reload` block recomputes.
//!
//! Errors are reported in modules with `#![safe]` or `#![lifetimes]`,
//! outside `unsafe { }`; `LOIMI_LIFETIMES=audit` lists the sites of every
//! module instead (`lifetime-site` lines on stderr), without failing.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::diag::{Diag, Res, Span};
use crate::tir::*;

/// Longest path kept; longer ones are cut and marked `trunc` (anything
/// below).
const K: usize = 8;
/// The trailing array of a GC object, as a field step.
const TRAIL: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
enum Root {
    Local(LocalId),
    /// Parameter i (summaries only).
    Param(u32),
    Global(u32),
    /// A GC object held by an immutable local (rooted while in scope).
    GcLocal(LocalId),
    /// Any other GC object.
    GcAny,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
enum Step {
    F(u32),
    /// Through a pointer.
    D,
    /// Through some field of an aggregate, then its pointer (a slice's
    /// buffer, the buffers a `Vec` value owns).
    AnyD,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
struct Path {
    root: Root,
    steps: Vec<Step>,
    trunc: bool,
}

impl Path {
    fn new(root: Root) -> Path {
        Path { root, steps: Vec::new(), trunc: false }
    }
    fn push(&self, s: Step) -> Path {
        let mut p = self.clone();
        if p.steps.len() >= K {
            p.trunc = true;
        } else if !p.trunc {
            p.steps.push(s);
        }
        p
    }
    fn has_deref(&self) -> bool {
        self.steps.iter().any(|s| matches!(s, Step::D | Step::AnyD))
    }
}

type Val = BTreeSet<Path>;

fn is_d(s: &Step) -> bool {
    matches!(s, Step::D | Step::AnyD)
}

/// Whether moving or freeing the memory at `r` invalidates a pointer
/// into `p`: `r` is a prefix of `p` and the rest of `p` stays in the same
/// block (no dereference). `AnyD` matches any fields and a dereference.
fn hits(r: &Path, p: &Path) -> bool {
    if r.root == Root::GcAny && r.steps.is_empty() && r.trunc {
        // A collection: pointers into objects not held by a local.
        return p.root == Root::GcAny && !p.steps.is_empty();
    }
    r.root == p.root && m(&r.steps, &p.steps, r.trunc, p.trunc)
}

fn m(r: &[Step], p: &[Step], rt: bool, pt: bool) -> bool {
    if r.is_empty() {
        // Both cut at the same depth: deep in some pointer chain; taken to
        // be different blocks (the approximation of the depth bound).
        if p.is_empty() && rt && pt {
            return false;
        }
        return !p.iter().any(is_d);
    }
    if p.is_empty() {
        // `p` was cut: it stands for something below. A restructure of a
        // block reached through further pointers is taken to be another
        // block (an approximation for long pointer chains: `f = f.parent`).
        return pt && !r.iter().any(is_d);
    }
    match (r[0], p[0]) {
        (Step::F(a), Step::F(b)) => a == b && m(&r[1..], &p[1..], rt, pt),
        (Step::D | Step::AnyD, Step::D | Step::AnyD) => m(&r[1..], &p[1..], rt, pt),
        (Step::AnyD, Step::F(_)) => m(r, &p[1..], rt, pt),
        (Step::F(_), Step::AnyD) => m(&r[1..], p, rt, pt),
        _ => false,
    }
}

/// How a type holds pointers.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    None,
    Ptr,
    /// A GC reference (also a `value` type with reference payloads).
    Gc,
    /// A slice or an aggregate holding pointers.
    Agg,
}

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Summary {
    restructs: BTreeSet<Path>,
    ret: BTreeSet<Path>,
    pairs: BTreeSet<(u32, u32)>,
    /// Parameter j -> where it may be stored: parameter i's memory, or
    /// `ELSEWHERE` (a global, a GC object, memory of unknown origin).
    keeps: BTreeMap<u32, BTreeSet<u32>>,
}

const ELSEWHERE: u32 = u32::MAX;
/// Memory owned by a local of the function itself (its buffers, its
/// arena's allocations).
const FRAME: u32 = u32::MAX - 1;

#[derive(Clone, Default, PartialEq)]
struct St {
    reach: bool,
    /// What each local may point into.
    vals: BTreeMap<LocalId, Val>,
    /// Locals that may dangle, and why (an index into `Fa::whys`).
    stale: BTreeMap<LocalId, u32>,
    /// Locals pointing into memory of parameter j after a restructure
    /// through parameter i: (i, j) (summary `pairs` if used).
    cst: BTreeMap<LocalId, BTreeSet<(u32, u32)>>,
    /// (local, place): the local holds a copy of the owning value at
    /// `place` (memory), so freeing through it leaves `place` dangling.
    aliases: BTreeSet<(LocalId, Path)>,
    /// Places in memory whose owned buffers were freed through a copy.
    stale_places: BTreeMap<Path, u32>,
    /// Pending escapes: (place, stored pointer, why): the address of a
    /// local (or a parameter's pointee) stored in memory that outlives
    /// the call, unless overwritten before it returns.
    esc: BTreeSet<(Path, Path, u32)>,
}

impl St {
    fn join(&mut self, o: &St) {
        if !o.reach {
            return;
        }
        if !self.reach {
            *self = o.clone();
            return;
        }
        for (l, v) in &o.vals {
            self.vals.entry(*l).or_default().extend(v.iter().cloned());
        }
        for (l, w) in &o.stale {
            self.stale.entry(*l).or_insert(*w);
        }
        for (l, v) in &o.cst {
            self.cst.entry(*l).or_default().extend(v.iter().cloned());
        }
        self.aliases.extend(o.aliases.iter().cloned());
        for (p, w) in &o.stale_places {
            self.stale_places.entry(p.clone()).or_insert(*w);
        }
        self.esc.extend(o.esc.iter().cloned());
    }

    /// Locals whose scope ended: nothing reads them again before they
    /// are bound anew, so their entries only cost (cloned at every branch).
    fn drop_locals(&mut self, ls: &[LocalId]) {
        for l in ls {
            self.vals.remove(l);
            self.stale.remove(l);
            self.cst.remove(l);
        }
    }
}

fn pat_locals(p: &TPat, out: &mut Vec<LocalId>) {
    match &p.kind {
        TPK::Bind(l) => out.push(*l),
        TPK::Variant(_, ps) | TPK::Tuple(ps) | TPK::Or(ps) => ps.iter().for_each(|q| pat_locals(q, out)),
        _ => {}
    }
}

/// The locals bound by `let`s and patterns anywhere in `e`.
fn bound_locals(e: &TExpr, out: &mut Vec<LocalId>) {
    match &e.kind {
        TK::Block(ss, _) => {
            for s in ss {
                match s {
                    TStmt::Let(l, _) => out.push(*l),
                    TStmt::LetPat(p, ..) => pat_locals(p, out),
                    _ => {}
                }
            }
        }
        TK::If(cs, ..) => {
            for c in cs {
                if let TCond::Let(p, _) = c {
                    pat_locals(p, out);
                }
            }
        }
        TK::Match(_, arms) => arms.iter().for_each(|a| pat_locals(&a.pat, out)),
        _ => {}
    }
    walk_kids(e, &mut |k| bound_locals(k, out));
}

struct Fa<'a> {
    p: &'a Program,
    fid: FnId,
    f: &'a FnDef,
    sums: &'a HashMap<FnId, Summary>,
    report: bool,
    unsafe_spans: &'a [Span],
    audit: Option<&'a crate::diag::SourceMap>,
    kinds: &'a mut HashMap<Ty, Kind>,
    whys: Vec<(Span, String)>,
    errs: Vec<Diag>,
    err_spans: HashSet<(u32, u32)>,
    breaks: HashMap<u32, (St, Val)>,
    conts: HashMap<u32, St>,
    /// Per loop: the locals bound inside its body (dropped when it exits).
    loop_locals: HashMap<u32, Vec<LocalId>>,
    defers: Vec<Vec<&'a TExpr>>,
    out: Summary,
    param_idx: HashMap<LocalId, u32>,
}

fn kind_of(p: &Program, t: &Ty, memo: &mut HashMap<Ty, Kind>, depth: u32) -> Kind {
    if let Some(k) = memo.get(t) {
        return *k;
    }
    if depth > 12 {
        return Kind::Agg;
    }
    let agg = |ts: &mut dyn Iterator<Item = &Ty>, memo: &mut HashMap<Ty, Kind>| {
        for x in ts {
            if kind_of(p, x, memo, depth + 1) != Kind::None {
                return Kind::Agg;
            }
        }
        Kind::None
    };
    let k = match t {
        Ty::Ptr(_) => Kind::Ptr,
        Ty::Gc(_) => Kind::Gc,
        Ty::Value(i) => {
            if p.values[*i as usize].variants.iter().any(|v| v.payload.as_ref().is_some_and(|t| matches!(t, Ty::Gc(_)))) {
                Kind::Gc
            } else {
                Kind::None
            }
        }
        Ty::Slice(_) | Ty::Closure(..) | Ty::Block(..) => Kind::Agg,
        Ty::Option(i) | Ty::Array(i, _) => match kind_of(p, i, memo, depth + 1) {
            Kind::None => Kind::None,
            Kind::Ptr if matches!(t, Ty::Option(_)) => Kind::Ptr,
            Kind::Gc => Kind::Gc,
            _ => Kind::Agg,
        },
        Ty::Result(a, b) => agg(&mut [&**a, &**b].into_iter(), memo),
        Ty::Tuple(ts) => agg(&mut ts.iter(), memo),
        Ty::Struct(s) => {
            let sd = &p.structs[*s as usize];
            if sd.object.is_some() {
                Kind::None
            } else {
                memo.insert(t.clone(), Kind::None);
                agg(&mut sd.fields.iter().map(|(_, t)| t), memo)
            }
        }
        Ty::Enum(e) => agg(&mut p.enums[*e as usize].variants.iter().flat_map(|v| v.fields.iter()), memo),
        _ => Kind::None,
    };
    memo.insert(t.clone(), k);
    k
}

impl<'a> Fa<'a> {
    fn kind(&mut self, t: &Ty) -> Kind {
        kind_of(self.p, t, self.kinds, 0)
    }

    fn in_unsafe(&self, s: Span) -> bool {
        self.unsafe_spans.iter().any(|u| u.file == s.file && u.lo <= s.lo && s.hi <= u.hi)
    }

    fn why(&mut self, span: Span, what: String) -> u32 {
        self.whys.push((span, what));
        (self.whys.len() - 1) as u32
    }

    fn error(&mut self, span: Span, msg: String, notes: Vec<(Span, String)>) {
        if !self.report || self.in_unsafe(span) || !self.err_spans.insert((span.file, span.lo)) {
            return;
        }
        if let Some(sm) = self.audit {
            let (l, c) = sm.line_col(span.file, span.lo);
            eprintln!("lifetime-site {}:{}:{} {} ({})", sm.files[span.file as usize].path, l, c, msg, self.f.symbol);
            return;
        }
        let mut d = Diag::new(span, msg);
        for (s, n) in notes {
            d = d.note(s, n);
        }
        self.errs.push(d);
    }

    fn local_name(&self, l: LocalId) -> String {
        let n = &self.f.locals[l as usize].name;
        if n.starts_with("tmp") { "the value being iterated over".into() } else { format!("`{}`", n) }
    }

    /// What a local points into before it is assigned (parameters, vm
    /// state): its own pointee.
    fn symbolic(&mut self, l: LocalId) -> Val {
        let t = self.f.locals[l as usize].ty.clone();
        let mut v = Val::new();
        match self.kind(&t) {
            Kind::Ptr => {
                v.insert(Path::new(Root::Local(l)).push(Step::D));
            }
            Kind::Agg => {
                v.insert(Path::new(Root::Local(l)).push(Step::AnyD));
            }
            _ => {}
        }
        v
    }

    fn use_local(&mut self, l: LocalId, span: Span, st: &St) {
        if let Some(&w) = st.stale.get(&l) {
            let (ws, what) = self.whys[w as usize].clone();
            let msg = format!("{} may dangle: it points into memory that {} may have moved or freed", self.local_name(l), what);
            self.error(
                span,
                msg,
                vec![(ws, format!("{} may move or free it here; take the pointer or slice again after this call, or keep an index", what))],
            );
        }
        if let Some(ps) = st.cst.get(&l) {
            self.out.pairs.extend(ps.iter().cloned());
        }
    }

    fn read_local(&mut self, l: LocalId, span: Span, st: &St) -> Val {
        self.use_local(l, span, st);
        let ld = &self.f.locals[l as usize];
        let t = ld.ty.clone();
        let mutable = ld.mutable;
        match self.kind(&t) {
            Kind::None => Val::new(),
            Kind::Gc if !mutable => [Path::new(Root::GcLocal(l))].into_iter().collect(),
            k => {
                let mut v = st.vals.get(&l).cloned().unwrap_or_else(|| self.symbolic(l));
                // An owning value (a `Vec`) read whole: its buffers.
                if k == Kind::Agg && !matches!(t, Ty::Slice(_)) {
                    v.insert(Path::new(Root::Local(l)).push(Step::AnyD));
                }
                v
            }
        }
    }

    /// The value read from memory at `paths`, of type `t`.
    fn read_mem(&mut self, paths: &[Path], t: &Ty) -> Val {
        match self.kind(t) {
            Kind::None => Val::new(),
            Kind::Ptr => paths.iter().map(|p| p.push(Step::D)).collect(),
            Kind::Agg => paths.iter().map(|p| p.push(Step::AnyD)).collect(),
            Kind::Gc => [Path::new(Root::GcAny)].into_iter().collect(),
        }
    }

    fn check_place_read(&mut self, paths: &[Path], span: Span, st: &St) {
        for (sp, w) in &st.stale_places {
            if paths.iter().any(|p| p.root == sp.root && p.steps.len() >= sp.steps.len() && p.steps[..sp.steps.len()] == sp.steps[..]) {
                let (ws, what) = self.whys[*w as usize].clone();
                self.error(
                    span,
                    format!("this place's buffers may have been freed through a copy of it ({})", what),
                    vec![(ws, "the copy's buffer moves or is freed here; copying a container makes two owners of one buffer".into())],
                );
                return;
            }
        }
    }

    /// The paths of place `e` (Ok), or the value of `e` if it is not a
    /// place (Err).
    fn place(&mut self, e: &'a TExpr, st: &mut St, read: bool) -> Result<Vec<Path>, Val> {
        match &e.kind {
            TK::Local(l) => {
                if read {
                    self.use_local(*l, e.span, st);
                }
                Ok(vec![Path::new(Root::Local(*l))])
            }
            TK::Global(g) => Ok(vec![Path::new(Root::Global(*g))]),
            TK::Deref(x) => Ok(self.eval(x, st).into_iter().collect()),
            TK::Field(b, k) => {
                if matches!(b.ty, Ty::Slice(_)) {
                    return Err(self.eval(b, st));
                }
                match self.place(b, st, read) {
                    Ok(ps) => Ok(ps.iter().map(|p| p.push(Step::F(*k))).collect()),
                    Err(v) => Err(v),
                }
            }
            TK::Index(b, i) => {
                self.eval(i, st);
                if matches!(b.ty, Ty::Array(..)) {
                    self.place(b, st, read)
                } else {
                    Ok(self.eval(b, st).into_iter().collect())
                }
            }
            _ => Err(self.eval(e, st)),
        }
    }

    /// Move or free the memory at `r`: what points into it may dangle.
    fn restructure(&mut self, r: &Path, w: u32, st: &mut St) {
        let mut newly = Vec::new();
        for (l, v) in &st.vals {
            if !st.stale.contains_key(l) && v.iter().any(|p| hits(r, p)) {
                newly.push(*l);
            }
        }
        for l in newly {
            st.stale.insert(l, w);
        }
        // Through parameter i: values pointing into parameter j's memory
        // would dangle if j aliased it.
        if let Root::Local(rl) = r.root {
            if let (Some(&i), true) = (self.param_idx.get(&rl), r.has_deref()) {
                for (l, v) in &st.vals {
                    for p in v {
                        if let Root::Local(pl) = p.root {
                            if let Some(&j) = self.param_idx.get(&pl) {
                                if j != i && p.has_deref() {
                                    st.cst.entry(*l).or_default().insert((i, j));
                                }
                            }
                        }
                    }
                }
            }
        }
        let mut places = Vec::new();
        for (l, place) in &st.aliases {
            if hits(r, &Path::new(Root::Local(*l)).push(Step::AnyD)) {
                places.push(place.clone());
            }
        }
        for pl in places {
            st.stale_places.entry(pl).or_insert(w);
        }
        // The summary: restructures through parameters and globals.
        match r.root {
            Root::Local(l) if r.has_deref() => {
                if let Some(&i) = self.param_idx.get(&l) {
                    let mut q = r.clone();
                    q.root = Root::Param(i);
                    self.out.restructs.insert(q);
                }
            }
            Root::Global(_) => {
                self.out.restructs.insert(r.clone());
            }
            _ => {}
        }
    }

    /// A callee's path in terms of the caller: parameter i's memory is
    /// what argument i points into.
    fn subst(&self, p: &Path, args: &[Val]) -> Vec<Path> {
        match p.root {
            Root::Param(i) => {
                let Some(pos) = p.steps.iter().position(is_d) else { return Vec::new() };
                let rest = &p.steps[pos + 1..];
                let Some(a) = args.get(i as usize) else { return Vec::new() };
                a.iter()
                    .map(|t| {
                        let mut q = t.clone();
                        for s in rest {
                            q = q.push(*s);
                        }
                        if p.trunc {
                            q.trunc = true;
                        }
                        q
                    })
                    .collect()
            }
            Root::Global(_) | Root::GcAny => vec![p.clone()],
            _ => Vec::new(),
        }
    }

    /// Whether `p` is the storage of one of this function's locals (`&x`).
    fn storage_local(&self, p: &Path) -> Option<LocalId> {
        match p.root {
            Root::Local(l) if !p.has_deref() && !p.trunc => Some(l),
            _ => None,
        }
    }

    /// Parameter j whose own pointee `p` is (`p` is `x.*` or `x.*.f`).
    fn param_pointee(&self, p: &Path) -> Option<u32> {
        let Root::Local(l) = p.root else { return None };
        let j = *self.param_idx.get(&l)?;
        let first = p.steps.iter().position(is_d)?;
        if p.steps[first + 1..].iter().any(is_d) {
            return None;
        }
        Some(j)
    }

    /// Where memory at path `p` belongs: a parameter's, the frame's, or
    /// elsewhere.
    fn dest(&self, p: &Path) -> u32 {
        match p.root {
            Root::Local(l) => match self.param_idx.get(&l) {
                Some(&i) if p.has_deref() => i,
                _ => FRAME,
            },
            _ => ELSEWHERE,
        }
    }

    /// `v` is stored at places `ps` (memory): record what may escape.
    fn store(&mut self, v: &Val, ps: &[Path], span: Span, st: &mut St) {
        let ps: Vec<Path> = if ps.is_empty() { vec![Path::new(Root::GcAny)] } else { ps.to_vec() };
        for q in v {
            if self.storage_local(q).is_none() && self.param_pointee(q).is_none() {
                continue;
            }
            for pl in &ps {
                if self.dest(pl) != FRAME {
                    let w = self.why(span, String::new());
                    st.esc.insert((pl.clone(), q.clone(), w));
                }
            }
        }
    }

    /// At a return: what is still stored escapes.
    fn exit_escapes(&mut self, st: &St) {
        for (pl, q, w) in st.esc.clone() {
            let d = self.dest(&pl);
            let span = self.whys[w as usize].0;
            self.escape(&q, d, span, "stores {} in memory that outlives the function");
        }
    }

    /// Pointer `q` ends up in memory of kind `d` (a parameter's, or
    /// elsewhere).
    fn escape(&mut self, q: &Path, d: u32, span: Span, how: &str) {
        // `main`'s locals live as long as the program.
        if d == FRAME || self.f.symbol == "main.main" {
            return;
        }
        if let Some(l) = self.storage_local(q) {
            let msg = how.replace("{}", &format!("the address of local {}", self.local_name(l)));
            self.error(span, msg, vec![(self.f.span, "a local's memory is reused once its function returns; allocate it (`new(&arena, ..)`), copy the value, or restore the place before returning".into())]);
        }
        if let Some(j) = self.param_pointee(q) {
            self.out.keeps.entry(j).or_default().insert(d);
        }
    }

    fn assign_local(&mut self, l: LocalId, v: Val, st: &mut St) {
        let t = self.f.locals[l as usize].ty.clone();
        let k = self.kind(&t);
        st.stale.remove(&l);
        st.cst.remove(&l);
        st.aliases.retain(|(x, _)| *x != l);
        if k == Kind::Agg && !matches!(t, Ty::Slice(_)) {
            // `l`'s old buffers are no longer `l`'s: forget who aliased them.
            for (x, vs) in st.vals.iter_mut() {
                if *x != l {
                    vs.retain(|p| !(p.root == Root::Local(l) && p.steps.first().is_some_and(is_d)));
                }
            }
        }
        if k == Kind::Gc {
            for vs in st.vals.values_mut() {
                let moved: Vec<Path> = vs.iter().filter(|p| p.root == Root::GcLocal(l)).cloned().collect();
                for p in moved {
                    vs.remove(&p);
                    let mut q = p;
                    q.root = Root::GcAny;
                    vs.insert(q);
                }
            }
        }
        if k != Kind::None {
            st.vals.insert(l, v);
        }
    }

    /// `l` was just set from expression `x`: a copy of an owning value
    /// makes two owners of its buffers.
    fn note_copy(&mut self, l: LocalId, x: &'a TExpr, st: &mut St) {
        let t = &self.f.locals[l as usize].ty;
        if matches!(t, Ty::Slice(_)) || self.kind(&t.clone()) != Kind::Agg {
            return;
        }
        let src = match &x.kind {
            TK::Local(m) => Some(Path::new(Root::Local(*m))),
            TK::Field(..) | TK::Deref(_) | TK::Index(..) => {
                let mut tmp = st.clone();
                self.report_off(|s| s.place(x, &mut tmp, false).ok()).and_then(|ps| if ps.len() == 1 { ps.into_iter().next() } else { None })
            }
            _ => None,
        };
        let Some(src) = src else { return };
        match src.root {
            Root::Local(m) if !src.has_deref() => {
                if m != l {
                    st.vals.entry(m).or_default().insert(Path::new(Root::Local(l)).push(Step::AnyD));
                }
            }
            _ => {
                st.aliases.insert((l, src));
            }
        }
    }

    fn report_off<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let r = self.report;
        let out = self.out.clone();
        self.report = false;
        let x = f(self);
        self.report = r;
        self.out = out;
        x
    }

    fn bind_pat(&mut self, pat: &TPat, v: &Val, st: &mut St) {
        match &pat.kind {
            TPK::Bind(l) => self.assign_local(*l, v.clone(), st),
            TPK::Variant(_, ps) | TPK::Tuple(ps) | TPK::Or(ps) => {
                for q in ps {
                    self.bind_pat(q, v, st);
                }
            }
            _ => {}
        }
    }

    fn call(&mut self, e: &'a TExpr, f: &'a TExpr, args: &'a [TExpr], st: &mut St) -> Val {
        self.eval(f, st);
        let vals: Vec<Val> = args.iter().map(|a| self.eval(a, st)).collect();
        let callee = match f.kind {
            TK::FnRef(id) => Some(id),
            _ => None,
        };
        let name = match callee {
            Some(id) => format!("`{}`", self.p.fns[id as usize].name),
            None => "this call".into(),
        };
        let mut res = Val::new();
        match callee {
            Some(id) if self.p.fns[id as usize].body.is_some() => {
                let s = self.sums.get(&id).cloned().unwrap_or_default();
                let may_alias = self.p.may_alias.get(&id).copied().unwrap_or(0);
                // Arguments into what the callee moves through another.
                for &(i, j) in &s.pairs {
                    if j < 64 && may_alias & (1 << j) != 0 {
                        continue;
                    }
                    for r in s.restructs.iter().filter(|r| r.root == Root::Param(i)) {
                        for rr in self.subst(r, &vals) {
                            if vals[j as usize].iter().any(|p| hits(&rr, p)) {
                                let msg = format!(
                                    "this argument points into memory that {} may move or free (through argument {}) while it still uses this one",
                                    name,
                                    i + 1
                                );
                                self.error(args[j as usize].span, msg, vec![(e.span, "pass a copy, or an index; or make the callee re-derive the pointer after it grows (see `Vec.extend`, `#[may_alias]`)".into())]);
                            }
                        }
                    }
                    // Propagate: both arguments are this function's parameters.
                    let pi: BTreeSet<u32> = vals[i as usize].iter().filter_map(|p| if let Root::Local(l) = p.root { self.param_idx.get(&l).copied() } else { None }).collect();
                    let pj: BTreeSet<u32> = vals[j as usize].iter().filter_map(|p| if let Root::Local(l) = p.root { self.param_idx.get(&l).copied() } else { None }).collect();
                    for a in &pi {
                        for b in &pj {
                            if a != b {
                                self.out.pairs.insert((*a, *b));
                            }
                        }
                    }
                }
                for (&j, ds) in &s.keeps {
                    let Some(v) = vals.get(j as usize) else { continue };
                    // Where the callee stores it, in this function's terms.
                    let mut dests = BTreeSet::new();
                    for &d in ds {
                        match vals.get(d as usize) {
                            Some(dv) if d < FRAME => {
                                for p in dv {
                                    dests.insert(self.dest(p));
                                }
                                if dv.is_empty() {
                                    dests.insert(ELSEWHERE);
                                }
                            }
                            _ => {
                                dests.insert(ELSEWHERE);
                            }
                        }
                    }
                    let how = format!("passes {{}} to {}, which may keep it beyond the call", name);
                    for q in v.clone() {
                        for &d in &dests {
                            self.escape(&q, d, args[j as usize].span, &how);
                        }
                    }
                }
                let rs: Vec<Path> = s.restructs.iter().flat_map(|r| self.subst(r, &vals)).collect();
                if !rs.is_empty() {
                    let w = self.why(e.span, format!("the call of {}", name));
                    for r in rs {
                        self.restructure(&r, w, st);
                    }
                }
                for r in &s.ret {
                    res.extend(self.subst(r, &vals));
                }
            }
            Some(id) => {
                // Extern (or bodiless) functions: `free` and `realloc` free
                // their first argument's memory; others return pointers
                // into their pointer arguments.
                let fd = &self.p.fns[id as usize];
                if matches!(fd.name.as_str(), "free" | "realloc") && !vals.is_empty() {
                    let w = self.why(e.span, format!("the call of {}", name));
                    for r in vals[0].clone() {
                        self.restructure(&r, w, st);
                    }
                }
                if fd.name != "realloc" && self.kind(&e.ty) != Kind::None {
                    for (a, v) in args.iter().zip(&vals) {
                        if self.kind(&a.ty.clone()) == Kind::Ptr {
                            res.extend(v.iter().cloned());
                        }
                    }
                }
            }
            None => {
                if self.kind(&e.ty) != Kind::None {
                    for v in &vals {
                        res.extend(v.iter().cloned());
                    }
                }
            }
        }
        if crate::objref::may_collect(self.p, f) {
            let w = self.why(e.span, format!("{} (which may collect)", if callee.is_some() { format!("the call of {}", name) } else { "this call".into() }));
            self.restructure(&Path { root: Root::GcAny, steps: Vec::new(), trunc: true }, w, st);
        }
        if self.kind(&e.ty) == Kind::Gc {
            return [Path::new(Root::GcAny)].into_iter().collect();
        }
        res
    }

    fn eval_stmts(&mut self, stmts: &'a [TStmt], st: &mut St) {
        for s in stmts {
            if !st.reach {
                return;
            }
            match s {
                TStmt::Let(l, Some(x)) => {
                    let v = self.eval(x, st);
                    self.assign_local(*l, v, st);
                    self.note_copy(*l, x, st);
                }
                TStmt::Let(l, None) => {
                    self.assign_local(*l, Val::new(), st);
                }
                TStmt::LetPat(pat, x, els) => {
                    let v = self.eval(x, st);
                    if let Some(e) = els {
                        let mut s2 = st.clone();
                        self.eval(e, &mut s2);
                    }
                    self.bind_pat(pat, &v, st);
                }
                TStmt::Expr(x) => {
                    self.eval(x, st);
                }
                TStmt::Defer(x) => {
                    if let Some(d) = self.defers.last_mut() {
                        d.push(x);
                    }
                }
            }
        }
    }

    fn eval(&mut self, e: &'a TExpr, st: &mut St) -> Val {
        if !st.reach {
            return Val::new();
        }
        match &e.kind {
            TK::Local(l) => self.read_local(*l, e.span, st),
            TK::Global(_) | TK::Deref(_) | TK::Field(..) | TK::Index(..) => {
                if let TK::Field(b, k) = &e.kind {
                    if matches!(b.ty, Ty::Slice(_)) {
                        let v = self.eval(b, st);
                        return if *k == 0 { v } else { Val::new() };
                    }
                }
                let t = e.ty.clone();
                match self.place(e, st, true) {
                    Ok(ps) => {
                        self.check_place_read(&ps, e.span, st);
                        self.read_mem(&ps, &t)
                    }
                    Err(v) => {
                        if self.kind(&t) == Kind::None {
                            Val::new()
                        } else {
                            v
                        }
                    }
                }
            }
            TK::AddrOf(x) => match self.place(x, st, true) {
                Ok(ps) => {
                    self.check_place_read(&ps, e.span, st);
                    ps.into_iter().collect()
                }
                Err(v) => v,
            },
            TK::SliceRange(b, lo, hi) => {
                for x in [lo, hi].into_iter().flatten() {
                    self.eval(x, st);
                }
                if matches!(b.ty, Ty::Array(..)) {
                    match self.place(b, st, true) {
                        Ok(ps) => ps.into_iter().collect(),
                        Err(v) => v,
                    }
                } else {
                    self.eval(b, st)
                }
            }
            TK::Trailing(o) => self.eval(o, st).iter().map(|p| p.push(Step::F(TRAIL))).collect(),
            TK::Unary(_, a) => {
                self.eval(a, st);
                Val::new()
            }
            TK::Binary(_, a, b) => {
                let mut v = self.eval(a, st);
                let w = self.eval(b, st);
                if self.kind(&e.ty) == Kind::None {
                    return Val::new();
                }
                v.extend(w);
                v
            }
            TK::Cast(a) => {
                let v = self.eval(a, st);
                if self.kind(&e.ty) == Kind::None { Val::new() } else { v }
            }
            TK::Call(f, args) => self.call(e, f, args, st),
            TK::Intrinsic(_, args) | TK::Struct(args) | TK::Tuple(args) | TK::Array(args) | TK::Variant(_, args) => {
                let mut v = Val::new();
                for a in args {
                    v.extend(self.eval(a, st));
                }
                if self.kind(&e.ty) == Kind::None { Val::new() } else { v }
            }
            TK::ArrayRepeat(a, _) => self.eval(a, st),
            TK::Block(stmts, tail) => {
                self.defers.push(Vec::new());
                self.eval_stmts(stmts, st);
                let v = match tail {
                    Some(t) => self.eval(t, st),
                    None => Val::new(),
                };
                let ds = self.defers.pop().unwrap_or_default();
                for d in ds.into_iter().rev() {
                    self.eval(d, st);
                }
                let mut ls = Vec::new();
                for s in stmts {
                    match s {
                        TStmt::Let(l, _) => ls.push(*l),
                        TStmt::LetPat(p, ..) => pat_locals(p, &mut ls),
                        _ => {}
                    }
                }
                st.drop_locals(&ls);
                v
            }
            TK::If(conds, then, els) => {
                for c in conds {
                    match c {
                        TCond::Expr(x) => {
                            self.eval(x, st);
                        }
                        TCond::Let(pat, x) => {
                            let v = self.eval(x, st);
                            self.bind_pat(pat, &v, st);
                        }
                    }
                }
                let mut s2 = st.clone();
                let mut v = self.eval(then, st);
                if let Some(x) = els {
                    v.extend(self.eval(x, &mut s2));
                }
                st.join(&s2);
                let mut ls = Vec::new();
                for c in conds {
                    if let TCond::Let(p, _) = c {
                        pat_locals(p, &mut ls);
                    }
                }
                st.drop_locals(&ls);
                v
            }
            TK::Match(s, arms) => {
                let sv = self.eval(s, st);
                let start = st.clone();
                let mut out = St::default();
                let mut v = Val::new();
                for a in arms {
                    let mut s2 = start.clone();
                    self.bind_pat(&a.pat, &sv, &mut s2);
                    if let Some(g) = &a.guard {
                        self.eval(g, &mut s2);
                    }
                    v.extend(self.eval(&a.body, &mut s2));
                    let mut ls = Vec::new();
                    pat_locals(&a.pat, &mut ls);
                    s2.drop_locals(&ls);
                    out.join(&s2);
                }
                *st = out;
                v
            }
            TK::Loop { body, step, id } => {
                let ls = self.loop_locals.entry(*id).or_insert_with(|| {
                    let mut ls = Vec::new();
                    bound_locals(body, &mut ls);
                    if let Some(x) = step {
                        bound_locals(x, &mut ls);
                    }
                    ls
                }).clone();
                let mut head = st.clone();
                let mut iters = 0;
                loop {
                    self.conts.remove(id);
                    let mut s = head.clone();
                    self.eval(body, &mut s);
                    if let Some(c) = self.conts.remove(id) {
                        s.join(&c);
                    }
                    s.drop_locals(&ls);
                    if let Some(x) = step {
                        self.eval(x, &mut s);
                    }
                    let mut next = head.clone();
                    next.join(&s);
                    iters += 1;
                    if next == head || iters > 20 {
                        break;
                    }
                    head = next;
                }
                match self.breaks.remove(id) {
                    Some((mut s, v)) => {
                        s.drop_locals(&ls);
                        *st = s;
                        v
                    }
                    None => {
                        st.reach = false;
                        Val::new()
                    }
                }
            }
            TK::Break(id, v) => {
                let val = match v {
                    Some(x) => self.eval(x, st),
                    None => Val::new(),
                };
                let ent = self.breaks.entry(*id).or_insert_with(|| (St::default(), Val::new()));
                ent.0.join(st);
                ent.1.extend(val);
                st.reach = false;
                Val::new()
            }
            TK::Continue(id) => {
                self.conts.entry(*id).or_default().join(st);
                st.reach = false;
                Val::new()
            }
            TK::Return(v) => {
                if let Some(x) = v {
                    let val = self.eval(x, st);
                    self.ret(&val, x.span);
                }
                self.exit_escapes(st);
                st.reach = false;
                Val::new()
            }
            TK::Assign(l, r) => {
                let v = self.eval(r, st);
                if let TK::Local(x) = l.kind {
                    self.assign_local(x, v, st);
                    self.note_copy(x, r, st);
                    return Val::new();
                }
                match self.place(l, st, false) {
                    Ok(ps) => {
                        let local = ps.iter().all(|p| matches!(p.root, Root::Local(_)) && !p.has_deref());
                        if local {
                            // A part of a local aggregate.
                            for p in &ps {
                                if let Root::Local(x) = p.root {
                                    let cur = st.vals.get(&x).cloned().unwrap_or_else(|| self.symbolic(x));
                                    let mut n = cur;
                                    n.extend(v.iter().cloned());
                                    if self.kind(&self.f.locals[x as usize].ty.clone()) != Kind::None {
                                        st.vals.insert(x, n);
                                    }
                                }
                            }
                        } else {
                            // The old value at the place is overwritten.
                            st.esc.retain(|(pl, _, _)| !ps.iter().any(|p| p.root == pl.root && pl.steps.starts_with(&p.steps)));
                            self.store(&v, &ps, r.span, st);
                        }
                        // The old value at the place is gone: what pointed
                        // into its buffers points into a detached block.
                        if self.kind(&l.ty) != Kind::None {
                            for vs in st.vals.values_mut() {
                                vs.retain(|q| !ps.iter().any(|p| q.root == p.root && q.steps.len() > p.steps.len() && q.steps.starts_with(&p.steps) && is_d(&q.steps[p.steps.len()])));
                            }
                        }
                        // A new value at the place: copies of the old one
                        // no longer share its buffers.
                        st.aliases.retain(|(_, pl)| !ps.iter().any(|p| p.root == pl.root && pl.steps.starts_with(&p.steps)));
                        st.stale_places.retain(|pl, _| !ps.iter().any(|p| p.root == pl.root && pl.steps.starts_with(&p.steps)));
                    }
                    Err(_) => {}
                }
                Val::new()
            }
            TK::Coalesce(a, b) => {
                let mut v = self.eval(a, st);
                let mut s2 = st.clone();
                v.extend(self.eval(b, &mut s2));
                st.join(&s2);
                v
            }
            TK::Try(a) => self.eval(a, st),
            TK::New { heap, init, len, fill, .. } => {
                self.eval(heap, st);
                self.eval(init, st);
                for x in [len, fill].into_iter().flatten() {
                    self.eval(x, st);
                }
                [Path::new(Root::GcAny)].into_iter().collect()
            }
            TK::Closure(..) => Val::new(),
            TK::NewClosure(_, caps, a) => {
                self.eval(a, st);
                for l in caps {
                    let v = self.read_local(*l, e.span, st);
                    for q in v {
                        self.escape(&q, ELSEWHERE, e.span, "an escaping closure captures {}, which may outlive it");
                    }
                }
                Val::new()
            }
            TK::Vm(v) => self.vm(e, v, st),
            _ => Val::new(),
        }
    }

    fn ret(&mut self, v: &Val, span: Span) {
        for p in v {
            if let Some(l) = self.storage_local(p) {
                let msg = format!("returns the address of local {}, which dies when the function returns", self.local_name(l));
                self.error(span, msg, vec![(self.f.span, "return the value, or allocate it (`new(&arena, ..)`)".into())]);
            }
            match p.root {
                Root::Local(l) => {
                    if let Some(&i) = self.param_idx.get(&l) {
                        if p.has_deref() {
                            let mut q = p.clone();
                            q.root = Root::Param(i);
                            self.out.ret.insert(q);
                        }
                    }
                }
                Root::Global(_) => {
                    self.out.ret.insert(p.clone());
                }
                _ => {}
            }
        }
    }

    fn vm(&mut self, e: &'a TExpr, v: &'a TVm, st: &mut St) -> Val {
        match v {
            TVm::Synced(call, sync, reload) => {
                self.eval(sync, st);
                let r = self.eval(call, st);
                if !matches!(reload.kind, TK::Unit) {
                    // Guest code ran: the state `reload` recomputes may
                    // have moved (a stack that grew).
                    let mut ls = Vec::new();
                    assigned_locals(reload, &mut ls);
                    let w = self.why(call.span, "this `reenter` call (guest code may grow or move the vm's stacks; `reload` recomputes the state)".into());
                    for l in ls {
                        let mut targets = st.vals.get(&l).cloned().unwrap_or_else(|| self.symbolic(l));
                        targets.insert(Path::new(Root::Local(l)).push(Step::D));
                        for t in targets {
                            self.restructure(&t, w, st);
                        }
                    }
                    // `reload` itself sets them again.
                    self.eval(reload, st);
                }
                r
            }
            TVm::Jump(a) | TVm::Exit(a) | TVm::Raise(a) | TVm::Cold(a) | TVm::Part(_, a) | TVm::WordAt(a, _, _) => {
                let _ = e;
                self.eval(a, st);
                Val::new()
            }
            TVm::SharedNext(t, _, _) => {
                self.eval(t, st);
                Val::new()
            }
            TVm::Shared(_, args, size, cont) => {
                for a in args {
                    self.eval(a, st);
                }
                self.eval(size, st);
                if let Some(c) = cont {
                    self.eval(c, st);
                }
                Val::new()
            }
            _ => Val::new(),
        }
    }
}

/// The locals a `reload` block assigns.
fn assigned_locals(e: &TExpr, out: &mut Vec<LocalId>) {
    match &e.kind {
        TK::Assign(l, _) => {
            if let TK::Local(x) = l.kind {
                out.push(x);
            }
        }
        TK::Block(ss, t) => {
            for s in ss {
                if let TStmt::Expr(x) = s {
                    assigned_locals(x, out);
                }
            }
            if let Some(t) = t {
                assigned_locals(t, out);
            }
        }
        _ => {}
    }
}

/// Where the checker found checked modules and `unsafe { }` blocks.
pub struct Ctx<'a> {
    pub checked_files: &'a HashSet<u32>,
    pub unsafe_spans: &'a [Span],
}

fn analyze<'a>(
    p: &'a Program,
    fid: FnId,
    sums: &'a HashMap<FnId, Summary>,
    kinds: &'a mut HashMap<Ty, Kind>,
    cx: &Ctx<'a>,
    report: bool,
    audit: Option<&'a crate::diag::SourceMap>,
) -> (Summary, Vec<Diag>) {
    let f = &p.fns[fid as usize];
    let Some(b) = &f.body else { return (Summary::default(), Vec::new()) };
    let mut fa = Fa {
        p,
        fid,
        f,
        sums,
        report,
        unsafe_spans: cx.unsafe_spans,
        audit,
        kinds,
        whys: Vec::new(),
        errs: Vec::new(),
        err_spans: HashSet::new(),
        breaks: HashMap::new(),
        conts: HashMap::new(),
        loop_locals: HashMap::new(),
        defers: Vec::new(),
        out: Summary::default(),
        param_idx: f.params.iter().enumerate().map(|(i, &l)| (l, i as u32)).collect(),
    };
    let _ = fa.fid;
    let mut st = St { reach: true, ..Default::default() };
    // Parameters and other locals not bound by a `let` or pattern (vm
    // state) start pointing at their own pointee; bound ones are assigned
    // before they are read.
    let mut bound = Vec::new();
    bound_locals(b, &mut bound);
    let bound: HashSet<LocalId> = bound.into_iter().collect();
    for (l, _) in f.locals.iter().enumerate() {
        if bound.contains(&(l as LocalId)) {
            continue;
        }
        let v = fa.symbolic(l as LocalId);
        if !v.is_empty() {
            st.vals.insert(l as LocalId, v);
        }
    }
    let v = fa.eval(b, &mut st);
    if std::env::var("LOIMI_LT_DBG").is_ok() && f.symbol.ends_with("__verify") {
        eprintln!("{}: locals {} vals {} paths {}", f.symbol, f.locals.len(), st.vals.len(), st.vals.values().map(|v| v.len()).sum::<usize>());
    }
    if st.reach {
        fa.ret(&v, b.span);
        fa.exit_escapes(&st);
    }
    (fa.out, fa.errs)
}

/// Summarize every function (a least fixpoint over the call graph), then
/// check the functions of checked modules.
pub fn check(p: &Program, cx: &Ctx, sm: &crate::diag::SourceMap) -> Res<()> {
    let mode = std::env::var("LOIMI_LIFETIMES").unwrap_or_default();
    if mode == "off" {
        return Ok(());
    }
    let audit = (mode == "audit").then_some(sm);
    let body = |i: FnId| p.fns[i as usize].body.is_some();
    let checked: Vec<FnId> = (0..p.fns.len() as FnId)
        .filter(|&i| body(i) && (audit.is_some() || cx.checked_files.contains(&p.fns[i as usize].span.file)))
        .collect();
    if checked.is_empty() {
        return Ok(());
    }
    // Only what checked functions call (transitively) needs a summary.
    // Post-order, so callees are summarized before their callers.
    let mut calls: HashMap<FnId, Vec<FnId>> = HashMap::new();
    let mut fns: Vec<FnId> = Vec::new();
    let mut seen: HashSet<FnId> = HashSet::new();
    for &c in &checked {
        let mut stack: Vec<(FnId, usize)> = Vec::new();
        if seen.insert(c) {
            stack.push((c, 0));
        }
        while let Some(&mut (f, ref mut k)) = stack.last_mut() {
            let cs = calls.entry(f).or_insert_with(|| {
                let mut cs = Vec::new();
                callees(p.fns[f as usize].body.as_ref().unwrap(), &mut cs);
                cs.sort();
                cs.dedup();
                cs.retain(|&x| body(x));
                cs
            });
            if *k < cs.len() {
                let g = cs[*k];
                *k += 1;
                if seen.insert(g) {
                    stack.push((g, 0));
                }
            } else {
                fns.push(f);
                stack.pop();
            }
        }
    }
    // Callers of each function, to revisit only what a changed summary
    // can change.
    let mut callers: HashMap<FnId, Vec<FnId>> = HashMap::new();
    for (&f, cs) in &calls {
        for &c in cs {
            callers.entry(c).or_default().push(f);
        }
    }
    let mut sums: HashMap<FnId, Summary> = HashMap::new();
    let mut kinds: HashMap<Ty, Kind> = HashMap::new();
    let mut work: Vec<FnId> = fns.iter().rev().copied().collect();
    let mut queued: HashSet<FnId> = fns.iter().copied().collect();
    let mut rounds = 0u64;
    let stats = std::env::var("LOIMI_LIFETIMES_STATS").is_ok();
    let mut times: HashMap<FnId, (f64, u32)> = HashMap::new();
    while let Some(i) = work.pop() {
        queued.remove(&i);
        rounds += 1;
        let t0 = std::time::Instant::now();
        let (s, _) = analyze(p, i, &sums, &mut kinds, cx, false, None);
        if stats {
            let e = times.entry(i).or_default();
            e.0 += t0.elapsed().as_secs_f64();
            e.1 += 1;
        }
        let old = sums.get(&i).cloned().unwrap_or_default();
        let mut merged = old.clone();
        merged.restructs.extend(s.restructs);
        merged.ret.extend(s.ret);
        merged.pairs.extend(s.pairs);
        for (j, d) in s.keeps {
            merged.keeps.entry(j).or_default().extend(d);
        }
        if merged != old {
            sums.insert(i, merged);
            for &c in callers.get(&i).map(|v| v.as_slice()).unwrap_or(&[]) {
                if queued.insert(c) {
                    work.push(c);
                }
            }
        }
    }
    if std::env::var("LOIMI_LIFETIMES_STATS").is_ok() {
        eprintln!("lifetimes: {} functions, {} analyses", fns.len(), rounds);
        let mut v: Vec<_> = times.iter().collect();
        v.sort_by(|a, b| b.1.0.partial_cmp(&a.1.0).unwrap());
        for (f, (t, n)) in v.iter().take(12) {
            eprintln!("  {:8.1} ms {:3}x {}", t * 1000.0, n, p.fns[**f as usize].symbol);
        }
    }
    let t1 = std::time::Instant::now();
    for &i in &checked {
        let (_, errs) = analyze(p, i, &sums, &mut kinds, cx, true, audit);
        if let Some(d) = errs.into_iter().next() {
            return Err(d);
        }
    }
    if stats {
        eprintln!("lifetimes: summaries {:.1} ms, checks {:.1} ms", times.values().map(|t| t.0).sum::<f64>() * 1000.0, t1.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(())
}

fn callees(e: &TExpr, out: &mut Vec<FnId>) {
    if let TK::FnRef(id) = e.kind {
        out.push(id);
    }
    walk_kids(e, &mut |k| callees(k, out));
}

/// Every direct subexpression of `e`.
fn walk_kids<'a>(e: &'a TExpr, f: &mut dyn FnMut(&'a TExpr)) {
    match &e.kind {
        TK::Deref(a) | TK::Field(a, _) | TK::AddrOf(a) | TK::Unary(_, a) | TK::Cast(a) | TK::ArrayRepeat(a, _) | TK::Try(a) | TK::Trailing(a) => f(a),
        TK::Index(a, b) | TK::Binary(_, a, b) | TK::Assign(a, b) | TK::Coalesce(a, b) => {
            f(a);
            f(b);
        }
        TK::SliceRange(a, lo, hi) => {
            f(a);
            for x in [lo, hi].into_iter().flatten() {
                f(x);
            }
        }
        TK::Call(c, args) => {
            f(c);
            args.iter().for_each(|a| f(a));
        }
        TK::Intrinsic(_, args) | TK::Struct(args) | TK::Tuple(args) | TK::Array(args) | TK::Variant(_, args) => args.iter().for_each(|a| f(a)),
        TK::Block(ss, t) => {
            for s in ss {
                match s {
                    TStmt::Let(_, Some(x)) | TStmt::Expr(x) | TStmt::Defer(x) => f(x),
                    TStmt::LetPat(_, x, els) => {
                        f(x);
                        if let Some(e) = els {
                            f(e);
                        }
                    }
                    TStmt::Let(_, None) => {}
                }
            }
            if let Some(t) = t {
                f(t);
            }
        }
        TK::If(cs, t, el) => {
            for c in cs {
                match c {
                    TCond::Expr(x) | TCond::Let(_, x) => f(x),
                }
            }
            f(t);
            if let Some(x) = el {
                f(x);
            }
        }
        TK::Match(s, arms) => {
            f(s);
            for a in arms {
                if let Some(g) = &a.guard {
                    f(g);
                }
                f(&a.body);
            }
        }
        TK::Loop { body, step, .. } => {
            f(body);
            if let Some(s) = step {
                f(s);
            }
        }
        TK::Break(_, Some(v)) | TK::Return(Some(v)) => f(v),
        TK::New { heap, init, len, fill, .. } => {
            f(heap);
            f(init);
            for x in [len, fill].into_iter().flatten() {
                f(x);
            }
        }
        TK::NewClosure(_, _, a) => f(a),
        TK::Vm(v) => match v {
            TVm::Jump(a) | TVm::Exit(a) | TVm::Raise(a) | TVm::Cold(a) | TVm::Part(_, a) | TVm::WordAt(a, _, _) => f(a),
            TVm::SharedNext(t, _, _) => f(t),
            TVm::Synced(a, b, c) => {
                f(a);
                f(b);
                f(c);
            }
            TVm::Shared(_, args, size, cont) => {
                args.iter().for_each(|a| f(a));
                f(size);
                if let Some(c) = cont {
                    f(c);
                }
            }
            _ => {}
        },
        _ => {}
    }
}
