//! TIR → textual LLVM IR.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::ast::{BinOp, Linkage, UnOp};
use crate::diag::{SourceMap, Span};
use crate::layout;
use crate::tir::*;

#[derive(Clone, Debug)]
pub struct Options {
    /// Trap on integer overflow of `+ - *`.
    pub overflow_checks: bool,
    /// Count executed ops and op pairs; print a report at exit.
    pub vm_stats: bool,
    /// Building a library (`--lib`, `--shared`): no `main` wrapper.
    pub library: bool,
    /// `--jit-stencils`: hooks for the copy-and-patch JIT, and stencils.
    pub jit: bool,
    /// The target architecture (`--target`, else the host's).
    pub arch: Arch,
    /// `--gc-check-roots`: the collector validates every reference it
    /// marks, and traced slots are poisoned when unassigned
    /// (`crate::gcgen`, docs/lowering.md section 9).
    pub gc_check_roots: bool,
}

/// Target architectures. The IR is the same for both except where the
/// runtime needs the machine: the setjmp in `std.rt`'s handlers
/// (src/rtgen.rs).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arch {
    X86_64,
    AArch64,
}

impl Arch {
    pub fn host() -> Arch {
        if cfg!(target_arch = "aarch64") { Arch::AArch64 } else { Arch::X86_64 }
    }
}

/// Parameter or result attribute at a C-ABI boundary (`extern "C"`,
/// `export "C"`): C passes `bool` as a byte holding 0 or 1, so the side
/// that produces it must zero-extend the `i1`.
pub fn c_abi_attr(t: &Ty) -> &'static str {
    if *t == Ty::Bool { "zeroext " } else { "" }
}

/// First metadata number of the loop properties of vm code (numbered far
/// from the JIT's stencil metadata).
const LOOP_MD: u32 = 1_000_000;

pub struct Codegen<'p> {
    pub p: &'p Program,
    pub sm: &'p SourceMap,
    pub opts: Options,
    pub globals: String,
    pub fns: String,
    pub decls: Decls,
    pub str_count: u32,
    pub str_count_jc: u32,
    /// Emitting a counting copy (`--jit-stencils`).
    pub in_jit_copy: bool,
    pub mark_fns: std::collections::HashMap<String, String>,
    /// Outlined cold blocks waiting to be emitted: (handler, body,
    /// captured locals, symbol, of a counting copy).
    pub cold_queue: Vec<(FnId, TExpr, Vec<LocalId>, String, bool)>,
    pub cold_count: u32,
    /// `--vm-stats`: per vm, the counted cold blocks' labels (op or
    /// function, source location), and the index of each (function,
    /// block) pair.
    pub cold_stats: Vec<Vec<String>>,
    pub cold_stat_index: std::collections::HashMap<(FnId, Span), usize>,
    /// The same for counting copies (`--jit-stencils`).
    pub cold_count_jc: u32,
    /// Thread-local argument slots of shared `-> never` vm functions already defined.
    pub tls_defined: std::collections::HashSet<String>,
    /// Per function: immutable f64 locals bound only from the unboxed
    /// variant of a nanbox value (see `values::nan_safe`).
    pub nan_safe: std::collections::HashMap<FnId, std::collections::HashSet<LocalId>>,
    /// Trampolines that call a plain function as a `block fn`, by signature.
    pub tramps: std::collections::HashMap<String, String>,
    /// Loop IDs of loops in tail-call vm code (`TK::Loop`), appended to
    /// the module with the properties they share (`LOOP_MD`).
    pub loop_md: String,
    pub loop_md_count: u32,
    /// Whether `std.gc.barrier` exists (stores into objects call it); computed on first use.
    pub gc_barrier: Option<bool>,
    /// `--jit-stencils`: each handler's outlined cold blocks, in order.
    pub cold_syms: std::collections::HashMap<FnId, Vec<String>>,
    /// Stencil holes used (name → end of the symbol's value range).
    pub jit_holes: BTreeMap<String, u64>,
    /// `LOIMI_JIT_REGDELAY` (`jitgen::reg_delay`), read once.
    pub reg_delay: u32,
    /// Continuation holes used (name → declaration).
    pub jit_conts: BTreeMap<String, String>,
    /// The stencil functions (`--jit-stencils`).
    pub stencils: String,
    /// `--jit-stencils`: the handler-convention functions that get a
    /// counting copy (`jitgen::counted_fns`).
    pub jit_counted: std::collections::HashSet<FnId>,
    /// The program installs panic handlers (`rtgen::has_handlers`).
    pub panic_handlers: bool,
    /// `--gc-check-roots`: info records emitted (`gcgen::rootck_info`).
    pub rootck_count: u32,
}

/// External declarations, deduplicated by symbol.
#[derive(Default)]
pub struct Decls(pub BTreeMap<String, String>);

impl Decls {
    pub fn insert(&mut self, line: impl Into<String>) {
        let line = line.into();
        let name = line.split('@').nth(1).and_then(|s| s.split('(').next()).unwrap_or("").trim_matches('"').to_string();
        self.0.entry(name).or_insert(line);
    }
}

pub fn llty(p: &Program, t: &Ty) -> String {
    match t {
        Ty::Int(it) => format!("i{}", it.bits),
        Ty::F32 => "float".into(),
        Ty::F64 => "double".into(),
        Ty::Bool => "i1".into(),
        Ty::Unit | Ty::Never => "{}".into(),
        Ty::Ptr(_) | Ty::Fn(..) => "ptr".into(),
        Ty::Slice(_) => "{ ptr, i64 }".into(),
        Ty::Array(e, n) => format!("[{} x {}]", n, llty(p, e)),
        Ty::Option(inner) => {
            if inner.is_nullable_ptr() {
                "ptr".into()
            } else {
                format!("{{ i1, {} }}", llty(p, inner))
            }
        }
        Ty::Result(a, b) => format!("{{ i1, {}, {} }}", llty(p, a), llty(p, b)),
        Ty::Tuple(ts) => format!("{{ {} }}", ts.iter().map(|t| llty(p, t)).collect::<Vec<_>>().join(", ")),
        Ty::Struct(i) => format!("%\"{}\"", p.structs[*i as usize].name),
        Ty::Enum(i) => {
            let e = &p.enums[*i as usize];
            if e.has_payload() {
                format!("{{ i{}, [{} x i64] }}", e.tag.bits, layout::enum_payload_words(p, e))
            } else {
                format!("i{}", e.tag.bits)
            }
        }
        Ty::Value(i) => {
            if p.values[*i as usize].repr == ValueRepr::Boxed {
                "{ i64, i64 }".into()
            } else {
                "i64".into()
            }
        }
        Ty::Param(_) | Ty::Opaque(_) => unreachable!("type parameter in codegen"),
        Ty::Gc(_) => "ptr".into(),
        Ty::Block(..) | Ty::Closure(..) => "{ ptr, ptr }".into(),
    }
}

pub fn ret_llty(p: &Program, t: &Ty) -> String {
    if t.is_unitlike() { "void".into() } else { llty(p, t) }
}

pub fn float_lit(f: f64, t: &Ty) -> String {
    let v = if *t == Ty::F32 { (f as f32) as f64 } else { f };
    format!("0x{:016X}", v.to_bits())
}

fn cc_str(cc: CallConv) -> &'static str {
    match cc {
        CallConv::C => "",
        CallConv::PreserveNone => "preserve_nonecc ",
    }
}

impl<'p> Codegen<'p> {
    pub fn new(p: &'p Program, sm: &'p SourceMap, opts: Options) -> Codegen<'p> {
        Codegen {
            p,
            sm,
            opts,
            globals: String::new(),
            fns: String::new(),
            decls: Decls::default(),
            str_count: 0,
            str_count_jc: 0,
            in_jit_copy: false,
            mark_fns: Default::default(),
            gc_barrier: None,
            nan_safe: Default::default(),
            cold_queue: vec![],
            cold_count: 0,
            cold_stats: vec![Vec::new(); p.vms.len()],
            cold_stat_index: std::collections::HashMap::new(),
            cold_count_jc: 0,
            tls_defined: Default::default(),
            tramps: Default::default(),
            loop_md: String::new(),
            loop_md_count: 0,
            cold_syms: Default::default(),
            jit_holes: Default::default(),
            reg_delay: crate::jitgen::reg_delay(),
            jit_conts: Default::default(),
            stencils: String::new(),
            jit_counted: Default::default(),
            panic_handlers: false,
            rootck_count: 0,
        }
    }

    /// The thread-locals holding the location of the last call to a
    /// `#[track_caller]` function (`FnGen::store_caller`).
    pub fn caller_globals(&mut self) {
        for (g, t) in [("@loimi.caller_ptr", "ptr null"), ("@loimi.caller_len", "i64 0")] {
            if self.tls_defined.insert(g.to_string()) {
                let _ = writeln!(self.globals, "{} = internal thread_local(initialexec) global {}", g, t);
            }
        }
    }

    pub fn string_const(&mut self, s: &[u8]) -> String {
        // Counting copies (`--jit-stencils`) number their own, so that the
        // rest is named as in a default build.
        let name = if self.in_jit_copy {
            self.str_count_jc += 1;
            format!("@.str.jc.{}", self.str_count_jc - 1)
        } else {
            self.str_count += 1;
            format!("@.str.{}", self.str_count - 1)
        };
        let mut enc = String::new();
        for &b in s {
            if b.is_ascii_graphic() && b != b'"' && b != b'\\' || b == b' ' {
                enc.push(b as char);
            } else {
                let _ = write!(enc, "\\{:02X}", b);
            }
        }
        let _ = writeln!(self.globals, "{} = private unnamed_addr constant [{} x i8] c\"{}\"", name, s.len(), enc);
        name
    }

    /// An LLVM constant for a global's initializer (checked by
    /// `Checker::static_init_error`).
    pub fn const_init(&mut self, e: &TExpr) -> String {
        let p = self.p;
        if e.ty.is_unitlike() {
            return "zeroinitializer".into();
        }
        let typed = |cg: &mut Self, x: &TExpr| format!("{} {}", llty(p, &x.ty), cg.const_init(x));
        match &e.kind {
            TK::Int(v) => match &e.ty {
                Ty::Bool => (*v != 0).to_string(),
                Ty::F32 | Ty::F64 => float_lit(*v as f64, &e.ty),
                _ => v.to_string(),
            },
            TK::Float(f) => float_lit(*f, &e.ty),
            TK::Bool(b) => b.to_string(),
            TK::Str(s) => {
                let g = self.string_const(s);
                format!("{{ ptr {}, i64 {} }}", g, s.len())
            }
            TK::FnRef(id) => format!("@\"{}\"", p.fns[*id as usize].symbol),
            TK::Zeroed => "zeroinitializer".into(),
            TK::Struct(es) | TK::Tuple(es) => {
                if es.is_empty() {
                    return "zeroinitializer".into();
                }
                let parts: Vec<String> = es.iter().map(|x| typed(self, x)).collect();
                format!("{{ {} }}", parts.join(", "))
            }
            TK::Array(es) => {
                if es.is_empty() {
                    return "zeroinitializer".into();
                }
                let parts: Vec<String> = es.iter().map(|x| typed(self, x)).collect();
                format!("[{}]", parts.join(", "))
            }
            TK::ArrayRepeat(v, n) => {
                let x = self.const_init(v);
                if x == "zeroinitializer" || x == "0" || x == "false" || x == "null" || x == "0x0000000000000000" || *n == 0 {
                    return "zeroinitializer".into();
                }
                let t = llty(p, &v.ty);
                let parts: Vec<String> = (0..*n).map(|_| format!("{} {}", t, x)).collect();
                format!("[{}]", parts.join(", "))
            }
            TK::Variant(idx, args) => match &e.ty {
                Ty::Enum(eid) => {
                    let ed = &p.enums[*eid as usize];
                    let d = ed.variants[*idx as usize].discr;
                    if ed.has_payload() {
                        format!("{{ i{} {}, [{} x i64] zeroinitializer }}", ed.tag.bits, d, layout::enum_payload_words(p, ed))
                    } else {
                        d.to_string()
                    }
                }
                Ty::Option(inner) => {
                    if inner.is_nullable_ptr() {
                        if *idx == 0 { "null".into() } else { self.const_init(&args[0]) }
                    } else if *idx == 0 {
                        "zeroinitializer".into()
                    } else {
                        format!("{{ i1 true, {} }}", typed(self, &args[0]))
                    }
                }
                Ty::Result(a, b) => {
                    let (at, bt) = (llty(p, a), llty(p, b));
                    let v = typed(self, &args[0]);
                    if *idx == 0 {
                        format!("{{ i1 false, {}, {} zeroinitializer }}", v, bt)
                    } else {
                        format!("{{ i1 true, {} zeroinitializer, {} }}", at, v)
                    }
                }
                _ => unreachable!("checked by static_init_error"),
            },
            _ => unreachable!("checked by static_init_error"),
        }
    }

    pub fn location(&self, span: Span) -> String {
        // User text in a vm's expansion (a `verify` clause, ...): where the
        // user wrote it.
        let span = self.sm.user_span(span);
        if (span.file as usize) < self.sm.files.len() {
            let (l, c) = self.sm.line_col(span.file, span.lo);
            format!("{}:{}:{}", self.sm.files[span.file as usize].path, l, c)
        } else {
            "?".into()
        }
    }

    /// Emit the outlined cold blocks queued so far (into `to`, or the
    /// module's functions).
    fn emit_cold_queue(&mut self, mut to: Option<&mut String>) {
        while let Some((fid, body, captured, sym, count)) = self.cold_queue.pop() {
            let mut g = FnGen::new(self, fid);
            g.cold = Some((body, captured, sym));
            g.jit_count = count;
            let text = g.run();
            match to.as_deref_mut() {
                Some(t) => t.push_str(&text),
                None => self.fns.push_str(&text),
            }
        }
    }

    /// The program's IR and, with `--jit-stencils`, the stencil functions.
    pub fn generate_with_stencils(mut self) -> (String, String) {
        let p = self.p;
        if self.opts.jit {
            if let Ok(vi) = crate::jitgen::jit_vm(p) {
                self.jit_counted = crate::jitgen::counted_fns(p, vi);
            }
        }
        let mut out = String::new();
        let _ = writeln!(out, "; generated by loimi");
        let mut lazy_types = Vec::new();
        for s in p.structs.iter().filter(|s| !s.pattern) {
            let fs: Vec<String> = s.fields.iter().map(|(_, t)| llty(p, t)).collect();
            let line = format!("%\"{}\" = type {{ {} }}\n", s.name, fs.join(", "));
            if LAZY_STD_TYPES.contains(&s.name.as_str()) {
                lazy_types.push((format!("%\"{}\"", s.name), line));
            } else {
                out.push_str(&line);
            }
        }
        for g in &p.globals {
            let init = self.const_init(&g.init);
            let tl = if g.thread_local { "thread_local(initialexec) " } else { "" };
            let align = layout::align_of(p, &g.ty).max(1);
            let _ = writeln!(self.globals, "@\"{}\" = internal {}global {} {}, align {}", g.symbol, tl, llty(p, &g.ty), init, align);
        }
        let keep = lazy_used(p, self.opts.gc_check_roots);
        for (i, f) in p.fns.iter().enumerate() {
            if is_lazy(f) && !keep[i] {
                continue;
            }
            if f.body.is_none() {
                if f.linkage == Linkage::Extern {
                    let mut ps: Vec<String> = f.param_tys.iter().map(|t| format!("{} {}", llty(p, t), c_abi_attr(t)).trim_end().to_string()).collect();
                    if f.variadic {
                        ps.push("...".into());
                    }
                    self.decls.insert(format!("declare {}{} @\"{}\"({})", c_abi_attr(&f.ret), ret_llty(p, &f.ret), f.symbol, ps.join(", ")));
                }
                continue;
            }
            if !crate::vmgen::standalone(p, f) {
                continue;
            }
            let text = FnGen::new(&mut self, i as FnId).run();
            self.fns.push_str(&text);
            self.emit_cold_queue(None);
        }
        // `--jit-stencils`: the functions that count for the JIT get a
        // second, counting copy (`jitgen::counted_fns`). They go last, so
        // that the rest of the program is laid out as in a default build.
        let mut copies = String::new();
        let mut counted: Vec<FnId> = self.jit_counted.iter().copied().collect();
        counted.sort();
        self.in_jit_copy = true;
        for fid in counted {
            let mut g = FnGen::new(&mut self, fid);
            g.jit_count = true;
            copies.push_str(&g.run());
            self.emit_cold_queue(Some(&mut copies));
        }
        self.in_jit_copy = false;
        // Program entry wrapper. Libraries have none: the host calls the
        // `export "C"` functions, and `os.arg_count()` is 0.
        if let Some(m) = p.fns.iter().find(|f| f.symbol == "main.main" && f.body.is_some() && !self.opts.library) {
            let ret = &m.ret;
            let _ = writeln!(self.fns, "define i32 @main(i32 %argc, ptr %argv) {{");
            let _ = writeln!(self.fns, "  store i32 %argc, ptr @loimi.argc");
            let _ = writeln!(self.fns, "  store ptr %argv, ptr @loimi.argv");
            match ret {
                Ty::Int(it) if it.bits == 32 => {
                    let _ = writeln!(self.fns, "  %r = call i32 @\"main.main\"()\n  ret i32 %r");
                }
                _ => {
                    let _ = writeln!(self.fns, "  call {} @\"main.main\"()\n  ret i32 0", ret_llty(p, ret));
                }
            }
            let _ = writeln!(self.fns, "}}");
        }
        if self.opts.jit {
            crate::jitgen::emit_stencils(&mut self);
            if let Ok(vi) = crate::jitgen::jit_vm(p) {
                crate::jitgen::emit_runtime(p, vi, &mut self.globals, &mut self.fns);
            }
        }
        crate::vmgen::emit_tables(p, &mut self.globals, &mut self.fns, self.opts.jit, &self.jit_counted);
        crate::gcgen::emit_runtime(&mut self);
        crate::rtgen::emit_runtime(&mut self);
        crate::vmgen::emit_stats(&mut self);
        self.decls.insert("declare void @llvm.trap() noreturn");
        let _ = writeln!(self.globals, "@loimi.argc = internal global i32 0");
        let _ = writeln!(self.globals, "@loimi.argv = internal global ptr null");
        let _ = writeln!(self.fns, "define internal i32 @loimi_argc() {{\n  %v = load i32, ptr @loimi.argc\n  ret i32 %v\n}}");
        let _ = writeln!(self.fns, "define internal ptr @loimi_argv() {{\n  %v = load ptr, ptr @loimi.argv\n  ret ptr %v\n}}");
        self.decls.0.remove("loimi_argc");
        self.decls.0.remove("loimi_argv");
        // Runtime support: panic (`rtgen::panic_body`).
        let body = crate::rtgen::panic_body(&self);
        let _ = writeln!(self.fns, "define internal void @loimi.panic(ptr %msg, i64 %len) cold noreturn nounwind noinline {{\n{}}}", body);
        self.decls.insert("declare i64 @write(i32, ptr, i64)");
        self.decls.insert("declare void @abort() noreturn");
        out.push_str(&self.globals);
        out.push_str(&self.fns);
        out.push_str(&copies);
        for d in self.decls.0.values() {
            let _ = writeln!(out, "{}", d);
        }
        if self.loop_md_count > 0 {
            // What clang emits for `#pragma clang loop vectorize(disable)
            // interleave(disable) unroll(disable)`.
            let _ = writeln!(out, "!{} = !{{!\"llvm.loop.vectorize.width\", i32 1}}", LOOP_MD);
            let _ = writeln!(out, "!{} = !{{!\"llvm.loop.interleave.count\", i32 1}}", LOOP_MD + 1);
            let _ = writeln!(out, "!{} = !{{!\"llvm.loop.unroll.disable\"}}", LOOP_MD + 2);
            out.push_str(&self.loop_md);
        }
        // std types few programs use: defined only if something refers to them.
        for (name, line) in lazy_types {
            if out.contains(&name) {
                let at = out.find('\n').map_or(0, |i| i + 1);
                out.insert_str(at, &line);
            }
        }
        (out, std::mem::take(&mut self.stencils))
    }
}

struct LoopTarget {
    id: u32,
    brk: String,
    cont: String,
    slot: Option<String>,
    defer_depth: usize,
}

pub struct FnGen<'a, 'p> {
    pub cg: &'a mut Codegen<'p>,
    pub fid: FnId,
    pub allocas: String,
    pub body: String,
    pub tmp: u32,
    pub terminated: bool,
    pub slots: Vec<String>,
    loops: Vec<LoopTarget>,
    defers: Vec<Vec<TExpr>>,
    /// VM handler state (set by vmgen when generating handlers).
    pub vm: Option<crate::vmgen::HandlerGen>,
    /// Locals in scope (and initialized), innermost last: roots at calls that may collect.
    pub live: Vec<Vec<LocalId>>,
    /// Generating an outlined cold block of this handler: (body, captured locals, symbol).
    pub cold: Option<(TExpr, Vec<LocalId>, String)>,
    /// Traced values computed before defers run (a block's tail, a return
    /// value), spilled to these slots: roots like `live` locals.
    pub pending: Vec<(String, Ty)>,
    /// Emitting a stencil of this handler (`--jit-stencils`).
    pub stencil: Option<crate::jitgen::StencilGen>,
    /// Emitting the counting copy of this function (`--jit-stencils`).
    pub jit_count: bool,
    /// A `#[track_caller]` function: its caller's location is in
    /// `%caller.ptr` and `%caller.len` (`store_caller`).
    caller_saved: bool,
}

impl<'a, 'p> FnGen<'a, 'p> {
    pub fn new(cg: &'a mut Codegen<'p>, fid: FnId) -> FnGen<'a, 'p> {
        FnGen {
            cg,
            fid,
            allocas: String::new(),
            body: String::new(),
            tmp: 0,
            terminated: false,
            slots: vec![],
            loops: vec![],
            defers: vec![],
            vm: None,
            live: vec![vec![]],
            cold: None,
            pending: vec![],
            stencil: None,
            jit_count: false,
            caller_saved: false,
        }
    }

    pub fn p(&self) -> &'p Program {
        self.cg.p
    }
    pub fn ty(&self, t: &Ty) -> String {
        llty(self.cg.p, t)
    }
    pub fn f(&self) -> &'p FnDef {
        &self.cg.p.fns[self.fid as usize]
    }

    pub fn fresh(&mut self) -> String {
        self.tmp += 1;
        format!("%t{}", self.tmp)
    }
    pub fn fresh_label(&mut self, hint: &str) -> String {
        self.tmp += 1;
        format!("{}{}", hint, self.tmp)
    }

    pub fn emit(&mut self, s: impl AsRef<str>) {
        if self.terminated {
            let l = self.fresh_label("dead");
            let _ = writeln!(self.body, "{}:", l);
            self.terminated = false;
        }
        let _ = writeln!(self.body, "  {}", s.as_ref());
    }
    /// Emit an instruction that produces a value.
    pub fn inst(&mut self, s: impl AsRef<str>) -> String {
        let t = self.fresh();
        self.emit(format!("{} = {}", t, s.as_ref()));
        t
    }
    pub fn term(&mut self, s: impl AsRef<str>) {
        self.emit(s);
        self.terminated = true;
    }
    pub fn label(&mut self, l: &str) {
        if !self.terminated {
            let _ = writeln!(self.body, "  br label %{}", l);
        }
        let _ = writeln!(self.body, "{}:", l);
        self.terminated = false;
    }
    pub fn br(&mut self, l: &str) {
        self.term(format!("br label %{}", l));
    }
    pub fn cond_br(&mut self, c: &str, t: &str, f: &str) {
        self.term(format!("br i1 {}, label %{}, label %{}", c, t, f));
    }
    pub fn alloca(&mut self, t: &Ty) -> String {
        let name = self.fresh();
        let lt = self.ty(t);
        let _ = writeln!(self.allocas, "  {} = alloca {}, align {}", name, lt, layout::align_of(self.p(), t).max(1));
        name
    }
    /// The stack slot of a local. With `--gc-check-roots`, a traced one
    /// starts out poisoned (`poison_slot`).
    pub fn local_slot(&mut self, t: &Ty) -> String {
        let s = self.alloca(t);
        if crate::gcgen::check_roots(self) && crate::gcgen::is_traced(self.p(), t) {
            let size = layout::size_of(self.p(), t);
            self.cg.decls.insert("declare void @llvm.memset.p0.i64(ptr, i8, i64, i1)");
            let _ = writeln!(self.allocas, "  call void @llvm.memset.p0.i64(ptr {}, i8 1, i64 {}, i1 false)", s, size);
        }
        s
    }

    /// `--gc-check-roots`: fill the slot of a traced local with 0x01
    /// bytes, a non-canonical address in every pointer (and `Some` in
    /// every option flag), where it is unassigned: at function entry and
    /// when its scope ends. A frame that lists it there makes the
    /// collector report it, whatever the stack held.
    fn poison_slot(&mut self, l: LocalId) {
        let t = self.f().locals[l as usize].ty.clone();
        if !crate::gcgen::is_traced(self.p(), &t) {
            return;
        }
        let size = layout::size_of(self.p(), &t);
        let slot = self.slots[l as usize].clone();
        self.cg.decls.insert("declare void @llvm.memset.p0.i64(ptr, i8, i64, i1)");
        self.emit(format!("call void @llvm.memset.p0.i64(ptr {}, i8 1, i64 {}, i1 false)", slot, size));
    }

    /// Close the innermost scope of `live`; with `--gc-check-roots`,
    /// poison its traced locals on the path that falls out of it.
    fn pop_live(&mut self) {
        let scope = self.live.pop().unwrap();
        if crate::gcgen::check_roots(self) && !self.terminated {
            for l in scope {
                self.poison_slot(l);
            }
        }
    }

    pub fn load(&mut self, t: &Ty, ptr: &str) -> String {
        if t.is_unitlike() {
            return "zeroinitializer".into();
        }
        let lt = self.ty(t);
        self.inst(format!("load {}, ptr {}", lt, ptr))
    }
    pub fn store(&mut self, t: &Ty, v: &str, ptr: &str) {
        if t.is_unitlike() {
            return;
        }
        if v == "zeroinitializer" {
            let size = layout::size_of(self.p(), t);
            if size > 64 {
                self.cg.decls.insert("declare void @llvm.memset.p0.i64(ptr, i8, i64, i1)");
                self.emit(format!("call void @llvm.memset.p0.i64(ptr {}, i8 0, i64 {}, i1 false)", ptr, size));
                return;
            }
        }
        let lt = self.ty(t);
        self.emit(format!("store {} {}, ptr {}", lt, v, ptr));
    }

    pub fn run(mut self) -> String {
        let f = self.f();
        let p = self.p();
        // A closure shares its enclosing function's local table: give stack
        // slots only to the locals it declares, not to captured ones.
        let needed: Option<Vec<LocalId>> = f.env.as_ref().map(|caps| {
            let mut used = Vec::new();
            let mut declared = f.params.clone();
            crate::tirwalk::locals(f.body.as_ref().unwrap(), &mut used, &mut declared);
            used.extend(declared);
            used.retain(|l| !caps.contains(l));
            used.extend(f.env_root);
            used
        });
        for (i, l) in f.locals.iter().enumerate() {
            if needed.as_ref().is_some_and(|n| !n.contains(&(i as LocalId))) {
                self.slots.push("%unused".into());
                continue;
            }
            let s = self.local_slot(&l.ty);
            self.slots.push(s);
        }
        let mut params = Vec::new();
        for (i, (&lid, t)) in f.params.iter().zip(f.param_tys.iter()).enumerate() {
            let mut pname = format!("%p{}", i);
            if let Ty::Block(..) = t {
                // Code and environment arrive as two scalars, so that LLVM
                // can specialize the function for a constant code pointer.
                params.push(format!("ptr {}.code, ptr {}.env", pname, pname));
                let a = self.inst(format!("insertvalue {{ ptr, ptr }} poison, ptr {}.code, 0", pname));
                pname = self.inst(format!("insertvalue {{ ptr, ptr }} {}, ptr {}.env, 1", a, pname));
            } else {
                let attr = if f.linkage == Linkage::Export && self.cold.is_none() { c_abi_attr(t) } else { "" };
                params.push(format!("{} {}{}", llty(p, t), attr, pname));
            }
            let slot = self.slots[lid as usize].clone();
            self.store(t, &pname, &slot);
            // Vm state arrives in parameters in tail-call mode: it is
            // rooted at calls that may collect by copy (`state_roots`),
            // not through its own slot, which then stays in a register.
            if !f.vm_state.contains(&lid) {
                self.live[0].push(lid);
            }
        }
        if let Some(x) = crate::vmgen::extra_param(p, f) {
            params.push(x);
        }
        if let (Some(caps), Some((sid, first))) = (&f.env, f.env_struct) {
            // An escaping closure: the captured values are fields of the
            // environment struct, used in place.
            params.insert(0, "ptr %env".to_string());
            let st = llty(p, &Ty::Struct(sid));
            for (i, &l) in caps.iter().enumerate() {
                self.slots[l as usize] = self.inst(format!("getelementptr inbounds {}, ptr %env, i32 0, i32 {}", st, first as usize + i));
            }
        } else if let Some(caps) = &f.env {
            // The environment: pointers to the captured locals' slots, or
            // (captured by value) the values themselves.
            params.insert(0, "ptr %env".to_string());
            for (i, &l) in caps.iter().enumerate() {
                let gp = self.inst(format!("getelementptr inbounds [{} x ptr], ptr %env, i64 0, i64 {}", caps.len(), i));
                self.slots[l as usize] = if f.env_val[i] { gp } else { self.inst(format!("load ptr, ptr {}", gp)) };
            }
        }
        if let Some(r) = f.env_root {
            // A GC closure's environment object stays rooted in the body.
            let slot = self.slots[r as usize].clone();
            self.emit(format!("store ptr %env, ptr {}", slot));
            self.live[0].push(r);
        }
        crate::vmgen::handler_prologue(&mut self);
        crate::gcgen::zero_traced_state(&mut self);
        if f.track_caller && self.cold.is_none() {
            self.cg.caller_globals();
            self.emit("%caller.ptr = load ptr, ptr @loimi.caller_ptr");
            self.emit("%caller.len = load i64, ptr @loimi.caller_len");
            self.caller_saved = true;
        }
        let cold = self.cold.take();
        let body = match &cold {
            Some((b, captured, sym)) => {
                // Captured locals arrive through thread-local slots.
                for &l in captured {
                    let t = f.locals[l as usize].ty.clone();
                    let v = self.load(&t, &format!("@\"{}.c{}\"", sym, l));
                    let slot = self.slots[l as usize].clone();
                    self.store(&t, &v, &slot);
                    self.live[0].push(l);
                }
                b
            }
            None => f.body.as_ref().unwrap(),
        };
        let v = self.val(body);
        let through_slot = crate::vmgen::returns_through_slot(p, f);
        if !self.terminated {
            if f.ret.is_unitlike() || through_slot {
                self.term("ret void");
            } else {
                let rt = self.ty(&f.ret);
                self.term(format!("ret {} {}", rt, v));
            }
        }
        if !self.terminated {
            self.term("unreachable");
        }
        crate::vmgen::epilogue(&mut self);
        // `#[catch_panic]`: the body is internal `sym.body`, called by the
        // exported wrapper (`rtgen::emit_catch_wrappers`) and inlined into
        // it: a separate call cost a regex match about 1 ns more.
        let catch_body = f.catch_panic.is_some() && cold.is_none() && self.stencil.is_none() && !self.jit_count;
        let linkage = if (f.linkage == Linkage::Export && !catch_body) || self.stencil.is_some() { "" } else { "internal " };
        let mut attrs = vec!["nounwind"];
        if catch_body {
            attrs.push("alwaysinline");
        }
        if self.stencil.is_some() {
            attrs.push("\"no-jump-tables\"=\"true\"");
        }
        if cold.is_some() {
            attrs.push("cold");
            attrs.push("noinline");
        }
        // Tail-call handlers: LLVM must not turn a loop into a call of
        // memset/memcpy/memmove (loop idiom recognition), which would
        // give the handler a stack frame and spills on every path. Their cold
        // blocks and super resume functions get them too: LLVM inlines a
        // callee only into a caller with at least its `no-builtin` set,
        // and cold blocks inline handlers and resume functions. Shared
        // copies of `-> never` vm functions (called from cold paths) may
        // still call memset.
        if matches!(f.vm_role, VmRole::Handler(_) | VmRole::Resume(..))
            && f.vm.is_some_and(|v| p.vms[v as usize].dispatch == crate::tir::Dispatch::TailCall)
        {
            attrs.push("\"no-builtin-memset\" \"no-builtin-memcpy\" \"no-builtin-memmove\"");
        }
        let symbol = match (&cold, &self.stencil) {
            (Some((_, _, s)), _) => s.clone(),
            (None, Some(st)) => st.symbol.clone(),
            (None, None) if self.jit_count => crate::jitgen::counting_symbol(&f.symbol),
            (None, None) if catch_body => format!("{}.body", f.symbol),
            (None, None) => f.symbol.clone(),
        };
        match f.inline {
            Some(true) => attrs.push("alwaysinline"),
            Some(false) => attrs.push("noinline"),
            None => {}
        }
        if f.cold {
            attrs.push("cold");
            if f.inline.is_none() {
                attrs.push("noinline");
            }
        }
        let mut out = String::new();
        let _ = writeln!(
            out,
            "define {}{}{}{} @\"{}\"({}) {} {{",
            linkage,
            cc_str(f.cc),
            if f.linkage == Linkage::Export && cold.is_none() && !through_slot { c_abi_attr(&f.ret) } else { "" },
            if through_slot { "void".to_string() } else { ret_llty(p, &f.ret) },
            symbol,
            params.join(", "),
            attrs.join(" ")
        );
        let _ = writeln!(out, "entry:");
        out.push_str(&self.allocas);
        let _ = writeln!(out, "  br label %start");
        let _ = writeln!(out, "start:");
        match crate::vmgen::depth_global(p, f) {
            // A nested entry counts the runs active on this thread in
            // `Vm.depth`: one more until it returns, by any path.
            Some(d) if cold.is_none() => {
                let _ = writeln!(out, "  %depth.saved = load i32, ptr {}", d);
                let _ = writeln!(out, "  %depth.inc = add i32 %depth.saved, 1");
                let _ = writeln!(out, "  store i32 %depth.inc, ptr {}", d);
                for line in self.body.lines() {
                    if line.starts_with("  ret ") {
                        let _ = writeln!(out, "  store i32 %depth.saved, ptr {}", d);
                    }
                    out.push_str(line);
                    out.push('\n');
                }
            }
            _ => out.push_str(&self.body),
        }
        let _ = writeln!(out, "}}\n");
        out
    }

    /// Before a call to a `#[track_caller]` function (or `test.caller()`'s
    /// runtime half): record where the call is, or, in a `#[track_caller]`
    /// function, pass on where it was called from.
    fn store_caller(&mut self, span: Span) {
        self.cg.caller_globals();
        let (ptr, len) = if self.caller_saved {
            ("%caller.ptr".to_string(), "%caller.len".to_string())
        } else if self.f().track_caller {
            // An outlined cold block: the location stays the caller's.
            return;
        } else {
            let loc = self.cg.location(span);
            let len = loc.len();
            (self.cg.string_const(loc.as_bytes()), len.to_string())
        };
        self.emit(format!("store ptr {}, ptr @loimi.caller_ptr", ptr));
        self.emit(format!("store i64 {}, ptr @loimi.caller_len", len));
    }

    // ── panics ──────────────────────────────────────────────────────

    pub fn panic(&mut self, msg: &str, span: Span) {
        let text = format!("{}: {}\n", self.cg.location(span), msg);
        let g = self.cg.string_const(text.as_bytes());
        self.emit(format!("call void @loimi.panic(ptr {}, i64 {})", g, text.len()));
        self.term("unreachable");
    }

    /// Branch to a panic if `cond` (i1) is true.
    pub fn check(&mut self, cond: &str, msg: &str, span: Span) {
        let bad = self.fresh_label("panic");
        let ok = self.fresh_label("ok");
        let c = self.inst(format!("call i1 @llvm.expect.i1(i1 {}, i1 false)", cond));
        self.cg.decls.insert("declare i1 @llvm.expect.i1(i1, i1)");
        self.cond_br(&c, &bad, &ok);
        self.label(&bad);
        self.panic(msg, span);
        self.label(&ok);
    }

    // ── places ──────────────────────────────────────────────────────

    pub fn place(&mut self, e: &TExpr) -> String {
        if self.stencil.is_some() {
            if let Some(t) = crate::jitgen::const_read(self, e) {
                return t;
            }
        }
        match &e.kind {
            TK::Local(id) => self.slots[*id as usize].clone(),
            TK::Global(id) => format!("@\"{}\"", self.p().globals[*id as usize].symbol),
            TK::Deref(p) => self.val(p),
            TK::Field(b, i) => {
                let base = self.place(b);
                let bt = self.ty(&b.ty);
                self.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 {}", bt, base, i))
            }
            TK::Index(b, idx) => {
                let elem = self.ty(&e.ty);
                match &b.ty {
                    Ty::Array(_, n) => {
                        let n = *n;
                        let base = self.place(b);
                        let i = self.index_val(idx);
                        let bad = self.inst(format!("icmp uge i64 {}, {}", i, n));
                        self.check(&bad, "index out of bounds", e.span);
                        let bt = self.ty(&b.ty);
                        self.inst(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", bt, base, i))
                    }
                    Ty::Slice(_) => {
                        let s = self.val(b);
                        let i = self.index_val(idx);
                        let ptr = self.inst(format!("extractvalue {{ ptr, i64 }} {}, 0", s));
                        let len = self.inst(format!("extractvalue {{ ptr, i64 }} {}, 1", s));
                        let bad = self.inst(format!("icmp uge i64 {}, {}", i, len));
                        self.check(&bad, "index out of bounds", e.span);
                        self.inst(format!("getelementptr inbounds {}, ptr {}, i64 {}", elem, ptr, i))
                    }
                    Ty::Ptr(_) => {
                        let ptr = self.val(b);
                        let i = self.index_val(idx);
                        self.inst(format!("getelementptr inbounds {}, ptr {}, i64 {}", elem, ptr, i))
                    }
                    _ => unreachable!(),
                }
            }
            TK::Vm(TVm::Word(off)) => crate::vmgen::word_place(self, *off),
            TK::Vm(TVm::WordAt(h, bytes, _)) => {
                let p = self.val(h);
                if *bytes == 0 {
                    p
                } else {
                    self.inst(format!("getelementptr inbounds i8, ptr {}, i64 {}", p, bytes))
                }
            }
            _ => {
                let v = self.val(e);
                let t = self.alloca(&e.ty);
                self.store(&e.ty, &v, &t);
                t
            }
        }
    }

    /// Index value, extended to i64.
    pub fn index_val(&mut self, idx: &TExpr) -> String {
        let v = self.val(idx);
        match &idx.ty {
            Ty::Int(it) if it.bits < 64 => {
                let op = if it.signed { "sext" } else { "zext" };
                self.inst(format!("{} i{} {} to i64", op, it.bits, v))
            }
            _ => v,
        }
    }

    // ── values ──────────────────────────────────────────────────────

    pub fn poison(&self, t: &Ty) -> String {
        if t.is_unitlike() { "zeroinitializer".into() } else { "poison".into() }
    }

    pub fn val(&mut self, e: &TExpr) -> String {
        match &e.kind {
            TK::Int(v) => match &e.ty {
                Ty::Value(_) | Ty::Int(_) => v.to_string(),
                Ty::Bool => (*v != 0).to_string(),
                _ => v.to_string(),
            },
            TK::Float(f) => float_lit(*f, &e.ty),
            TK::Bool(b) => b.to_string(),
            TK::Str(s) => {
                let g = self.cg.string_const(s);
                format!("{{ ptr {}, i64 {} }}", g, s.len())
            }
            TK::Unit | TK::Zeroed => "zeroinitializer".into(),
            TK::Local(_) | TK::Global(_) | TK::Deref(_) | TK::Index(..) => {
                if self.stencil.is_some() {
                    if let Some(v) = crate::jitgen::delayed_reg_read(self, e) {
                        return v;
                    }
                }
                let p = self.place(e);
                self.load(&e.ty, &p)
            }
            TK::Field(b, i) => {
                if crate::check::is_place(b) {
                    let p = self.place(e);
                    self.load(&e.ty, &p)
                } else {
                    let v = self.val(b);
                    if e.ty.is_unitlike() {
                        return "zeroinitializer".into();
                    }
                    let bt = self.ty(&b.ty);
                    self.inst(format!("extractvalue {} {}, {}", bt, v, i))
                }
            }
            TK::FnRef(id) => format!("@\"{}\"", self.p().fns[*id as usize].symbol),
            TK::Closure(fid, caps) => self.closure(*fid, caps),
            TK::NewClosure(fid, caps, mem) => self.new_closure(&e.ty, *fid, caps, mem),
            TK::AddrOf(inner) => self.place(inner),
            TK::SliceRange(b, lo, hi) => self.slice_range(e, b, lo.as_deref(), hi.as_deref()),
            TK::Unary(op, inner) => {
                let v = self.val(inner);
                let t = self.ty(&e.ty);
                match op {
                    UnOp::Neg if e.ty.is_float() => self.inst(format!("fneg {} {}", t, v)),
                    UnOp::Neg => self.inst(format!("sub {} 0, {}", t, v)),
                    UnOp::Not if e.ty == Ty::Bool => self.inst(format!("xor i1 {}, true", v)),
                    UnOp::Not => self.inst(format!("xor {} {}, -1", t, v)),
                    _ => unreachable!(),
                }
            }
            TK::Binary(op, a, b) => self.binary(e, *op, a, b),
            TK::Cast(inner) if inner.ty.is_int() && matches!(e.ty, Ty::Enum(_)) => self.int_to_enum(inner, e),
            TK::Cast(inner) => self.cast(inner, &e.ty),
            TK::Call(f, args) => self.call(e, f, args),
            TK::Intrinsic(i, args) => self.intrinsic(e, *i, args),
            TK::Struct(es) | TK::Tuple(es) | TK::Array(es) => {
                let t = self.ty(&e.ty);
                let mut acc = "poison".to_string();
                if es.is_empty() {
                    return "zeroinitializer".into();
                }
                for (i, x) in es.iter().enumerate() {
                    let v = self.val(x);
                    let xt = self.ty(&x.ty);
                    acc = self.inst(format!("insertvalue {} {}, {} {}, {}", t, acc, xt, v, i));
                }
                acc
            }
            TK::ArrayRepeat(v, n) => {
                if matches!(v.kind, TK::Int(0) | TK::Bool(false) | TK::Zeroed) || matches!(v.kind, TK::Float(f) if f.to_bits() == 0) {
                    return "zeroinitializer".into();
                }
                let x = self.val(v);
                let slot = self.alloca(&e.ty);
                let et = self.ty(&v.ty);
                let at = self.ty(&e.ty);
                let head = self.fresh_label("rep");
                let body = self.fresh_label("repb");
                let done = self.fresh_label("repd");
                let ctr = self.alloca(&Ty::i64());
                self.emit(format!("store i64 0, ptr {}", ctr));
                self.label(&head);
                let i = self.inst(format!("load i64, ptr {}", ctr));
                let c = self.inst(format!("icmp ult i64 {}, {}", i, n));
                self.cond_br(&c, &body, &done);
                self.label(&body);
                let p = self.inst(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", at, slot, i));
                self.emit(format!("store {} {}, ptr {}", et, x, p));
                let i2 = self.inst(format!("add i64 {}, 1", i));
                self.emit(format!("store i64 {}, ptr {}", i2, ctr));
                self.br(&head);
                self.label(&done);
                self.load(&e.ty, &slot)
            }
            TK::Variant(idx, args) => self.variant(&e.ty, *idx, args),
            TK::Block(stmts, tail) => {
                self.defers.push(vec![]);
                self.live.push(vec![]);
                for s in stmts {
                    self.stmt(s);
                }
                let v = match tail {
                    Some(t) => self.val(t),
                    None => self.poison(&e.ty),
                };
                // The block's locals, and its value, stay rooted while its
                // defers run.
                let ds = self.defers.pop().unwrap();
                if !self.terminated && !ds.is_empty() {
                    let rooted = self.root_pending(&v, &e.ty);
                    for d in ds.iter().rev() {
                        self.val(d);
                    }
                    self.unroot_pending(rooted);
                }
                self.pop_live();
                v
            }
            TK::If(conds, then, els) => {
                let slot = if e.ty.is_unitlike() { None } else { Some(self.alloca(&e.ty)) };
                let l_then = self.fresh_label("then");
                let l_else = self.fresh_label("else");
                let l_end = self.fresh_label("endif");
                self.live.push(vec![]);
                self.conds(conds, &l_else);
                self.label(&l_then);
                let v = self.val(then);
                self.pop_live();
                if let Some(s) = &slot {
                    if !self.terminated {
                        self.store(&e.ty, &v, s);
                    }
                }
                self.br(&l_end);
                self.label(&l_else);
                if let Some(els) = els {
                    let v = self.val(els);
                    if let Some(s) = &slot {
                        if !self.terminated {
                            self.store(&e.ty, &v, s);
                        }
                    }
                }
                self.br(&l_end);
                self.label(&l_end);
                match slot {
                    Some(s) => self.load(&e.ty, &s),
                    None => "zeroinitializer".into(),
                }
            }
            TK::Match(scrut, arms) => {
                let sp = if crate::check::is_place(scrut) && !matches!(scrut.kind, TK::Vm(_)) {
                    let p = self.place(scrut);
                    // Copy so arms can't observe mutation of the scrutinee.
                    let v = self.load(&scrut.ty, &p);
                    let t = self.alloca(&scrut.ty);
                    self.store(&scrut.ty, &v, &t);
                    t
                } else {
                    self.place(scrut)
                };
                let slot = if e.ty.is_unitlike() { None } else { Some(self.alloca(&e.ty)) };
                let l_end = self.fresh_label("endmatch");
                // Leading string-literal arms: switch on the length first.
                let mut first = 0;
                if scrut.ty == Ty::str() {
                    let k = arms.iter().take_while(|a| a.guard.is_none() && str_lits(&a.pat).is_some()).count();
                    if k >= 2 {
                        self.str_switch(&scrut.ty, &sp, &arms[..k], &e.ty, slot.as_deref(), &l_end);
                        first = k;
                    }
                }
                for arm in &arms[first..] {
                    let l_next = self.fresh_label("arm");
                    self.live.push(vec![]);
                    self.test_pat(&arm.pat, &sp, &l_next);
                    if let Some(g) = &arm.guard {
                        // A guard that collects runs before later arms
                        // bind from the scrutinee, which may be a
                        // temporary that nothing else references: it is
                        // rooted while the guard runs.
                        let root = crate::gcgen::is_traced(self.p(), &scrut.ty);
                        if root {
                            self.pending.push((sp.clone(), scrut.ty.clone()));
                        }
                        let c = self.val(g);
                        if root {
                            self.pending.pop();
                        }
                        let l_ok = self.fresh_label("guard");
                        self.cond_br(&c, &l_ok, &l_next);
                        self.label(&l_ok);
                    }
                    let v = self.val(&arm.body);
                    self.pop_live();
                    if let Some(s) = &slot {
                        if !self.terminated {
                            self.store(&e.ty, &v, s);
                        }
                    }
                    self.br(&l_end);
                    self.label(&l_next);
                }
                self.term("unreachable");
                self.label(&l_end);
                match slot {
                    Some(s) => self.load(&e.ty, &s),
                    None => "zeroinitializer".into(),
                }
            }
            TK::Loop { body, step, id } => {
                let slot = if e.ty.is_unitlike() { None } else { Some(self.alloca(&e.ty)) };
                let head = self.fresh_label("loop");
                let cont = self.fresh_label("cont");
                let exit = self.fresh_label("exit");
                self.loops.push(LoopTarget {
                    id: *id,
                    brk: exit.clone(),
                    cont: cont.clone(),
                    slot: slot.clone(),
                    defer_depth: self.defers.len(),
                });
                self.label(&head);
                self.val(body);
                self.label(&cont);
                if let Some(s) = step {
                    self.val(s);
                }
                // In tail-call vm code (handlers, their cold blocks, vm
                // functions), loops are not vectorized, interleaved or
                // unrolled: they run a few times (zeroing a frame's
                // locals, copying arguments), and the wide versions'
                // registers can give the handler a stack frame and spills.
                let f = self.f();
                if f.vm.is_some_and(|v| self.p().vms[v as usize].dispatch == crate::tir::Dispatch::TailCall) {
                    let n = LOOP_MD + 3 + self.cg.loop_md_count;
                    self.cg.loop_md_count += 1;
                    let _ = writeln!(self.cg.loop_md, "!{n} = distinct !{{!{n}, !{}, !{}, !{}}}", LOOP_MD, LOOP_MD + 1, LOOP_MD + 2);
                    self.term(format!("br label %{}, !llvm.loop !{}", head, n));
                } else {
                    self.br(&head);
                }
                self.loops.pop();
                self.label(&exit);
                match slot {
                    Some(s) => self.load(&e.ty, &s),
                    None => self.poison(&e.ty),
                }
            }
            TK::Break(id, v) => {
                let v = v.as_ref().map(|v| (self.val(v), v.ty.clone()));
                let li = self.loops.iter().rposition(|l| l.id == *id).unwrap();
                if let (Some((v, t)), Some(s)) = (&v, self.loops[li].slot.clone()) {
                    self.store(t, v, &s);
                }
                let depth = self.loops[li].defer_depth;
                let rooted = match &v {
                    Some((v, t)) if self.has_defers(depth) => self.root_pending(v, t),
                    _ => false,
                };
                self.run_defers_to(depth);
                self.unroot_pending(rooted);
                let l = self.loops[li].brk.clone();
                self.br(&l);
                "zeroinitializer".into()
            }
            TK::Continue(id) => {
                let li = self.loops.iter().rposition(|l| l.id == *id).unwrap();
                let depth = self.loops[li].defer_depth;
                self.run_defers_to(depth);
                let l = self.loops[li].cont.clone();
                self.br(&l);
                "zeroinitializer".into()
            }
            TK::Return(v) => {
                let rv = v.as_ref().map(|v| self.val(v));
                let rooted = match (&rv, v) {
                    (Some(rv), Some(v)) if self.has_defers(0) => self.root_pending(rv, &v.ty),
                    _ => false,
                };
                self.run_defers_to(0);
                self.unroot_pending(rooted);
                let rt = self.f().ret.clone();
                match rv {
                    Some(v) if !rt.is_unitlike() => {
                        let t = self.ty(&rt);
                        self.term(format!("ret {} {}", t, v));
                    }
                    _ => self.term("ret void"),
                }
                "zeroinitializer".into()
            }
            TK::Assign(lhs, rhs) => {
                let v = self.val(rhs);
                if crate::gcgen::needs_barrier(self, lhs) {
                    crate::gcgen::store_with_barrier(self, lhs, &v);
                    return "zeroinitializer".into();
                }
                let p = self.place(lhs);
                if matches!(lhs.kind, TK::Vm(TVm::Word(_) | TVm::WordAt(..))) && !lhs.ty.is_unitlike() {
                    // A code word or inline cache field: only word-aligned.
                    let t = self.ty(&lhs.ty);
                    let a = match &lhs.kind {
                        TK::Vm(TVm::WordAt(_, _, a)) => *a,
                        _ => crate::vmgen::word_align(self),
                    };
                    self.emit(format!("store {} {}, ptr {}, align {}", t, v, p, a));
                } else {
                    self.store(&lhs.ty, &v, &p);
                    if let Some(obj) = crate::gcgen::env_barrier(self, lhs) {
                        self.emit(format!("call void @\"std.gc.barrier\"(ptr {})", obj));
                    }
                }
                "zeroinitializer".into()
            }
            TK::Coalesce(a, b) => {
                let ov = self.val(a);
                let (is_some, payload) = self.option_parts(&a.ty, &ov);
                let slot = self.alloca(&e.ty);
                let l_some = self.fresh_label("some");
                let l_none = self.fresh_label("none");
                let l_end = self.fresh_label("coal");
                self.cond_br(&is_some, &l_some, &l_none);
                self.label(&l_some);
                // `a ?? b` with `b: ?T` keeps the option.
                let keep = if e.ty == a.ty { ov.clone() } else { payload };
                self.store(&e.ty, &keep, &slot);
                self.br(&l_end);
                self.label(&l_none);
                let d = self.val(b);
                self.store(&e.ty, &d, &slot);
                self.br(&l_end);
                self.label(&l_end);
                self.load(&e.ty, &slot)
            }
            TK::Try(inner) => self.try_expr(e, inner),
            TK::Unreachable => {
                self.term("unreachable");
                self.poison(&e.ty)
            }
            TK::Vm(v) => crate::vmgen::vm_expr(self, e, v),
            TK::New { heap, init, len } => crate::gcgen::new_object(self, e, heap, init, len.as_deref()),
            TK::Trailing(obj) => crate::gcgen::trailing(self, obj),
        }
    }

    fn has_defers(&self, depth: usize) -> bool {
        self.defers[depth..].iter().any(|d| !d.is_empty())
    }

    /// Spill a traced value `v` of type `t` that must survive running
    /// defers to a slot that shadow-stack frames include. Returns whether
    /// it did (to pass to `unroot_pending`).
    fn root_pending(&mut self, v: &str, t: &Ty) -> bool {
        if !crate::gcgen::is_traced(self.p(), t) {
            return false;
        }
        let slot = self.alloca(t);
        self.store(t, v, &slot);
        self.pending.push((slot, t.clone()));
        true
    }

    fn unroot_pending(&mut self, rooted: bool) {
        if rooted {
            self.pending.pop();
        }
    }

    pub fn run_defers_to(&mut self, depth: usize) {
        let mut all: Vec<TExpr> = Vec::new();
        for scope in self.defers[depth..].iter().rev() {
            for d in scope.iter().rev() {
                all.push(d.clone());
            }
        }
        for d in all {
            self.val(&d);
        }
    }

    fn stmt(&mut self, s: &TStmt) {
        match s {
            TStmt::Let(id, init) => {
                let t = self.f().locals[*id as usize].ty.clone();
                // Only the checker's operand decodes: a `let` of a cache
                // field (`cache.n`, a word too) reads the code at run time.
                let hole = match (init, &self.vm, &self.stencil) {
                    (Some(_), Some(_), Some(_)) => self.f().locals[*id as usize].field,
                    _ => None,
                };
                let v = match (init, hole) {
                    // A stencil's operand field: a hole patched with its value.
                    (_, Some(FieldDecode { off, shift, bits, signed })) => {
                        let st = self.stencil.as_ref().unwrap();
                        let base = st.part_offs[st.part];
                        if !signed {
                            crate::jitgen::note_const_field(self, *id, off, shift, bits);
                        }
                        crate::jitgen::field_hole(self, base + off, shift, bits, signed)
                    }
                    (Some(e), None) => self.val(e),
                    (None, None) => "zeroinitializer".into(),
                };
                let slot = self.slots[*id as usize].clone();
                if !t.is_unitlike() {
                    let v = if v == "zeroinitializer" && matches!(t, Ty::Int(_) | Ty::Ptr(_) | Ty::Bool | Ty::Value(_) | Ty::F64 | Ty::F32) {
                        match t {
                            Ty::Ptr(_) => "null".into(),
                            Ty::Bool => "false".into(),
                            Ty::F64 | Ty::F32 => "0.0".into(),
                            _ => "0".into(),
                        }
                    } else {
                        v
                    };
                    self.store(&t, &v, &slot);
                }
                self.live.last_mut().unwrap().push(*id);
            }
            TStmt::LetPat(p, init, els) => {
                let sp = self.place_copy(init);
                match els {
                    None => {
                        let l = self.fresh_label("nomatch");
                        let ok = self.fresh_label("letok");
                        self.test_pat(p, &sp, &l);
                        self.br(&ok);
                        self.label(&l);
                        self.term("unreachable");
                        self.label(&ok);
                    }
                    Some(eb) => {
                        let l_else = self.fresh_label("letelse");
                        let l_ok = self.fresh_label("letok");
                        // The bindings are rooted from the match on: the
                        // else block runs with their slots unassigned (or
                        // holding a previous iteration's values), so a
                        // collection there must not scan them.
                        let n = self.live.last().unwrap().len();
                        self.test_pat(p, &sp, &l_else);
                        let bound = self.live.last_mut().unwrap().split_off(n);
                        self.br(&l_ok);
                        self.label(&l_else);
                        self.val(eb);
                        if !self.terminated {
                            self.term("unreachable");
                        }
                        self.label(&l_ok);
                        self.live.last_mut().unwrap().extend(bound);
                    }
                }
            }
            TStmt::Expr(e) => {
                self.val(e);
            }
            TStmt::Defer(e) => self.defers.last_mut().unwrap().push(e.clone()),
        }
    }

    fn place_copy(&mut self, e: &TExpr) -> String {
        let v = self.val(e);
        let t = self.alloca(&e.ty);
        self.store(&e.ty, &v, &t);
        t
    }

    /// Emit tests for a chain of conditions; fall through when all hold.
    fn conds(&mut self, conds: &[TCond], fail: &str) {
        for c in conds {
            match c {
                TCond::Expr(e) => {
                    let v = self.val(e);
                    let ok = self.fresh_label("c");
                    self.cond_br(&v, &ok, fail);
                    self.label(&ok);
                }
                TCond::Let(p, e) => {
                    let sp = if let TK::Local(id) = e.kind {
                        // Patterns don't mutate, so test the local in place.
                        self.slots[id as usize].clone()
                    } else {
                        self.place_copy(e)
                    };
                    self.test_pat(p, &sp, fail);
                }
            }
        }
    }

    // ── patterns ────────────────────────────────────────────────────

    pub fn test_pat(&mut self, pat: &TPat, ptr: &str, fail: &str) {
        let t = &pat.ty;
        match &pat.kind {
            TPK::Wild => {}
            TPK::Bind(id) => {
                let v = self.load(t, ptr);
                let slot = self.slots[*id as usize].clone();
                self.store(t, &v, &slot);
                self.live.last_mut().unwrap().push(*id);
            }
            TPK::Int(v) => {
                let x = self.load(t, ptr);
                let lt = self.ty(t);
                let c = self.inst(format!("icmp eq {} {}, {}", lt, x, v));
                self.branch_ok(&c, fail);
            }
            TPK::Bool(b) => {
                let x = self.load(t, ptr);
                let c = self.inst(format!("icmp eq i1 {}, {}", x, b));
                self.branch_ok(&c, fail);
            }
            TPK::Str(bytes) => {
                let lt = self.ty(t);
                let lp = self.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 1", lt, ptr));
                let len = self.inst(format!("load i64, ptr {}", lp));
                let c = self.inst(format!("icmp eq i64 {}, {}", len, bytes.len()));
                self.branch_ok(&c, fail);
                self.str_bytes_eq(t, ptr, bytes, fail);
            }
            TPK::Range(lo, hi) => {
                let x = self.load(t, ptr);
                let lt = self.ty(t);
                let signed = matches!(t, Ty::Int(it) if it.signed);
                let (ge, le) = if signed { ("sge", "sle") } else { ("uge", "ule") };
                let a = self.inst(format!("icmp {} {} {}, {}", ge, lt, x, lo));
                let b = self.inst(format!("icmp {} {} {}, {}", le, lt, x, hi));
                let c = self.inst(format!("and i1 {}, {}", a, b));
                self.branch_ok(&c, fail);
            }
            TPK::Tuple(ps) => {
                if crate::values::try_merged_tuple(self, ps, t, ptr, fail) {
                    return;
                }
                let lt = self.ty(t);
                for (i, sp) in ps.iter().enumerate() {
                    if matches!(sp.kind, TPK::Wild) {
                        continue;
                    }
                    let fp = self.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 {}", lt, ptr, i));
                    self.test_pat(sp, &fp, fail);
                }
            }
            TPK::Or(alts) => {
                let ok = self.fresh_label("orok");
                for a in alts {
                    let next = self.fresh_label("ornext");
                    self.test_pat(a, ptr, &next);
                    self.br(&ok);
                    self.label(&next);
                }
                self.br(fail);
                self.label(&ok);
            }
            TPK::Variant(idx, subs) => self.test_variant(t, *idx, subs, ptr, fail),
        }
    }

    /// Match `arms`, whose patterns are all string literals (or `|` of
    /// them): a `switch` on the length, then the literals of that length
    /// in arm order. Falls through when none matches.
    fn str_switch(&mut self, t: &Ty, sp: &str, arms: &[TArm], rty: &Ty, slot: Option<&str>, l_end: &str) {
        let lt = self.ty(t);
        let lp = self.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 1", lt, sp));
        let len = self.inst(format!("load i64, ptr {}", lp));
        let l_rest = self.fresh_label("strrest");
        let bodies: Vec<String> = arms.iter().map(|_| self.fresh_label("strarm")).collect();
        let mut by_len: std::collections::BTreeMap<usize, Vec<(usize, &[u8])>> = Default::default();
        for (i, a) in arms.iter().enumerate() {
            for b in str_lits(&a.pat).unwrap() {
                by_len.entry(b.len()).or_default().push((i, b));
            }
        }
        let lens: Vec<(usize, String)> = by_len.keys().map(|n| (*n, self.fresh_label("strlen"))).collect();
        let cases: String = lens.iter().map(|(n, l)| format!(" i64 {}, label %{}", n, l)).collect();
        self.term(format!("switch i64 {}, label %{} [{} ]", len, l_rest, cases));
        for (n, l) in &lens {
            self.label(l);
            for (i, b) in &by_len[n] {
                let next = self.fresh_label("strnext");
                self.str_bytes_eq(t, sp, b, &next);
                self.br(&bodies[*i]);
                self.label(&next);
            }
            self.br(&l_rest);
        }
        for (a, l) in arms.iter().zip(&bodies) {
            self.label(l);
            self.live.push(vec![]);
            let v = self.val(&a.body);
            self.pop_live();
            if let Some(s) = slot {
                if !self.terminated {
                    self.store(rty, &v, s);
                }
            }
            self.br(l_end);
        }
        self.label(&l_rest);
    }

    /// Branch to `fail` unless the bytes of the `str` at `ptr`, whose
    /// length is known to be `bytes.len()`, equal `bytes`. Compares in
    /// 8-, 4-, 2- and 1-byte loads against constants (the last load may
    /// overlap the previous one), with one branch.
    fn str_bytes_eq(&mut self, t: &Ty, ptr: &str, bytes: &[u8], fail: &str) {
        let n = bytes.len();
        if n == 0 {
            return;
        }
        let lt = self.ty(t);
        let dp = self.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 0", lt, ptr));
        let data = self.inst(format!("load ptr, ptr {}", dp));
        let mut all: Option<String> = None;
        let mut o = 0;
        while o < n {
            let mut w = 8;
            while w > 1 && w > n {
                w /= 2;
            }
            if o + w > n {
                // Shrink to what is left, or overlap the previous load.
                let rem = n - o;
                let mut w2 = w;
                while w2 > rem {
                    w2 /= 2;
                }
                if w2 == rem { w = w2 } else { o = n - w }
            }
            let mut k: u64 = 0;
            for i in (0..w).rev() {
                k = (k << 8) | bytes[o + i] as u64;
            }
            let p = self.inst(format!("getelementptr inbounds i8, ptr {}, i64 {}", data, o));
            let it = format!("i{}", w * 8);
            let v = self.inst(format!("load {}, ptr {}, align 1", it, p));
            // As a signed constant of the load's width.
            let sk = (k << (64 - w * 8)) as i64 >> (64 - w * 8);
            let c = self.inst(format!("icmp eq {} {}, {}", it, v, sk));
            all = Some(match all {
                None => c,
                Some(a) => self.inst(format!("and i1 {}, {}", a, c)),
            });
            o += w;
        }
        self.branch_ok(&all.unwrap(), fail);
    }

    pub fn branch_ok(&mut self, c: &str, fail: &str) {
        let ok = self.fresh_label("m");
        self.cond_br(c, &ok, fail);
        self.label(&ok);
    }

    fn test_variant(&mut self, t: &Ty, idx: u32, subs: &[TPat], ptr: &str, fail: &str) {
        let p = self.p();
        match t {
            Ty::Enum(eid) => {
                let e = &p.enums[*eid as usize];
                let discr = e.variants[idx as usize].discr;
                let tt = format!("i{}", e.tag.bits);
                let tag = self.inst(format!("load {}, ptr {}", tt, ptr));
                let c = self.inst(format!("icmp eq {} {}, {}", tt, tag, discr));
                self.branch_ok(&c, fail);
                if !subs.is_empty() {
                    let fields = e.variants[idx as usize].fields.clone();
                    let et = self.ty(t);
                    let pp = self.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 1", et, ptr));
                    let st = Ty::Tuple(fields);
                    let stl = self.ty(&st);
                    for (i, sp) in subs.iter().enumerate() {
                        if matches!(sp.kind, TPK::Wild) {
                            continue;
                        }
                        let fp = self.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 {}", stl, pp, i));
                        self.test_pat(sp, &fp, fail);
                    }
                }
            }
            Ty::Option(inner) => {
                if inner.is_nullable_ptr() {
                    let v = self.inst(format!("load ptr, ptr {}", ptr));
                    let op = if idx == 0 { "eq" } else { "ne" };
                    let c = self.inst(format!("icmp {} ptr {}, null", op, v));
                    self.branch_ok(&c, fail);
                    if idx == 1 {
                        self.test_pat(&subs[0], ptr, fail);
                    }
                } else {
                    let lt = self.ty(t);
                    let tp = self.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 0", lt, ptr));
                    let tag = self.inst(format!("load i1, ptr {}", tp));
                    let c = if idx == 1 { tag } else { self.inst(format!("xor i1 {}, true", tag)) };
                    self.branch_ok(&c, fail);
                    if idx == 1 {
                        let pp = self.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 1", lt, ptr));
                        self.test_pat(&subs[0], &pp, fail);
                    }
                }
            }
            Ty::Result(..) => {
                let lt = self.ty(t);
                let tp = self.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 0", lt, ptr));
                let tag = self.inst(format!("load i1, ptr {}", tp));
                let c = if idx == 1 { tag } else { self.inst(format!("xor i1 {}, true", tag)) };
                self.branch_ok(&c, fail);
                let pp = self.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 {}", lt, ptr, idx + 1));
                self.test_pat(&subs[0], &pp, fail);
            }
            Ty::Value(_) => crate::values::test_variant(self, t, idx, subs, ptr, fail),
            _ => unreachable!("variant pattern on {:?}", t),
        }
    }

    // ── helpers ─────────────────────────────────────────────────────

    /// (is_some: i1, payload) for an option value.
    pub fn option_parts(&mut self, t: &Ty, v: &str) -> (String, String) {
        let Ty::Option(inner) = t else { unreachable!() };
        if inner.is_nullable_ptr() {
            let c = self.inst(format!("icmp ne ptr {}, null", v));
            (c, v.to_string())
        } else {
            let lt = self.ty(t);
            let c = self.inst(format!("extractvalue {} {}, 0", lt, v));
            let p = if inner.is_unitlike() {
                "zeroinitializer".to_string()
            } else {
                self.inst(format!("extractvalue {} {}, 1", lt, v))
            };
            (c, p)
        }
    }

    pub fn make_option(&mut self, t: &Ty, some: Option<&str>) -> String {
        let Ty::Option(inner) = t else { unreachable!() };
        if inner.is_nullable_ptr() {
            return some.unwrap_or("null").to_string();
        }
        let lt = self.ty(t);
        let it = self.ty(inner);
        match some {
            None => format!("zeroinitializer"),
            Some(v) => {
                let a = self.inst(format!("insertvalue {} poison, i1 true, 0", lt));
                if inner.is_unitlike() {
                    return self.inst(format!("insertvalue {} {}, {{}} zeroinitializer, 1", lt, a));
                }
                self.inst(format!("insertvalue {} {}, {} {}, 1", lt, a, it, v))
            }
        }
    }

    fn variant(&mut self, t: &Ty, idx: u32, args: &[TExpr]) -> String {
        let vals: Vec<String> = args.iter().map(|a| self.val(a)).collect();
        let p = self.p();
        match t {
            Ty::Enum(eid) => {
                let e = &p.enums[*eid as usize];
                let discr = e.variants[idx as usize].discr;
                if !e.has_payload() {
                    return discr.to_string();
                }
                let tmp = self.alloca(t);
                let lt = self.ty(t);
                self.emit(format!("store {} zeroinitializer, ptr {}", lt, tmp));
                self.emit(format!("store i{} {}, ptr {}", e.tag.bits, discr, tmp));
                if !args.is_empty() {
                    let pp = self.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 1", lt, tmp));
                    let st = Ty::Tuple(args.iter().map(|a| a.ty.clone()).collect());
                    let stl = self.ty(&st);
                    for (i, (a, v)) in args.iter().zip(vals.iter()).enumerate() {
                        let fp = self.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 {}", stl, pp, i));
                        self.store(&a.ty, v, &fp);
                    }
                }
                self.load(t, &tmp)
            }
            Ty::Option(_) => {
                if idx == 0 {
                    self.make_option(t, None)
                } else {
                    self.make_option(t, Some(&vals[0]))
                }
            }
            Ty::Result(a, b) => {
                let lt = self.ty(t);
                let x = self.inst(format!("insertvalue {} zeroinitializer, i1 {}, 0", lt, idx == 1));
                let (ft, pos) = if idx == 0 { (a, 1) } else { (b, 2) };
                if ft.is_unitlike() {
                    return x;
                }
                let ftl = self.ty(ft);
                self.inst(format!("insertvalue {} {}, {} {}, {}", lt, x, ftl, vals[0], pos))
            }
            Ty::Value(_) => crate::values::make(self, t, idx, args, &vals),
            _ => unreachable!(),
        }
    }

    fn try_expr(&mut self, e: &TExpr, inner: &TExpr) -> String {
        let v = self.val(inner);
        let l_bad = self.fresh_label("tryfail");
        let l_ok = self.fresh_label("tryok");
        match &inner.ty {
            Ty::Option(_) => {
                let (is_some, payload) = self.option_parts(&inner.ty, &v);
                self.cond_br(&is_some, &l_ok, &l_bad);
                self.label(&l_bad);
                self.run_defers_to(0);
                let rt = self.f().ret.clone();
                let none = self.make_option(&rt, None);
                let rtl = self.ty(&rt);
                self.term(format!("ret {} {}", rtl, none));
                self.label(&l_ok);
                payload
            }
            Ty::Result(_, et) => {
                let lt = self.ty(&inner.ty);
                let is_err = self.inst(format!("extractvalue {} {}, 0", lt, v));
                self.cond_br(&is_err, &l_bad, &l_ok);
                self.label(&l_bad);
                let err = if et.is_unitlike() { "zeroinitializer".into() } else { self.inst(format!("extractvalue {} {}, 2", lt, v)) };
                if self.vm.is_some() {
                    crate::vmgen::raise_value(self, &err, et);
                } else {
                    let rooted = if self.has_defers(0) { self.root_pending(&err, et) } else { false };
                    self.run_defers_to(0);
                    self.unroot_pending(rooted);
                    let rt = self.f().ret.clone();
                    let rtl = self.ty(&rt);
                    let x = self.inst(format!("insertvalue {} zeroinitializer, i1 true, 0", rtl));
                    let x = if et.is_unitlike() {
                        x
                    } else {
                        let etl = self.ty(et);
                        self.inst(format!("insertvalue {} {}, {} {}, 2", rtl, x, etl, err))
                    };
                    self.term(format!("ret {} {}", rtl, x));
                }
                self.label(&l_ok);
                if e.ty.is_unitlike() {
                    return "zeroinitializer".into();
                }
                self.inst(format!("extractvalue {} {}, 1", lt, v))
            }
            _ => unreachable!(),
        }
    }

    fn slice_range(&mut self, e: &TExpr, b: &TExpr, lo: Option<&TExpr>, hi: Option<&TExpr>) -> String {
        let elem = match &e.ty {
            Ty::Slice(t) => (**t).clone(),
            _ => unreachable!(),
        };
        let el = self.ty(&elem);
        let (ptr, len) = match &b.ty {
            Ty::Array(_, n) => {
                let p = self.place(b);
                (p, n.to_string())
            }
            Ty::Slice(_) => {
                let s = self.val(b);
                let p = self.inst(format!("extractvalue {{ ptr, i64 }} {}, 0", s));
                let l = self.inst(format!("extractvalue {{ ptr, i64 }} {}, 1", s));
                (p, l)
            }
            Ty::Ptr(_) => {
                let p = self.val(b);
                let h = hi.map(|h| self.val(h)).unwrap();
                (p, h)
            }
            _ => unreachable!(),
        };
        let lo_v = match lo {
            Some(l) => self.val(l),
            None => "0".into(),
        };
        let hi_v = match (hi, &b.ty) {
            (Some(_), Ty::Ptr(_)) => len.clone(),
            (Some(h), _) => self.val(h),
            (None, _) => len.clone(),
        };
        if !matches!(b.ty, Ty::Ptr(_)) {
            let c1 = self.inst(format!("icmp ugt i64 {}, {}", hi_v, len));
            let c2 = self.inst(format!("icmp ugt i64 {}, {}", lo_v, hi_v));
            let c = self.inst(format!("or i1 {}, {}", c1, c2));
            self.check(&c, "slice range out of bounds", e.span);
        }
        let np = self.inst(format!("getelementptr inbounds {}, ptr {}, i64 {}", el, ptr, lo_v));
        let nl = self.inst(format!("sub i64 {}, {}", hi_v, lo_v));
        let a = self.inst(format!("insertvalue {{ ptr, i64 }} poison, ptr {}, 0", np));
        self.inst(format!("insertvalue {{ ptr, i64 }} {}, i64 {}, 1", a, nl))
    }

    fn binary(&mut self, e: &TExpr, op: BinOp, a: &TExpr, b: &TExpr) -> String {
        use BinOp::*;
        if matches!(op, LAnd | LOr) {
            let slot = self.alloca(&Ty::Bool);
            let x = self.val(a);
            self.emit(format!("store i1 {}, ptr {}", x, slot));
            let l_rhs = self.fresh_label("rhs");
            let l_end = self.fresh_label("sc");
            if op == LAnd {
                self.cond_br(&x, &l_rhs, &l_end);
            } else {
                self.cond_br(&x, &l_end, &l_rhs);
            }
            self.label(&l_rhs);
            let y = self.val(b);
            self.emit(format!("store i1 {}, ptr {}", y, slot));
            self.br(&l_end);
            self.label(&l_end);
            return self.inst(format!("load i1, ptr {}", slot));
        }
        let x = self.val(a);
        let y = self.val(b);
        let t = &a.ty;
        let lt = self.ty(t);
        if t.is_float() {
            let ins = match op {
                Add => "fadd",
                Sub => "fsub",
                Mul => "fmul",
                Div => "fdiv",
                Rem => "frem",
                Eq => "fcmp oeq",
                Ne => "fcmp une",
                Lt => "fcmp olt",
                Le => "fcmp ole",
                Gt => "fcmp ogt",
                Ge => "fcmp oge",
                _ => unreachable!(),
            };
            return self.inst(format!("{} {} {}, {}", ins, lt, x, y));
        }
        let signed = matches!(t, Ty::Int(it) if it.signed);
        if let Ty::Option(inner) = t {
            if !inner.is_nullable_ptr() && matches!(op, Eq | Ne) {
                let (sa, pa) = self.option_parts(t, &x);
                let (sb, pb) = self.option_parts(t, &y);
                let it = self.ty(inner);
                let peq = if inner.is_float() {
                    self.inst(format!("fcmp oeq {} {}, {}", it, pa, pb))
                } else if crate::values::is_boxed(self.p(), inner) {
                    "true".to_string()
                } else {
                    self.inst(format!("icmp eq {} {}, {}", it, pa, pb))
                };
                let same_tag = self.inst(format!("icmp eq i1 {}, {}", sa, sb));
                let pay = self.inst(format!("select i1 {}, i1 {}, i1 true", sa, peq));
                let r = self.inst(format!("and i1 {}, {}", same_tag, pay));
                return if op == Eq { r } else { self.inst(format!("xor i1 {}, true", r)) };
            }
        }
        if crate::values::is_boxed(self.p(), t) {
            let mut parts = Vec::new();
            for k in 0..2 {
                let xa = self.inst(format!("extractvalue {{ i64, i64 }} {}, {}", x, k));
                let ya = self.inst(format!("extractvalue {{ i64, i64 }} {}, {}", y, k));
                parts.push(self.inst(format!("icmp eq i64 {}, {}", xa, ya)));
            }
            let both = self.inst(format!("and i1 {}, {}", parts[0], parts[1]));
            return if op == BinOp::Eq { both } else { self.inst(format!("xor i1 {}, true", both)) };
        }
        if op.is_cmp() {
            let c = match (op, signed) {
                (Eq, _) => "eq",
                (Ne, _) => "ne",
                (Lt, true) => "slt",
                (Le, true) => "sle",
                (Gt, true) => "sgt",
                (Ge, true) => "sge",
                (Lt, false) => "ult",
                (Le, false) => "ule",
                (Gt, false) => "ugt",
                (Ge, false) => "uge",
                _ => unreachable!(),
            };
            if let Ty::Int(it) = t {
                if !matches!(it.bits, 8 | 16 | 32 | 64) {
                    // Odd widths (i48 payloads): compare in i64 with both
                    // operands shifted to the top, one `shl` each, instead
                    // of LLVM's sign-extension (`shl` + `sar`) of each.
                    let k = 64 - it.bits as u32;
                    let mut w = Vec::new();
                    for v in [&x, &y] {
                        let z = self.inst(format!("zext {} {} to i64", lt, v));
                        w.push(self.inst(format!("shl i64 {}, {}", z, k)));
                    }
                    return self.inst(format!("icmp {} i64 {}, {}", c, w[0], w[1]));
                }
            }
            return self.inst(format!("icmp {} {} {}, {}", c, lt, x, y));
        }
        if matches!(op, Add | Sub | Mul) && self.cg.opts.overflow_checks && t.is_int() {
            let name = match op {
                Add => "add",
                Sub => "sub",
                _ => "mul",
            };
            let Ty::Int(it) = t else { unreachable!() };
            let (r, ov) = self.overflow_op(*it, name, &x, &y);
            self.check(&ov, "integer overflow", e.span);
            return r;
        }
        if matches!(op, Div | Rem) && t.is_int() {
            let z = self.inst(format!("icmp eq {} {}, 0", lt, y));
            self.check(&z, "division by zero", e.span);
            if signed {
                // MIN / -1 overflows (and traps in hardware).
                let Ty::Int(it) = t else { unreachable!() };
                let m1 = self.inst(format!("icmp eq {} {}, -1", lt, y));
                let mn = self.inst(format!("icmp eq {} {}, {}", lt, x, it.min()));
                let both = self.inst(format!("and i1 {}, {}", m1, mn));
                self.check(&both, "integer overflow in division", e.span);
            }
        }
        let ins = match op {
            Add | AddWrap => "add",
            Sub | SubWrap => "sub",
            Mul | MulWrap => "mul",
            Div => if signed { "sdiv" } else { "udiv" },
            Rem => if signed { "srem" } else { "urem" },
            And => "and",
            Or => "or",
            Xor => "xor",
            Shl => "shl",
            Shr => if signed { "ashr" } else { "lshr" },
            _ => unreachable!(),
        };
        if matches!(op, Shl | Shr) {
            // The amount is taken modulo the width, as an unsigned number
            // of its own type, which may differ from the shifted value's:
            // reduce it in the wider of the two, then fit it to `lt`.
            let Ty::Int(it) = t else { unreachable!() };
            let Ty::Int(bt) = &b.ty else { unreachable!() };
            let w = it.bits.max(bt.bits);
            let wt = format!("i{}", w);
            let yw = if bt.bits < w { self.inst(format!("zext i{} {} to {}", bt.bits, y, wt)) } else { y };
            let m = if it.bits.is_power_of_two() {
                self.inst(format!("and {} {}, {}", wt, yw, it.bits - 1))
            } else {
                self.inst(format!("urem {} {}, {}", wt, yw, it.bits))
            };
            let m = if it.bits < w { self.inst(format!("trunc {} {} to {}", wt, m, lt)) } else { m };
            return self.inst(format!("{} {} {}, {}", ins, lt, x, m));
        }
        if let (Div | Rem, Ty::Int(it)) = (op, t) {
            if it.bits > 32 && it.bits < 64 && y.parse::<i64>().is_err() {
                return self.narrow_div(*it, if op == Div { "udiv" } else { "urem" }, ins, &x, &y);
            }
        }
        self.inst(format!("{} {} {}, {}", ins, lt, x, y))
    }

    /// Division of odd-width integers wider than 32 bits (i48 payloads),
    /// which LLVM does with a 64-bit divide: when both operands are
    /// non-negative and below 2^31, which interpreters' integers mostly
    /// are, use a 32-bit unsigned divide instead, two to three times
    /// faster on many x86 cores (LLVM's "bypass slow division", which it
    /// applies to i64 only on some CPU tunings).
    fn narrow_div(&mut self, it: IntTy, ins32: &str, ins: &str, x: &str, y: &str) -> String {
        let nt = format!("i{}", it.bits);
        let o = self.inst(format!("or {} {}, {}", nt, x, y));
        let hi = self.inst(format!("lshr {} {}, 31", nt, o));
        let small = self.inst(format!("icmp eq {} {}, 0", nt, hi));
        let (l_fast, l_slow, l_join) = (self.fresh_label("divfast"), self.fresh_label("divslow"), self.fresh_label("divjoin"));
        self.cond_br(&small, &l_fast, &l_slow);
        self.label(&l_fast);
        let xs = self.inst(format!("trunc {} {} to i32", nt, x));
        let ys = self.inst(format!("trunc {} {} to i32", nt, y));
        let rs = self.inst(format!("{} i32 {}, {}", ins32, xs, ys));
        let rf = self.inst(format!("zext i32 {} to {}", rs, nt));
        self.br(&l_join);
        self.label(&l_slow);
        let rw = self.inst(format!("{} {} {}, {}", ins, nt, x, y));
        self.br(&l_join);
        self.label(&l_join);
        self.inst(format!("phi {} [ {}, %{} ], [ {}, %{} ]", nt, rf, l_fast, rw, l_slow))
    }

    /// `x op y` (add, sub or mul) on integer type `it` with overflow
    /// detection: (result, overflowed). Widths other than 8/16/32/64 (such
    /// as `i48` value payloads) are computed in i64 with the operands
    /// shifted into the high bits, so the overflow flag of the 64-bit
    /// operation is the overflow of the narrow one; for `mul` only one
    /// operand is shifted. (LLVM's own legalization of odd-width overflow
    /// intrinsics sign-extends both operands and compares the result with
    /// its re-extension, which is longer.)
    pub fn overflow_op(&mut self, it: IntTy, op: &str, x: &str, y: &str) -> (String, String) {
        let pre = if it.signed { "s" } else { "u" };
        let native = matches!(it.bits, 8 | 16 | 32 | 64);
        let lt = if native { format!("i{}", it.bits) } else { "i64".to_string() };
        let name = format!("llvm.{}{}.with.overflow.{}", pre, op, lt);
        self.cg.decls.insert(format!("declare {{ {}, i1 }} @{}({}, {})", lt, name, lt, lt));
        if native {
            let r = self.inst(format!("call {{ {}, i1 }} @{}({} {}, {} {})", lt, name, lt, x, lt, y));
            let v = self.inst(format!("extractvalue {{ {}, i1 }} {}, 0", lt, r));
            let ov = self.inst(format!("extractvalue {{ {}, i1 }} {}, 1", lt, r));
            return (v, ov);
        }
        let k = 64 - it.bits as u32;
        // The extension bits of a shifted operand are shifted out: zext lets
        // LLVM fold `zext(trunc(v >> k)) << k` back into `v`.
        let nt = format!("i{}", it.bits);
        let xw = self.inst(format!("zext {} {} to i64", nt, x));
        let xs = self.inst(format!("shl i64 {}, {}", xw, k));
        let ys = if op == "mul" {
            self.inst(format!("{} {} {} to i64", if it.signed { "sext" } else { "zext" }, nt, y))
        } else {
            let yw = self.inst(format!("zext {} {} to i64", nt, y));
            self.inst(format!("shl i64 {}, {}", yw, k))
        };
        let r = self.inst(format!("call {{ i64, i1 }} @{}(i64 {}, i64 {})", name, xs, ys));
        let v = self.inst(format!("extractvalue {{ i64, i1 }} {}, 0", r));
        let ov = self.inst(format!("extractvalue {{ i64, i1 }} {}, 1", r));
        // The low k bits of the result are zero (both operands, or one factor, had them zero).
        let sh = self.inst(format!("{} exact i64 {}, {}", if it.signed { "ashr" } else { "lshr" }, v, k));
        let n = self.inst(format!("trunc i64 {} to {}", sh, nt));
        (n, ov)
    }

    /// `x as Kind` for a fieldless enum. With checks on (debug builds), a
    /// value that is not one of the discriminants panics.
    fn int_to_enum(&mut self, inner: &TExpr, e: &TExpr) -> String {
        let Ty::Int(it) = inner.ty else { unreachable!() };
        let Ty::Enum(eid) = e.ty else { unreachable!() };
        let ed = &self.p().enums[eid as usize];
        if self.cg.opts.overflow_checks {
            let v = self.val(inner);
            let w = if it.bits < 64 {
                let op = if it.signed { "sext" } else { "zext" };
                self.inst(format!("{} i{} {} to i64", op, it.bits, v))
            } else {
                v.clone()
            };
            let mut ds: Vec<i128> = ed.variants.iter().map(|v| v.discr).collect();
            ds.sort();
            ds.dedup();
            let (lo, hi) = (ds[0], *ds.last().unwrap());
            let mut bad = if hi - lo + 1 == ds.len() as i128 {
                let d = self.inst(format!("sub i64 {}, {}", w, lo as i64));
                self.inst(format!("icmp ugt i64 {}, {}", d, (hi - lo) as i64))
            } else {
                let mut ok = "false".to_string();
                for d in &ds {
                    let c = self.inst(format!("icmp eq i64 {}, {}", w, *d as i64));
                    ok = self.inst(format!("or i1 {}, {}", ok, c));
                }
                self.inst(format!("xor i1 {}, true", ok))
            };
            if !it.signed && it.bits == 64 && lo < 0 {
                // Large u64 values would look negative.
                let neg = self.inst(format!("icmp slt i64 {}, 0", w));
                bad = self.inst(format!("or i1 {}, {}", bad, neg));
            }
            self.check(&bad, "integer cast to enum is not a valid discriminant", e.span);
            let tb = ed.tag.bits as u32;
            return if tb == 64 { w } else { self.inst(format!("trunc i64 {} to i{}", w, tb)) };
        }
        self.cast(inner, &e.ty)
    }

    pub fn cast(&mut self, inner: &TExpr, to: &Ty) -> String {
        let v = self.val(inner);
        let from = &inner.ty;
        let ft = self.ty(from);
        let tt = self.ty(to);
        let bits = |t: &Ty, p: &Program| match t {
            Ty::Int(it) => it.bits as u32,
            Ty::Bool => 1,
            Ty::Enum(e) => p.enums[*e as usize].tag.bits as u32,
            Ty::Value(_) | Ty::Ptr(_) => 64,
            _ => 0,
        };
        let p = self.p();
        let fsigned = matches!(from, Ty::Int(it) if it.signed) || matches!(from, Ty::Enum(_));
        let tsigned = matches!(to, Ty::Int(it) if it.signed);
        match (from, to) {
            (Ty::Int(_) | Ty::Bool | Ty::Enum(_) | Ty::Value(_), Ty::Int(_) | Ty::Value(_) | Ty::Enum(_)) => {
                let (fb, tb) = (bits(from, p), bits(to, p));
                if fb == tb {
                    v
                } else if fb > tb {
                    self.inst(format!("trunc {} {} to {}", ft, v, tt))
                } else if fsigned && *from != Ty::Bool {
                    self.inst(format!("sext {} {} to {}", ft, v, tt))
                } else {
                    self.inst(format!("zext {} {} to {}", ft, v, tt))
                }
            }
            (Ty::Int(_), Ty::F32 | Ty::F64) => {
                let op = if fsigned { "sitofp" } else { "uitofp" };
                self.inst(format!("{} {} {} to {}", op, ft, v, tt))
            }
            (Ty::F32 | Ty::F64, Ty::Int(_)) => {
                let name = format!("llvm.fpto{}i.sat.{}.{}", if tsigned { "s" } else { "u" }, tt, if *from == Ty::F32 { "f32" } else { "f64" });
                self.cg.decls.insert(format!("declare {} @{}({})", tt, name, ft));
                self.inst(format!("call {} @{}({} {})", tt, name, ft, v))
            }
            (Ty::Closure(..), Ty::Block(..)) => v,
            (Ty::Fn(ps, r, _), Ty::Block(..) | Ty::Closure(..)) => {
                let t = self.trampoline(ps, r);
                let a = self.inst(format!("insertvalue {{ ptr, ptr }} poison, ptr {}, 0", t));
                self.inst(format!("insertvalue {{ ptr, ptr }} {}, ptr {}, 1", a, v))
            }
            (Ty::F32, Ty::F64) => self.inst(format!("fpext float {} to double", v)),
            (Ty::F64, Ty::F32) => self.inst(format!("fptrunc double {} to float", v)),
            (Ty::Ptr(_) | Ty::Fn(..) | Ty::Gc(_), Ty::Ptr(_) | Ty::Gc(_)) => v,
            (Ty::Int(_), Ty::Gc(_)) => self.inst(format!("inttoptr {} {} to ptr", ft, v)),
            (Ty::Ptr(_) | Ty::Gc(_), Ty::Int(_)) => self.inst(format!("ptrtoint ptr {} to {}", v, tt)),
            (Ty::Int(_), Ty::Ptr(_)) => self.inst(format!("inttoptr {} {} to ptr", ft, v)),
            _ => v,
        }
    }

    /// A closure value: `{ code, env }`, with the environment (pointers to
    /// the captured locals' slots) in this frame.
    fn closure(&mut self, fid: FnId, caps: &[LocalId]) -> String {
        let sym = self.p().fns[fid as usize].symbol.clone();
        let by_val = self.p().fns[fid as usize].env_val.clone();
        let env = if caps.is_empty() {
            "null".to_string()
        } else {
            let env = self.fresh();
            let _ = writeln!(self.allocas, "  {} = alloca [{} x ptr], align 8", env, caps.len());
            for (i, &l) in caps.iter().enumerate() {
                let gp = self.inst(format!("getelementptr inbounds [{} x ptr], ptr {}, i64 0, i64 {}", caps.len(), env, i));
                let slot = self.slots[l as usize].clone();
                if by_val[i] {
                    let t = self.f().locals[l as usize].ty.clone();
                    let v = self.load(&t, &slot);
                    self.store(&t, &v, &gp);
                } else {
                    self.emit(format!("store ptr {}, ptr {}", slot, gp));
                }
            }
            env
        };
        let a = self.inst(format!("insertvalue {{ ptr, ptr }} poison, ptr @\"{}\", 0", sym));
        self.inst(format!("insertvalue {{ ptr, ptr }} {}, ptr {}, 1", a, env))
    }

    /// `new(a, |..| ..)`: allocate the environment and copy the captured
    /// values into it. A `closure fn` is `{ code, env }` (env null if
    /// nothing is captured); a `Gc<closure fn>` is the object, whose first
    /// field is the code.
    fn new_closure(&mut self, ty: &Ty, fid: FnId, caps: &[LocalId], mem: &TExpr) -> String {
        let p = self.p();
        let fd = &p.fns[fid as usize];
        let sym = fd.symbol.clone();
        let env_struct = fd.env_struct;
        let m = self.val(mem);
        let gc = matches!(ty, Ty::Gc(_));
        let env = match env_struct {
            None => "null".to_string(),
            Some((sid, first)) => {
                let st = Ty::Struct(sid);
                let lt = llty(p, &st);
                let env = if gc {
                    let size = layout::size_of(p, &st);
                    let tid = p.structs[sid as usize].object.unwrap();
                    let obj = self.inst(format!("call ptr @\"std.gc.alloc\"(ptr {}, i64 {}, i32 {}, i64 0)", m, size, tid));
                    let cp = self.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 0", lt, obj));
                    self.emit(format!("store ptr @\"{}\", ptr {}", sym, cp));
                    obj
                } else {
                    m
                };
                for (i, &l) in caps.iter().enumerate() {
                    let t = self.f().locals[l as usize].ty.clone();
                    let slot = self.slots[l as usize].clone();
                    let v = self.load(&t, &slot);
                    let fp = self.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 {}", lt, env, first as usize + i));
                    self.store(&t, &v, &fp);
                }
                env
            }
        };
        if gc {
            return env;
        }
        let a = self.inst(format!("insertvalue {{ ptr, ptr }} poison, ptr @\"{}\", 0", sym));
        self.inst(format!("insertvalue {{ ptr, ptr }} {}, ptr {}, 1", a, env))
    }

    /// `R (ptr env, A..)` that calls the plain function `env` with `A..`:
    /// the code half of a plain function used as a `block fn`.
    fn trampoline(&mut self, ps: &[Ty], r: &Ty) -> String {
        let p = self.p();
        let lps: Vec<String> = ps.iter().map(|t| if matches!(t, Ty::Block(..)) { "ptr, ptr".to_string() } else { llty(p, t) }).collect();
        let rt = ret_llty(p, r);
        let key = format!("{} ({})", rt, lps.join(", "));
        if let Some(n) = self.cg.tramps.get(&key) {
            return n.clone();
        }
        let name = format!("@\"loimi.tramp.{}\"", self.cg.tramps.len());
        self.cg.tramps.insert(key, name.clone());
        let formals: Vec<String> = lps
            .iter()
            .enumerate()
            .map(|(i, t)| if t == "ptr, ptr" { format!("ptr %a{}.code, ptr %a{}.env", i, i) } else { format!("{} %a{}", t, i) })
            .collect();
        let actuals = formals.join(", ");
        let mut text = format!("define internal {} {}(ptr %env{}{}) nounwind alwaysinline {{\n", rt, name, if formals.is_empty() { "" } else { ", " }, actuals);
        if rt == "void" {
            text.push_str(&format!("  tail call void %env({})\n  ret void\n}}\n\n", actuals));
        } else {
            text.push_str(&format!("  %r = tail call {} %env({})\n  ret {} %r\n}}\n\n", rt, actuals, rt));
        }
        self.cg.fns.push_str(&text);
        name
    }

    fn call(&mut self, e: &TExpr, f: &TExpr, args: &[TExpr]) -> String {
        let p = self.p();
        let mut env = None;
        let (callee, cc, variadic, fixed) = match &f.kind {
            TK::FnRef(id) => {
                let fd = &p.fns[*id as usize];
                (format!("@\"{}\"", fd.symbol), fd.cc, fd.variadic, fd.param_tys.len())
            }
            _ if matches!(f.ty, Ty::Block(..) | Ty::Closure(..)) => {
                let v = self.val(f);
                let code = self.inst(format!("extractvalue {{ ptr, ptr }} {}, 0", v));
                env = Some(self.inst(format!("extractvalue {{ ptr, ptr }} {}, 1", v)));
                (code, CallConv::C, false, args.len())
            }
            _ if matches!(&f.ty, Ty::Gc(t) if matches!(**t, Ty::Closure(..))) => {
                // A GC closure object: the code pointer is its first field.
                let v = self.val(f);
                let code = self.inst(format!("load ptr, ptr {}", v));
                env = Some(v);
                (code, CallConv::C, false, args.len())
            }
            _ => (self.val(f), CallConv::C, false, args.len()),
        };
        let mut avs = Vec::new();
        if let Some(env) = env {
            avs.push(format!("ptr {}", env));
        }
        // Objects whose traced places are passed by address (`o.pair.set(x)`
        // auto-refs `&o.pair`): the callee may store references there, so
        // they get the write barrier after the call.
        let mut barriers = Vec::new();
        for a in args {
            if let TK::AddrOf(inner) = &a.kind {
                if crate::gcgen::needs_barrier(self, inner) {
                    let (addr, obj) = crate::gcgen::object_place(self, inner);
                    barriers.push(obj);
                    avs.push(format!("ptr {}", addr));
                    continue;
                }
            }
            let v = self.val(a);
            if let Ty::Block(..) = a.ty {
                let code = self.inst(format!("extractvalue {{ ptr, ptr }} {}, 0", v));
                let env = self.inst(format!("extractvalue {{ ptr, ptr }} {}, 1", v));
                avs.push(format!("ptr {}, ptr {}", code, env));
                continue;
            }
            let t = self.ty(&a.ty);
            avs.push(format!("{} {}", t, v));
        }
        let rt = ret_llty(p, &e.ty);
        let fnty = if variadic {
            let ps: Vec<String> = args[..fixed].iter().map(|a| llty(p, &a.ty)).collect();
            format!("{} ({}, ...)", rt, ps.join(", "))
        } else {
            rt.clone()
        };
        if let TK::FnRef(id) = &f.kind {
            let fd = &p.fns[*id as usize];
            if fd.track_caller || matches!(fd.symbol.as_str(), "loimi_caller_ptr" | "loimi_caller_len") {
                self.store_caller(e.span);
            }
        }
        let pre = crate::gcgen::push_frame(self, f);
        let call = format!("call {}{} {}({})", cc_str(cc), fnty, callee, avs.join(", "));
        let r = if rt == "void" {
            self.emit(&call);
            "zeroinitializer".to_string()
        } else {
            self.inst(&call)
        };
        crate::gcgen::pop_frame(self, pre);
        for obj in barriers {
            self.emit(format!("call void @\"std.gc.barrier\"(ptr {})", obj));
        }
        if e.ty == Ty::Never {
            self.term("unreachable");
        }
        r
    }

    fn intrinsic(&mut self, e: &TExpr, i: Intr, args: &[TExpr]) -> String {
        let vals: Vec<String> = args.iter().map(|a| self.val(a)).collect();
        let decl = |cg: &mut Codegen, s: &str| {
            cg.decls.insert(s.to_string());
        };
        match i {
            Intr::ClosureEnv => {
                // Null (None) unless `new(a, ..)` allocated the environment:
                // a converted plain function's code is the trampoline of
                // its signature (effects do not change it), and a closure
                // without captures has a null environment.
                let Ty::Closure(ps, r, _) = &args[0].ty else { unreachable!() };
                let t = self.trampoline(ps, r);
                let code = self.inst(format!("extractvalue {{ ptr, ptr }} {}, 0", vals[0]));
                let env = self.inst(format!("extractvalue {{ ptr, ptr }} {}, 1", vals[0]));
                let conv = self.inst(format!("icmp eq ptr {}, {}", code, t));
                self.inst(format!("select i1 {}, ptr null, ptr {}", conv, env))
            }
            Intr::Panic => {
                let TK::Str(m) = &args[0].kind else { unreachable!() };
                let m = String::from_utf8_lossy(m).into_owned();
                self.panic(&m, e.span);
                self.poison(&e.ty)
            }
            Intr::Likely | Intr::Unlikely => {
                decl(self.cg, "declare i1 @llvm.expect.i1(i1, i1)");
                self.inst(format!("call i1 @llvm.expect.i1(i1 {}, i1 {})", vals[0], i == Intr::Likely))
            }
            Intr::Assume => {
                decl(self.cg, "declare void @llvm.assume(i1)");
                self.emit(format!("call void @llvm.assume(i1 {})", vals[0]));
                "zeroinitializer".into()
            }
            // In a library, `trap()` is a panic, so that `#[catch_panic]`
            // and `rt.catch_panic` can catch it.
            Intr::Trap if self.cg.opts.library => {
                self.panic("trap", e.span);
                "zeroinitializer".into()
            }
            Intr::Trap => {
                decl(self.cg, "declare void @llvm.trap() noreturn");
                self.emit("call void @llvm.trap()");
                self.term("unreachable");
                "zeroinitializer".into()
            }
            Intr::AddChecked | Intr::SubChecked | Intr::MulChecked => {
                let Ty::Int(it) = args[0].ty else { unreachable!() };
                let lt = format!("i{}", it.bits);
                let op = match i {
                    Intr::AddChecked => "add",
                    Intr::SubChecked => "sub",
                    _ => "mul",
                };
                let (v, ov) = self.overflow_op(it, op, &vals[0], &vals[1]);
                let ok = self.inst(format!("xor i1 {}, true", ov));
                let ot = self.ty(&e.ty);
                let a = self.inst(format!("insertvalue {} poison, i1 {}, 0", ot, ok));
                self.inst(format!("insertvalue {} {}, {} {}, 1", ot, a, lt, v))
            }
            Intr::IsSome | Intr::IsNone => {
                let (s, _) = self.option_parts(&args[0].ty, &vals[0]);
                if i == Intr::IsSome { s } else { self.inst(format!("xor i1 {}, true", s)) }
            }
            Intr::Unwrap => {
                let (s, payload) = self.option_parts(&args[0].ty, &vals[0]);
                let n = self.inst(format!("xor i1 {}, true", s));
                self.check(&n, "unwrap of None", e.span);
                payload
            }
            Intr::UnwrapOr => {
                let (s, payload) = self.option_parts(&args[0].ty, &vals[0]);
                let t = self.ty(&e.ty);
                self.inst(format!("select i1 {}, {} {}, {} {}", s, t, payload, t, vals[1]))
            }
            Intr::PtrOffset => {
                let Ty::Ptr(inner) = &args[0].ty else { unreachable!() };
                let et = self.ty(inner);
                self.inst(format!("getelementptr inbounds {}, ptr {}, i64 {}", et, vals[0], vals[1]))
            }
            Intr::SliceFromRaw => {
                let a = self.inst(format!("insertvalue {{ ptr, i64 }} poison, ptr {}, 0", vals[0]));
                self.inst(format!("insertvalue {{ ptr, i64 }} {}, i64 {}, 1", a, vals[1]))
            }
            Intr::Clz | Intr::Ctz | Intr::Popcount => {
                let lt = self.ty(&e.ty);
                let (n, extra) = match i {
                    Intr::Clz => ("ctlz", ", i1 false"),
                    Intr::Ctz => ("cttz", ", i1 false"),
                    _ => ("ctpop", ""),
                };
                let extra_decl = if extra.is_empty() { "" } else { ", i1" };
                decl(self.cg, &format!("declare {} @llvm.{}.{}({}{})", lt, n, lt, lt, extra_decl));
                self.inst(format!("call {} @llvm.{}.{}({} {}{})", lt, n, lt, lt, vals[0], extra))
            }
            Intr::FloatBits if args[0].ty == Ty::F32 => self.inst(format!("bitcast float {} to i32", vals[0])),
            Intr::FloatBits => self.inst(format!("bitcast double {} to i64", vals[0])),
            Intr::BitsToFloat if e.ty == Ty::F32 => self.inst(format!("bitcast i32 {} to float", vals[0])),
            Intr::BitsToFloat => self.inst(format!("bitcast i64 {} to double", vals[0])),
            Intr::LoadUnaligned => {
                let lt = self.ty(&e.ty);
                self.inst(format!("load {}, ptr {}, align 1", lt, vals[0]))
            }
            Intr::StoreUnaligned => {
                let lt = self.ty(&args[1].ty);
                self.emit(format!("store {} {}, ptr {}, align 1", lt, vals[1], vals[0]));
                "zeroinitializer".into()
            }
            Intr::AtomicLoad | Intr::AtomicLoadRelaxed => {
                let lt = self.ty(&e.ty);
                let al = layout::size_of(self.p(), &e.ty);
                let ord = if i == Intr::AtomicLoad { "acquire" } else { "monotonic" };
                self.inst(format!("load atomic {}, ptr {} {}, align {}", lt, vals[0], ord, al))
            }
            Intr::AtomicStore => {
                let lt = self.ty(&args[1].ty);
                let al = layout::size_of(self.p(), &args[1].ty);
                self.emit(format!("store atomic {} {}, ptr {} release, align {}", lt, vals[1], vals[0], al));
                "zeroinitializer".into()
            }
            Intr::AtomicSwap | Intr::AtomicAdd => {
                let lt = self.ty(&e.ty);
                let op = if i == Intr::AtomicSwap { "xchg" } else { "add" };
                self.inst(format!("atomicrmw {} ptr {}, {} {} seq_cst", op, vals[0], lt, vals[1]))
            }
            Intr::AtomicCas => {
                let lt = self.ty(&args[1].ty);
                let r = self.inst(format!("cmpxchg ptr {}, {} {}, {} {} seq_cst seq_cst", vals[0], lt, vals[1], lt, vals[2]));
                self.inst(format!("extractvalue {{ {}, i1 }} {}, 1", lt, r))
            }
            Intr::IsNan => {
                let ft = self.ty(&args[0].ty);
                self.inst(format!("fcmp uno {} {}, {}", ft, vals[0], vals[0]))
            }
            Intr::Sqrt | Intr::Floor | Intr::Ceil | Intr::Trunc | Intr::Nearest | Intr::Fabs => {
                let n = match i {
                    Intr::Sqrt => "sqrt",
                    Intr::Floor => "floor",
                    Intr::Ceil => "ceil",
                    Intr::Trunc => "trunc",
                    Intr::Nearest => "roundeven",
                    _ => "fabs",
                };
                let (ft, sfx) = if e.ty == Ty::F32 { ("float", "f32") } else { ("double", "f64") };
                decl(self.cg, &format!("declare {ft} @llvm.{n}.{sfx}({ft})"));
                self.inst(format!("call {ft} @llvm.{n}.{sfx}({ft} {})", vals[0]))
            }
            Intr::Copysign | Intr::MinNum | Intr::MaxNum | Intr::Minimum | Intr::Maximum => {
                let n = match i {
                    Intr::Copysign => "copysign",
                    Intr::MinNum => "minnum",
                    Intr::MaxNum => "maxnum",
                    Intr::Minimum => "minimum",
                    _ => "maximum",
                };
                let (ft, sfx) = if e.ty == Ty::F32 { ("float", "f32") } else { ("double", "f64") };
                decl(self.cg, &format!("declare {ft} @llvm.{n}.{sfx}({ft}, {ft})"));
                let r = self.inst(format!("call {ft} @llvm.{n}.{sfx}({ft} {}, {ft} {})", vals[0], vals[1]));
                if !matches!(i, Intr::Minimum | Intr::Maximum) {
                    return r;
                }
                // IEEE 754-2019 (and wasm) want a quiet NaN; LLVM may pass
                // a signaling NaN operand through as is. `a + b` quiets it.
                let uno = self.inst(format!("fcmp uno {ft} {}, {}", vals[0], vals[1]));
                let q = self.inst(format!("fadd {ft} {}, {}", vals[0], vals[1]));
                self.inst(format!("select i1 {uno}, {ft} {q}, {ft} {r}"))
            }
            Intr::GcMark => {
                if crate::gcgen::check_roots(self) {
                    let text = format!("gc_mark in {} at {}", self.f().symbol, self.cg.location(e.span));
                    let info = crate::gcgen::rootck_info(self.cg, crate::gcgen::RC_GC_MARK, &text);
                    crate::gcgen::rootck_set(self, &info, "-1");
                }
                crate::gcgen::mark_expr(self, &args[1].ty, &vals[1], &vals[0]);
                "zeroinitializer".into()
            }
        }
    }
}

/// Which lazy functions (`FnDef::lazy`) some emitted function refers to.
/// Emitted only where used: the functions marked `LAZY_ATTR`, the
/// checks of `--gc-check-roots` in std.gc (`rootck_*`), which only
/// generated code calls, and std.gc's free hooks and dirty pinning,
/// so that the IR of programs that do not use them stays the same.
fn is_lazy(f: &FnDef) -> bool {
    f.lazy || f.symbol.starts_with("std.gc.rootck_") || f.symbol.starts_with("loimi_rootck_") || LAZY_STD.contains(&f.symbol.as_str())
}

/// std struct types whose LLVM definition is emitted only where used, so
/// that the IR of programs that do not use them stays the same.
const LAZY_STD_TYPES: &[&str] = &["std.mem.Malloc"];

const LAZY_STD: &[&str] = &[
    "std.gc.Heap.on_free",
    "std.gc.free_hooks",
    "std.gc.run_free_hooks",
    "std.gc.Heap.pin_dirty",
    "std.gc.Heap.unpin_dirty",
    "std.gc.Heap.is_pinned_dirty",
    "std.gc.pinned",
    "std.gc.scan_pinned",
    "std.gc.release_hooks",
    "std.gc.mark_shadow_stack",
    "std.gc.shadow_top",
    "std.mem.Arena.free_bytes",
    "std.mem.Malloc.new",
    "std.mem.Malloc.alloc_bytes",
    "std.mem.Malloc.free_bytes",
];

fn lazy_used(p: &Program, check_roots: bool) -> Vec<bool> {
    let mut used = vec![false; p.fns.len()];
    if !p.fns.iter().any(is_lazy) {
        return used;
    }
    let mut work: Vec<usize> = (0..p.fns.len()).filter(|&i| !is_lazy(&p.fns[i])).collect();
    if check_roots {
        for (i, f) in p.fns.iter().enumerate() {
            if f.symbol == "std.gc.rootck_mark" || f.symbol == "std.gc.rootck_alloc" {
                used[i] = true;
                work.push(i);
            }
        }
    }
    while let Some(i) = work.pop() {
        let Some(body) = &p.fns[i].body else { continue };
        crate::tirwalk::for_each_expr(body, &mut |e| {
            if let TK::FnRef(id) | TK::Closure(id, _) | TK::NewClosure(id, ..) = &e.kind {
                let id = *id as usize;
                if is_lazy(&p.fns[id]) && !used[id] {
                    used[id] = true;
                    work.push(id);
                }
            }
        });
    }
    used
}

/// The literals of a string pattern (`"a"` or `"a" | "b"`), if it is one.
fn str_lits(p: &TPat) -> Option<Vec<&[u8]>> {
    match &p.kind {
        TPK::Str(b) => Some(vec![b.as_slice()]),
        TPK::Or(alts) => {
            let mut out = vec![];
            for a in alts {
                out.extend(str_lits(a)?);
            }
            Some(out)
        }
        _ => None,
    }
}
