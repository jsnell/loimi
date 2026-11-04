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
use std::rc::Rc;

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

/// At most `K` steps, inline: paths are copied at every branch and call,
/// so they must not allocate.
#[derive(Clone, Copy)]
struct Steps {
    a: [Step; K],
    n: u8,
}

impl Steps {
    fn new() -> Steps {
        Steps { a: [Step::D; K], n: 0 }
    }
    fn push(&mut self, s: Step) {
        self.a[self.n as usize] = s;
        self.n += 1;
    }
    fn extend_from_slice(&mut self, ss: &[Step]) {
        for s in ss {
            self.push(*s);
        }
    }
}

impl std::ops::Deref for Steps {
    type Target = [Step];
    fn deref(&self) -> &[Step] {
        &self.a[..self.n as usize]
    }
}

impl PartialEq for Steps {
    fn eq(&self, o: &Steps) -> bool {
        **self == **o
    }
}
impl Eq for Steps {}
impl PartialOrd for Steps {
    fn partial_cmp(&self, o: &Steps) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Steps {
    fn cmp(&self, o: &Steps) -> std::cmp::Ordering {
        (**self).cmp(&**o)
    }
}
impl std::hash::Hash for Steps {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        (**self).hash(h)
    }
}
impl std::fmt::Debug for Steps {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        (**self).fmt(f)
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
struct Path {
    root: Root,
    steps: Steps,
    trunc: bool,
}

impl Path {
    fn new(root: Root) -> Path {
        Path { root, steps: Steps::new(), trunc: false }
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

/// The flow state. Cloned at every branch and joined after it, so every
/// part is shared (`Rc`) and copied only when a branch changes it; a join
/// of two states that share a part skips it.
#[derive(Clone, Default, PartialEq)]
struct St {
    reach: bool,
    /// What each local may point into.
    vals: Rc<BTreeMap<LocalId, Rc<Val>>>,
    /// Locals that may dangle, and why (an index into `Fa::whys`).
    stale: Rc<BTreeMap<LocalId, u32>>,
    /// Locals pointing into memory of parameter j after a restructure
    /// through parameter i: (i, j) (summary `pairs` if used).
    cst: Rc<BTreeMap<LocalId, BTreeSet<(u32, u32)>>>,
    /// (local, place): the local holds a copy of the owning value at
    /// `place` (memory), so freeing through it leaves `place` dangling.
    aliases: Rc<BTreeSet<(LocalId, Path)>>,
    /// Places in memory whose owned buffers were freed through a copy.
    stale_places: Rc<BTreeMap<Path, u32>>,
    /// Pending escapes: (place, stored pointer, why): the address of a
    /// local (or a parameter's pointee) stored in memory that outlives
    /// the call, unless overwritten before it returns.
    esc: Rc<BTreeSet<(Path, Path, u32)>>,
}

/// `a` |= `b` for a shared set, copying `a` only if it grows.
fn union_set<T: Ord + Clone>(a: &mut Rc<BTreeSet<T>>, b: &Rc<BTreeSet<T>>) {
    if Rc::ptr_eq(a, b) || b.iter().all(|x| a.contains(x)) {
        return;
    }
    Rc::make_mut(a).extend(b.iter().cloned());
}

/// Keep the first value of each key of `b` missing in `a`.
fn union_first<K: Ord + Clone, V: Copy>(a: &mut Rc<BTreeMap<K, V>>, b: &Rc<BTreeMap<K, V>>) {
    if Rc::ptr_eq(a, b) || b.keys().all(|k| a.contains_key(k)) {
        return;
    }
    let m = Rc::make_mut(a);
    for (k, v) in b.iter() {
        m.entry(k.clone()).or_insert(*v);
    }
}

/// Remove the keys of a shared map not matching `f`, copying it only if
/// any does not.
fn retain_map<K: Ord + Clone, V: Clone>(a: &mut Rc<BTreeMap<K, V>>, mut f: impl FnMut(&K, &V) -> bool) {
    if a.iter().all(|(k, v)| f(k, v)) {
        return;
    }
    Rc::make_mut(a).retain(|k, v| f(k, v));
}

fn retain_set<T: Ord + Clone>(a: &mut Rc<BTreeSet<T>>, mut f: impl FnMut(&T) -> bool) {
    if a.iter().all(|x| f(x)) {
        return;
    }
    Rc::make_mut(a).retain(|x| f(x));
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
        if !Rc::ptr_eq(&self.vals, &o.vals) {
            for (l, v) in o.vals.iter() {
                match self.vals.get(l) {
                    Some(sv) if Rc::ptr_eq(sv, v) || v.iter().all(|p| sv.contains(p)) => {}
                    Some(_) => {
                        let sv = Rc::make_mut(&mut self.vals).get_mut(l).unwrap();
                        Rc::make_mut(sv).extend(v.iter().cloned());
                    }
                    None => {
                        Rc::make_mut(&mut self.vals).insert(*l, v.clone());
                    }
                }
            }
        }
        union_first(&mut self.stale, &o.stale);
        if !Rc::ptr_eq(&self.cst, &o.cst) {
            for (l, v) in o.cst.iter() {
                if !self.cst.get(l).is_some_and(|sv| v.iter().all(|x| sv.contains(x))) {
                    Rc::make_mut(&mut self.cst).entry(*l).or_default().extend(v.iter().cloned());
                }
            }
        }
        union_set(&mut self.aliases, &o.aliases);
        union_first(&mut self.stale_places, &o.stale_places);
        union_set(&mut self.esc, &o.esc);
    }

    /// Locals whose scope ended: nothing reads them again before they
    /// are bound anew, so their entries only cost (cloned at every branch).
    fn drop_locals(&mut self, ls: &[LocalId]) {
        for l in ls {
            if self.vals.contains_key(l) {
                Rc::make_mut(&mut self.vals).remove(l);
            }
            if self.stale.contains_key(l) {
                Rc::make_mut(&mut self.stale).remove(l);
            }
            if self.cst.contains_key(l) {
                Rc::make_mut(&mut self.cst).remove(l);
            }
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
    sums: &'a FxMap<FnId, Summary>,
    report: bool,
    unsafe_spans: &'a [Span],
    audit: Option<&'a crate::diag::SourceMap>,
    kinds: &'a mut Kinds,
    whys: Vec<(Span, Why)>,
    why_ids: FxMap<(Span, Why), u32>,
    errs: Vec<Diag>,
    /// Audit mode: the `lifetime-site` lines.
    lines: Vec<String>,
    err_spans: FxSet<(u32, u32)>,
    breaks: FxMap<u32, (St, Val)>,
    conts: FxMap<u32, St>,
    /// Per loop: the locals bound inside its body (dropped when it exits).
    loop_locals: FxMap<u32, Vec<LocalId>>,
    defers: Vec<Vec<&'a TExpr>>,
    out: Summary,
    param_idx: ParamIdx,
    /// (callee, argument, what it points into) whose restructures are
    /// already in `out`.
    summed: FxSet<(FnId, u32, Path)>,
}

/// Local -> parameter index.
struct ParamIdx(Vec<u32>);

impl ParamIdx {
    fn new(f: &FnDef) -> ParamIdx {
        let mut v = vec![u32::MAX; f.locals.len()];
        for (i, &l) in f.params.iter().enumerate() {
            v[l as usize] = i as u32;
        }
        ParamIdx(v)
    }
    fn get(&self, l: &LocalId) -> Option<&u32> {
        self.0.get(*l as usize).filter(|i| **i != u32::MAX)
    }
    fn contains_key(&self, l: &LocalId) -> bool {
        self.get(l).is_some()
    }
}

/// A fast hasher for the pass's maps of small keys (ids, addresses).
#[derive(Default, Clone, Copy)]
struct Fx(u64);

impl std::hash::Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.write_u64(*b as u64);
        }
    }
    fn write_u8(&mut self, i: u8) {
        self.write_u64(i as u64);
    }
    fn write_u32(&mut self, i: u32) {
        self.write_u64(i as u64);
    }
    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64);
    }
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

type FxMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<Fx>>;
type FxSet<K> = HashSet<K, std::hash::BuildHasherDefault<Fx>>;

/// `kind_of`, memoized by type and by the address of a type in the program.
#[derive(Default)]
struct Kinds {
    by_ty: HashMap<Ty, Kind>,
    by_addr: FxMap<usize, Kind>,
}

/// Why a local may dangle (formatted only for a message).
#[derive(Clone, PartialEq, Eq, Hash)]
enum Why {
    Text(String),
    /// The call of a function (or of a function value).
    Call(Option<FnId>),
    /// A call that may collect.
    Collect(Option<FnId>),
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
    fn kind(&mut self, t: &'a Ty) -> Kind {
        // Every type asked about is in the program (`'a`), so its
        // address identifies it for the whole pass.
        let a = t as *const Ty as usize;
        if let Some(k) = self.kinds.by_addr.get(&a) {
            return *k;
        }
        let k = kind_of(self.p, t, &mut self.kinds.by_ty, 0);
        self.kinds.by_addr.insert(a, k);
        k
    }

    fn in_unsafe(&self, s: Span) -> bool {
        self.unsafe_spans.iter().any(|u| u.file == s.file && u.lo <= s.lo && s.hi <= u.hi)
    }

    /// The same reason at the same place has the same index, so that a
    /// loop's state reaches its fixpoint.
    fn why(&mut self, span: Span, what: Why) -> u32 {
        if let Some(&w) = self.why_ids.get(&(span, what.clone())) {
            return w;
        }
        self.whys.push((span, what.clone()));
        let w = (self.whys.len() - 1) as u32;
        self.why_ids.insert((span, what), w);
        w
    }

    fn why_text(&self, w: u32) -> (Span, String) {
        let (s, why) = &self.whys[w as usize];
        let name = |id: &Option<FnId>| match id {
            Some(id) => format!("the call of `{}`", self.p.fns[*id as usize].name),
            None => "this call".into(),
        };
        let t = match why {
            Why::Text(t) => t.clone(),
            Why::Call(id) => name(id),
            Why::Collect(id) => format!("{} (which may collect)", name(id)),
        };
        (*s, t)
    }

    fn error(&mut self, span: Span, msg: String, notes: Vec<(Span, String)>) {
        if !self.report || self.in_unsafe(span) || !self.err_spans.insert((span.file, span.lo)) {
            return;
        }
        if let Some(sm) = self.audit {
            let (l, c) = sm.line_col(span.file, span.lo);
            self.lines.push(format!("lifetime-site {}:{}:{} {} ({})", sm.files[span.file as usize].path, l, c, msg, self.f.symbol));
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
        let t = &self.f.locals[l as usize].ty;
        let mut v = Val::new();
        match self.kind(t) {
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
            let (ws, what) = self.why_text(w);
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
        let t = &ld.ty;
        let mutable = ld.mutable;
        match self.kind(t) {
            Kind::None => Val::new(),
            Kind::Gc if !mutable => [Path::new(Root::GcLocal(l))].into_iter().collect(),
            k => {
                let mut v = st.vals.get(&l).map(|v| (**v).clone()).unwrap_or_else(|| self.symbolic(l));
                // An owning value (a `Vec`) read whole: its buffers.
                if k == Kind::Agg && !matches!(t, Ty::Slice(_)) {
                    v.insert(Path::new(Root::Local(l)).push(Step::AnyD));
                }
                v
            }
        }
    }

    /// The value read from memory at `paths`, of type `t`.
    fn read_mem(&mut self, paths: &[Path], t: &'a Ty) -> Val {
        match self.kind(t) {
            Kind::None => Val::new(),
            Kind::Ptr => paths.iter().map(|p| p.push(Step::D)).collect(),
            Kind::Agg => paths.iter().map(|p| p.push(Step::AnyD)).collect(),
            Kind::Gc => [Path::new(Root::GcAny)].into_iter().collect(),
        }
    }

    fn check_place_read(&mut self, paths: &[Path], span: Span, st: &St) {
        for (sp, w) in st.stale_places.iter() {
            if paths.iter().any(|p| p.root == sp.root && p.steps.len() >= sp.steps.len() && p.steps[..sp.steps.len()] == sp.steps[..]) {
                let (ws, what) = self.why_text(*w);
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

    /// Move or free the memory at each of `rs`: what points into it may
    /// dangle. One pass over the state for all of them (a call's summary
    /// may name hundreds of blocks).
    fn restructure(&mut self, rs: &[Path], w: u32, st: &mut St, summarize: bool) {
        if rs.is_empty() {
            return;
        }
        let hit = |p: &Path| rs.iter().any(|r| hits(r, p));
        let mut newly = Vec::new();
        for (l, v) in st.vals.iter() {
            if !st.stale.contains_key(l) && v.iter().any(hit) {
                newly.push(*l);
            }
        }
        if !newly.is_empty() {
            let stale = Rc::make_mut(&mut st.stale);
            for l in newly {
                stale.insert(l, w);
            }
        }
        // Through parameter i: values pointing into parameter j's memory
        // would dangle if j aliased it.
        let mut through: Vec<u32> = rs
            .iter()
            .filter_map(|r| match r.root {
                Root::Local(rl) if r.has_deref() => self.param_idx.get(&rl).copied(),
                _ => None,
            })
            .collect();
        through.sort_unstable();
        through.dedup();
        if !through.is_empty() {
            let mut add = Vec::new();
            for (l, v) in st.vals.iter() {
                for p in v.iter() {
                    if let Root::Local(pl) = p.root {
                        if let Some(&j) = self.param_idx.get(&pl) {
                            if !p.has_deref() {
                                continue;
                            }
                            for &i in &through {
                                if j != i && !st.cst.get(l).is_some_and(|c| c.contains(&(i, j))) {
                                    add.push((*l, (i, j)));
                                }
                            }
                        }
                    }
                }
            }
            for (l, ij) in add {
                Rc::make_mut(&mut st.cst).entry(l).or_default().insert(ij);
            }
        }
        let mut places = Vec::new();
        for (l, place) in st.aliases.iter() {
            if hit(&Path::new(Root::Local(*l)).push(Step::AnyD)) {
                places.push(place.clone());
            }
        }
        for pl in places {
            if !st.stale_places.contains_key(&pl) {
                Rc::make_mut(&mut st.stale_places).insert(pl, w);
            }
        }
        if summarize {
            for r in rs {
                self.summarize(r);
            }
        }
    }

    /// The summary: restructures through parameters and globals.
    fn summarize(&mut self, r: &Path) {
        match r.root {
            Root::Local(l) if r.has_deref() => {
                if let Some(&i) = self.param_idx.get(&l) {
                    let mut q = r.clone();
                    q.root = Root::Param(i);
                    if !self.out.restructs.contains(&q) {
                        self.out.restructs.insert(q);
                    }
                }
            }
            Root::Global(_) => {
                if !self.out.restructs.contains(r) {
                    self.out.restructs.insert(r.clone());
                }
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
                let Some(a) = args.get(i as usize) else { return Vec::new() };
                a.iter().map(|t| subst_one(p, pos, t)).collect()
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
                    let w = self.why(span, Why::Text(String::new()));
                    Rc::make_mut(&mut st.esc).insert((pl.clone(), q.clone(), w));
                }
            }
        }
    }

    /// At a return: what is still stored escapes.
    fn exit_escapes(&mut self, st: &St) {
        for (pl, q, w) in st.esc.iter().cloned().collect::<Vec<_>>() {
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
        let t = &self.f.locals[l as usize].ty;
        let k = self.kind(t);
        if st.stale.contains_key(&l) {
            Rc::make_mut(&mut st.stale).remove(&l);
        }
        if st.cst.contains_key(&l) {
            Rc::make_mut(&mut st.cst).remove(&l);
        }
        retain_set(&mut st.aliases, |(x, _)| *x != l);
        if k == Kind::Agg && !matches!(t, Ty::Slice(_)) {
            // `l`'s old buffers are no longer `l`'s: forget who aliased them.
            let old = |p: &Path| p.root == Root::Local(l) && p.steps.first().is_some_and(is_d);
            let hit: Vec<LocalId> = st.vals.iter().filter(|(x, vs)| **x != l && vs.iter().any(old)).map(|(x, _)| *x).collect();
            for x in hit {
                let vs = Rc::make_mut(Rc::make_mut(&mut st.vals).get_mut(&x).unwrap());
                vs.retain(|p| !old(p));
            }
        }
        if k == Kind::Gc {
            let hit: Vec<LocalId> = st.vals.iter().filter(|(_, vs)| vs.iter().any(|p| p.root == Root::GcLocal(l))).map(|(x, _)| *x).collect();
            for x in hit {
                let vs = Rc::make_mut(Rc::make_mut(&mut st.vals).get_mut(&x).unwrap());
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
            Rc::make_mut(&mut st.vals).insert(l, Rc::new(v));
        }
    }

    /// `l` was just set from expression `x`: a copy of an owning value
    /// makes two owners of its buffers.
    fn note_copy(&mut self, l: LocalId, x: &'a TExpr, st: &mut St) {
        let t = &self.f.locals[l as usize].ty;
        if matches!(t, Ty::Slice(_)) || self.kind(t) != Kind::Agg {
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
                    let p = Path::new(Root::Local(l)).push(Step::AnyD);
                    if !st.vals.get(&m).is_some_and(|v| v.contains(&p)) {
                        Rc::make_mut(Rc::make_mut(&mut st.vals).entry(m).or_default()).insert(p);
                    }
                }
            }
            _ => {
                if !st.aliases.contains(&(l, src.clone())) {
                    Rc::make_mut(&mut st.aliases).insert((l, src));
                }
            }
        }
    }

    fn report_off<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let r = self.report;
        let out = self.out.clone();
        let summed = std::mem::take(&mut self.summed);
        self.report = false;
        let x = f(self);
        self.report = r;
        self.out = out;
        self.summed = summed;
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
        let prog = self.p;
        let name = move || match callee {
            Some(id) => format!("`{}`", prog.fns[id as usize].name),
            None => "this call".into(),
        };
        let mut res = Val::new();
        match callee {
            Some(id) if self.p.fns[id as usize].body.is_some() => {
                static EMPTY: std::sync::LazyLock<Summary> = std::sync::LazyLock::new(Summary::default);
                let s: &'a Summary = self.sums.get(&id).unwrap_or(&EMPTY);
                let may_alias = self.p.may_alias.get(&id).copied().unwrap_or(0);
                // What it moves or frees, in this function's terms (and
                // through which parameter).
                // The summary takes them once per callee and argument path
                // (a recursive walker's summary may name hundreds).
                let mut fresh: Vec<Vec<bool>> = Vec::with_capacity(vals.len());
                for (i, a) in vals.iter().enumerate() {
                    let mut f = Vec::with_capacity(a.len());
                    for t in a.iter() {
                        let param = matches!(t.root, Root::Local(l) if self.param_idx.contains_key(&l));
                        f.push(param && self.summed.insert((id, i as u32, t.clone())));
                    }
                    fresh.push(f);
                }
                let fresh_globals = self.summed.insert((id, u32::MAX, Path::new(Root::GcAny)));
                let mut rsub: Vec<(Option<u32>, Path)> = Vec::new();
                for r in &s.restructs {
                    match r.root {
                        Root::Param(i) => {
                            let Some(pos) = r.steps.iter().position(is_d) else { continue };
                            let Some(a) = vals.get(i as usize) else { continue };
                            for (k, t) in a.iter().enumerate() {
                                let q = subst_one(r, pos, t);
                                if fresh[i as usize][k] {
                                    self.summarize(&q);
                                }
                                rsub.push((Some(i), q));
                            }
                        }
                        Root::Global(_) | Root::GcAny => {
                            if fresh_globals {
                                self.summarize(r);
                            }
                            rsub.push((None, r.clone()));
                        }
                        _ => {}
                    }
                }
                // Arguments into what the callee moves through another.
                for &(i, j) in &s.pairs {
                    if j < 64 && may_alias & (1 << j) != 0 {
                        continue;
                    }
                    for (_, rr) in rsub.iter().filter(|(k, _)| *k == Some(i)) {
                        {
                            if vals[j as usize].iter().any(|p| hits(rr, p)) {
                                let msg = format!(
                                    "this argument points into memory that {} may move or free (through argument {}) while it still uses this one",
                                    name(),
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
                    let how = format!("passes {{}} to {}, which may keep it beyond the call", name());
                    for q in v.clone() {
                        for &d in &dests {
                            self.escape(&q, d, args[j as usize].span, &how);
                        }
                    }
                }
                if !rsub.is_empty() {
                    let rs: Vec<Path> = rsub.into_iter().map(|(_, q)| q).collect();
                    let w = self.why(e.span, Why::Call(callee));
                    self.restructure(&rs, w, st, false);
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
                    let w = self.why(e.span, Why::Call(callee));
                    let rs: Vec<Path> = vals[0].iter().cloned().collect();
                    self.restructure(&rs, w, st, true);
                }
                if fd.name != "realloc" && self.kind(&e.ty) != Kind::None {
                    for (a, v) in args.iter().zip(&vals) {
                        if self.kind(&a.ty) == Kind::Ptr {
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
        if may_collect(self.p, f) {
            let w = self.why(e.span, Why::Collect(callee));
            self.restructure(&[Path { root: Root::GcAny, steps: Steps::new(), trunc: true }], w, st, true);
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
                let t = &e.ty;
                match self.place(e, st, true) {
                    Ok(ps) => {
                        self.check_place_read(&ps, e.span, st);
                        self.read_mem(&ps, t)
                    }
                    Err(v) => {
                        if self.kind(t) == Kind::None {
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
                                    let cur = st.vals.get(&x).map(|v| (**v).clone()).unwrap_or_else(|| self.symbolic(x));
                                    let mut n = cur;
                                    n.extend(v.iter().cloned());
                                    if self.kind(&self.f.locals[x as usize].ty) != Kind::None {
                                        Rc::make_mut(&mut st.vals).insert(x, Rc::new(n));
                                    }
                                }
                            }
                        } else {
                            // The old value at the place is overwritten.
                            retain_set(&mut st.esc, |(pl, _, _)| !ps.iter().any(|p| p.root == pl.root && pl.steps.starts_with(&p.steps)));
                            self.store(&v, &ps, r.span, st);
                        }
                        // The old value at the place is gone: what pointed
                        // into its buffers points into a detached block.
                        if self.kind(&l.ty) != Kind::None {
                            let det = |q: &Path| ps.iter().any(|p| q.root == p.root && q.steps.len() > p.steps.len() && q.steps.starts_with(&p.steps) && is_d(&q.steps[p.steps.len()]));
                            let hit: Vec<LocalId> = st.vals.iter().filter(|(_, vs)| vs.iter().any(det)).map(|(x, _)| *x).collect();
                            for x in hit {
                                Rc::make_mut(Rc::make_mut(&mut st.vals).get_mut(&x).unwrap()).retain(|q| !det(q));
                            }
                        }
                        // A new value at the place: copies of the old one
                        // no longer share its buffers.
                        retain_set(&mut st.aliases, |(_, pl)| !ps.iter().any(|p| p.root == pl.root && pl.steps.starts_with(&p.steps)));
                        retain_map(&mut st.stale_places, |pl, _| !ps.iter().any(|p| p.root == pl.root && pl.steps.starts_with(&p.steps)));
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
                    let w = self.why(call.span, Why::Text("this `reenter` call (guest code may grow or move the vm's stacks; `reload` recomputes the state)".into()));
                    let mut targets = Val::new();
                    for l in ls {
                        targets.extend(st.vals.get(&l).map(|v| (**v).clone()).unwrap_or_else(|| self.symbolic(l)));
                        targets.insert(Path::new(Root::Local(l)).push(Step::D));
                    }
                    let rs: Vec<Path> = targets.into_iter().collect();
                    self.restructure(&rs, w, st, true);
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

/// Parameter path `p` (its first dereference at `pos`) for an argument
/// pointing into `t`.
fn subst_one(p: &Path, pos: usize, t: &Path) -> Path {
    let rest = &p.steps[pos + 1..];
    let mut q = t.clone();
    if !q.trunc {
        let room = K - q.steps.len().min(K);
        q.steps.extend_from_slice(&rest[..rest.len().min(room)]);
        if rest.len() > room {
            q.trunc = true;
        }
    }
    if p.trunc {
        q.trunc = true;
    }
    q
}

/// `objref::may_collect` without copying the callee's type.
fn may_collect(p: &Program, f: &TExpr) -> bool {
    let t = match &f.ty {
        Ty::Gc(inner) if matches!(**inner, Ty::Closure(..)) => &**inner,
        t => t,
    };
    let (Ty::Fn(_, _, mut effects) | Ty::Block(_, _, mut effects) | Ty::Closure(_, _, mut effects)) = *t else {
        return false;
    };
    if let TK::FnRef(id) = f.kind {
        effects |= p.fns[id as usize].sync_effects;
    }
    effects & EFF_GC != 0
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
    sums: &'a FxMap<FnId, Summary>,
    kinds: &'a mut Kinds,
    cx: &Ctx<'a>,
    report: bool,
    audit: Option<&'a crate::diag::SourceMap>,
) -> (Summary, Vec<Diag>, Vec<String>) {
    let f = &p.fns[fid as usize];
    let Some(b) = &f.body else { return (Summary::default(), Vec::new(), Vec::new()) };
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
        why_ids: FxMap::default(),
        errs: Vec::new(),
        lines: Vec::new(),
        err_spans: FxSet::default(),
        breaks: FxMap::default(),
        conts: FxMap::default(),
        loop_locals: FxMap::default(),
        defers: Vec::new(),
        out: Summary::default(),
        summed: FxSet::default(),
        param_idx: ParamIdx::new(f),
    };
    let _ = fa.fid;
    let mut st = St { reach: true, ..Default::default() };
    // Parameters and other locals not bound by a `let` or pattern (vm
    // state) start pointing at their own pointee; bound ones are assigned
    // before they are read.
    let mut bound = Vec::new();
    bound_locals(b, &mut bound);
    let bound: FxSet<LocalId> = bound.into_iter().collect();
    for (l, _) in f.locals.iter().enumerate() {
        if bound.contains(&(l as LocalId)) {
            continue;
        }
        let v = fa.symbolic(l as LocalId);
        if !v.is_empty() {
            Rc::make_mut(&mut st.vals).insert(l as LocalId, Rc::new(v));
        }
    }
    let v = fa.eval(b, &mut st);
    if st.reach {
        fa.ret(&v, b.span);
        fa.exit_escapes(&st);
    }
    (fa.out, fa.errs, fa.lines)
}

/// Summarize every function (a least fixpoint over the call graph), then
/// check the functions of checked modules.
pub fn check(p: &Program, cx: &Ctx, sm: &crate::diag::SourceMap) -> Res<()> {
    let mode = std::env::var("LOIMI_LIFETIMES").unwrap_or_default();
    if mode == "off" {
        return Ok(());
    }
    let audit = (mode == "audit").then_some(sm);
    let t_start = std::time::Instant::now();
    let body = |i: FnId| p.fns[i as usize].body.is_some();
    let checked: Vec<FnId> = (0..p.fns.len() as FnId)
        .filter(|&i| body(i) && (audit.is_some() || cx.checked_files.contains(&p.fns[i as usize].span.file)))
        .collect();
    if checked.is_empty() {
        return Ok(());
    }
    // Only what checked functions call (transitively) needs a summary.
    // Post-order, so callees are summarized before their callers.
    let mut calls: FxMap<FnId, Vec<FnId>> = FxMap::default();
    let mut fns: Vec<FnId> = Vec::new();
    let mut seen: FxSet<FnId> = FxSet::default();
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
    let mut callers: FxMap<FnId, Vec<FnId>> = FxMap::default();
    for (&f, cs) in &calls {
        for &c in cs {
            callers.entry(c).or_default().push(f);
        }
    }
    if std::env::var("LOIMI_LIFETIMES_STATS").is_ok() {
        eprintln!("lifetimes: call graph {:.1} ms", t_start.elapsed().as_secs_f64() * 1000.0);
    }
    let mut sums: FxMap<FnId, Summary> = FxMap::default();
    let mut kinds = Kinds::default();
    // The worklist in post-order (callees first): a function whose callee's
    // summary changed waits until the rest of their cycle is done.
    let order: FxMap<FnId, usize> = fns.iter().enumerate().map(|(k, &f)| (f, k)).collect();
    let mut work: BTreeSet<(usize, FnId)> = fns.iter().enumerate().map(|(k, &f)| (k, f)).collect();
    let mut rounds = 0u64;
    // A function's last analysis saw the final summaries of its callees
    // (a change to one queues it again), so its errors are reported from
    // that analysis rather than from one more pass.
    let checked_set: FxSet<FnId> = checked.iter().copied().collect();
    let mut reports: FxMap<FnId, (Vec<Diag>, Vec<String>)> = FxMap::default();
    let stats = std::env::var("LOIMI_LIFETIMES_STATS").is_ok();
    let mut times: FxMap<FnId, (f64, u32)> = FxMap::default();
    while let Some((_, i)) = work.pop_first() {
        rounds += 1;
        let t0 = std::time::Instant::now();
        let report = checked_set.contains(&i);
        let (s, errs, lines) = analyze(p, i, &sums, &mut kinds, cx, report, audit);
        if report {
            reports.insert(i, (errs, lines));
        }
        if stats {
            let e = times.entry(i).or_default();
            e.0 += t0.elapsed().as_secs_f64();
            e.1 += 1;
        }
        let old = sums.get(&i).cloned().unwrap_or_default();
        let mut merged = old.clone();
        merged.restructs.extend(s.restructs.into_iter().filter(|r| !r.trunc));
        merged.ret.extend(s.ret);
        merged.pairs.extend(s.pairs);
        for (j, d) in s.keeps {
            merged.keeps.entry(j).or_default().extend(d);
        }
        if merged != old {
            sums.insert(i, merged);
            for &c in callers.get(&i).map(|v| v.as_slice()).unwrap_or(&[]) {
                work.insert((order[&c], c));
            }
        }
    }
    if std::env::var("LOIMI_LIFETIMES_STATS").is_ok() {
        eprintln!("lifetimes: {} functions, {} analyses", fns.len(), rounds);
        let mut v: Vec<_> = times.iter().collect();
        v.sort_by(|a, b| b.1.0.partial_cmp(&a.1.0).unwrap());
        let tot: f64 = v.iter().map(|x| x.1.0).sum();
        let mut acc = 0.0;
        for (k, x) in v.iter().enumerate() {
            acc += x.1.0;
            if [10usize, 50, 100, 200, 500, 1000].contains(&(k + 1)) {
                eprintln!("top {}: {:.0}%", k + 1, acc * 100.0 / tot);
            }
        }
        if let Ok(fname) = std::env::var("LOIMI_LT_DUMP") {
            for (f, s) in &sums {
                if p.fns[*f as usize].symbol == fname {
                    for r in &s.restructs {
                        eprintln!("R {:?} {:?} {}", r.root, &*r.steps, r.trunc);
                    }
                }
            }
        }
        for (f, (t, n)) in v.iter().take(30) {
            let s = sums.get(*f).cloned().unwrap_or_default();
            eprintln!("  {:8.1} ms {:3}x {} (restructs {} ret {} pairs {} keeps {})", t * 1000.0, n, p.fns[**f as usize].symbol, s.restructs.len(), s.ret.len(), s.pairs.len(), s.keeps.len());
        }
    }
    if stats {
        eprintln!("lifetimes: analyses {:.1} ms, pass {:.1} ms", times.values().map(|t| t.0).sum::<f64>() * 1000.0, t_start.elapsed().as_secs_f64() * 1000.0);
    }
    for &i in &checked {
        let Some((errs, lines)) = reports.remove(&i) else { continue };
        for l in lines {
            eprintln!("{}", l);
        }
        if let Some(d) = errs.into_iter().next() {
            return Err(d);
        }
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
