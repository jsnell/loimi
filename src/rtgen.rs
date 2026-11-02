//! Runtime support for `std.rt` and for panics in libraries: a chain of
//! setjmp/longjmp handlers on each thread.
//!
//! - `loimi_protect(f, data)` runs `f(data)` and returns 1, or 0 if `f`
//!   called `loimi_throw` (`rt.protect` / `rt.throw`).
//! - `loimi_catch_panic(f, data)` is the same for panics
//!   (`rt.catch_panic`): bounds checks, `unwrap` of None, overflow checks,
//!   `trap()` in libraries, `rt.panic`, out of memory in `mem`.
//! - `#[catch_panic(v)]` on an `export "C"` function wraps it in the same
//!   handler: a panic inside returns `v` to the C caller
//!   (`emit_catch_wrappers`).
//!
//! A handler is `{ previous, saved shadow-stack top, kind, jmp_buf }`,
//! linked from the thread-local `@loimi.handler_top`. `throw` jumps to the
//! innermost kind-0 handler, a panic to the innermost kind-1 handler;
//! either one pops every handler above its target. Without a target a
//! panic prints its message and aborts, as before.
//!
//! None of this is on a hot path: the handlers cost only at the calls
//! that install them, and `@loimi.panic` is a cold, out-of-line call
//! whose callers are unchanged.

use std::fmt::Write;

use crate::codegen::{c_abi_attr, llty, ret_llty, Codegen};
use crate::ast::Linkage;

/// glibc's `jmp_buf` is 200 bytes on x86-64 and 312 on AArch64.
const JMP_BUF_BYTES: u32 = 512;

/// On x86-64 the handlers use LLVM's builtin setjmp (`__builtin_setjmp`):
/// it saves the frame pointer, stack pointer and resume address in five
/// words, and the function that calls it saves the callee-saved
/// registers itself. Elsewhere, glibc's `_setjmp`/`longjmp`, which also
/// save the signal-free context and mangle pointers.
const BUILTIN_SJLJ: bool = cfg!(target_arch = "x86_64");

/// The jump buffer's type in `%loimi.handler`.
fn buf_ty() -> String {
    if BUILTIN_SJLJ { "[5 x ptr]".into() } else { format!("[{} x i8]", JMP_BUF_BYTES) }
}

/// IR lines: `%r` = 0 after saving the context in `%buf`, nonzero when
/// a jump comes back.
fn setjmp_ir() -> &'static str {
    if BUILTIN_SJLJ {
        "  %fa = call ptr @llvm.frameaddress.p0(i32 0)\n  \
           store ptr %fa, ptr %buf\n  \
           %sa = call ptr @llvm.stacksave.p0()\n  \
           %sslot = getelementptr inbounds [5 x ptr], ptr %buf, i64 0, i64 2\n  \
           store ptr %sa, ptr %sslot\n  \
           %r = call i32 @llvm.eh.sjlj.setjmp(ptr %buf)\n"
    } else {
        "  %r = call i32 @_setjmp(ptr %buf) returns_twice\n"
    }
}

/// IR line: jump to the handler whose buffer is `%buf`.
fn longjmp_ir() -> &'static str {
    if BUILTIN_SJLJ {
        "  call void @llvm.eh.sjlj.longjmp(ptr %buf) noreturn\n"
    } else {
        "  call void @longjmp(ptr %buf, i32 1) noreturn\n"
    }
}

fn declare_sjlj(cg: &mut Codegen) {
    if BUILTIN_SJLJ {
        cg.decls.insert("declare ptr @llvm.frameaddress.p0(i32 immarg)");
        cg.decls.insert("declare ptr @llvm.stacksave.p0()");
        cg.decls.insert("declare i32 @llvm.eh.sjlj.setjmp(ptr)");
        cg.decls.insert("declare void @llvm.eh.sjlj.longjmp(ptr) noreturn");
    } else {
        cg.decls.insert("declare i32 @_setjmp(ptr) returns_twice");
        cg.decls.insert("declare void @longjmp(ptr, i32) noreturn");
    }
}

/// The most bytes of an `rt.panic` message a catch keeps.
const PANIC_BUF: u32 = 512;

const KIND_THROW: u32 = 0;
const KIND_PANIC: u32 = 1;

/// Whether the program installs panic or throw handlers: uses `std.rt`,
/// or has an export with `#[catch_panic]`. Without, `@loimi.panic` just
/// prints and aborts.
pub fn has_handlers(cg: &Codegen) -> bool {
    ["loimi_protect", "loimi_throw", "loimi_catch_panic", "loimi_catch_panic_reenter"].iter().any(|n| cg.decls.0.contains_key(*n))
        || cg.p.fns.iter().any(|f| f.catch_panic.is_some() && f.body.is_some())
}

/// The `Vm.depth` counters of nested entries: a panic that unwinds
/// through a nested run would leave them counted up, so a catching export
/// saves and restores them.
fn depth_globals(cg: &Codegen) -> Vec<String> {
    let mut v: Vec<String> = cg.p.fns.iter().filter_map(|f| crate::vmgen::depth_global(cg.p, f)).collect();
    v.sort();
    v.dedup();
    v
}

/// The body of a function that installs a handler of `kind`, runs `call`,
/// and on a longjmp back runs `caught`. `call` and `caught` are IR lines
/// (`caught` ends in a `ret`); the handler is `%h`.
fn handler_body(kind: u32, depths: &[String], call: &str, caught: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "entry:\n  %h = alloca %loimi.handler, align 16");
    for (i, _) in depths.iter().enumerate() {
        let _ = writeln!(s, "  %depth{} = alloca i32", i);
    }
    let _ = writeln!(
        s,
        "  %prev = load ptr, ptr @loimi.handler_top\n  \
           %shadow = load ptr, ptr @loimi.shadow_top\n  \
           store ptr %prev, ptr %h\n  \
           %hs = getelementptr inbounds %loimi.handler, ptr %h, i32 0, i32 1\n  \
           store ptr %shadow, ptr %hs\n  \
           %hk = getelementptr inbounds %loimi.handler, ptr %h, i32 0, i32 2\n  \
           store i64 {}, ptr %hk",
        kind
    );
    for (i, d) in depths.iter().enumerate() {
        let _ = writeln!(s, "  %dv{0} = load i32, ptr {1}\n  store i32 %dv{0}, ptr %depth{0}", i, d);
    }
    let _ = writeln!(
        s,
        "  %buf = getelementptr inbounds %loimi.handler, ptr %h, i32 0, i32 3\n\
         {}  \
           %first = icmp eq i32 %r, 0\n  \
           br i1 %first, label %run, label %caught\n\
         run:\n  \
           store ptr %h, ptr @loimi.handler_top\n\
         {}  \
           store ptr %prev, ptr @loimi.handler_top\n\
         {}\
         caught:\n  \
           %prev2 = load volatile ptr, ptr %h\n  \
           %shadow2 = load volatile ptr, ptr %hs\n  \
           store ptr %prev2, ptr @loimi.handler_top\n  \
           store ptr %shadow2, ptr @loimi.shadow_top",
        setjmp_ir(),
        call.lines().map(|l| format!("  {}\n", l)).collect::<String>(),
        "  ret RESULT\n"
    );
    for (i, d) in depths.iter().enumerate() {
        let _ = writeln!(s, "  %dw{0} = load volatile i32, ptr %depth{0}\n  store i32 %dw{0}, ptr {1}", i, d);
    }
    s.push_str(caught);
    s
}

/// The handler that `@loimi.panic` or `loimi_throw` jumps to: the
/// innermost one of `kind`, or null. A function `@loimi.find_handler`.
fn emit_find(cg: &mut Codegen) {
    let _ = writeln!(
        cg.fns,
        "define internal ptr @loimi.find_handler(i64 %kind) nounwind noinline cold {{\n\
         entry:\n  \
           %h0 = load ptr, ptr @loimi.handler_top\n  \
           br label %loop\n\
         loop:\n  \
           %h = phi ptr [ %h0, %entry ], [ %next, %skip ]\n  \
           %none = icmp eq ptr %h, null\n  \
           br i1 %none, label %done, label %test\n\
         test:\n  \
           %kp = getelementptr inbounds %loimi.handler, ptr %h, i32 0, i32 2\n  \
           %k = load i64, ptr %kp\n  \
           %hit = icmp eq i64 %k, %kind\n  \
           br i1 %hit, label %done, label %skip\n\
         skip:\n  \
           %next = load ptr, ptr %h\n  \
           br label %loop\n\
         done:\n  \
           ret ptr %h\n\
         }}\n"
    );
}

/// The runtime, emitted before `@loimi.panic` (`panic_body` is its body).
pub fn emit_runtime(cg: &mut Codegen) {
    cg.panic_handlers = has_handlers(cg);
    // `loimi_panic(msg, len)`: `rt.panic` and `mem`'s out-of-memory path.
    // Its message may be built at run time, in memory the unwinding frees
    // or overwrites (a stack buffer, a `StrBuf` freed by a `defer` that
    // never runs): when a handler will catch it, it is copied into a
    // thread-local buffer first, truncated to `PANIC_BUF` bytes. Panics
    // the compiler generates have static messages and skip this.
    if cg.decls.0.remove("loimi_panic").is_some() {
        if cg.panic_handlers {
            let _ = writeln!(cg.globals, "@loimi.panic_buf = internal thread_local(initialexec) global [{} x i8] zeroinitializer", PANIC_BUF);
            cg.decls.insert("declare void @llvm.memmove.p0.p0.i64(ptr, ptr, i64, i1 immarg)");
            let _ = writeln!(
                cg.fns,
                "define internal void @loimi_panic(ptr %m, i64 %n) cold noreturn nounwind noinline {{\n\
                 entry:\n  \
                   %h = call ptr @loimi.find_handler(i64 {kind})\n  \
                   %none = icmp eq ptr %h, null\n  \
                   br i1 %none, label %die, label %copy\n\
                 copy:\n  \
                   %big = icmp ugt i64 %n, {cap}\n  \
                   %k = select i1 %big, i64 {cap}, i64 %n\n  \
                   call void @llvm.memmove.p0.p0.i64(ptr @loimi.panic_buf, ptr %m, i64 %k, i1 false)\n  \
                   call void @loimi.panic(ptr @loimi.panic_buf, i64 %k)\n  \
                   unreachable\n\
                 die:\n  \
                   call void @loimi.panic(ptr %m, i64 %n)\n  \
                   unreachable\n\
                 }}\n",
                kind = KIND_PANIC,
                cap = PANIC_BUF
            );
        } else {
            let _ = writeln!(
                cg.fns,
                "define internal void @loimi_panic(ptr %m, i64 %n) cold noreturn nounwind noinline {{\n  \
                   call void @loimi.panic(ptr %m, i64 %n)\n  unreachable\n}}\n"
            );
        }
    }
    // `test.caller()`: the location the last `#[track_caller]` call stored.
    if cg.decls.0.remove("loimi_caller_ptr").is_some() | cg.decls.0.remove("loimi_caller_len").is_some() {
        cg.caller_globals();
        let _ = writeln!(cg.fns, "define internal ptr @loimi_caller_ptr() {{\n  %v = load ptr, ptr @loimi.caller_ptr\n  ret ptr %v\n}}");
        let _ = writeln!(cg.fns, "define internal i64 @loimi_caller_len() {{\n  %v = load i64, ptr @loimi.caller_len\n  ret i64 %v\n}}");
    }
    // The last caught panic's message (`rt.panic_message`).
    let want_msg = cg.decls.0.remove("loimi_panic_msg_ptr").is_some() | cg.decls.0.remove("loimi_panic_msg_len").is_some();
    if !cg.panic_handlers {
        if want_msg {
            let _ = writeln!(cg.fns, "define internal ptr @loimi_panic_msg_ptr() {{\n  ret ptr null\n}}");
            let _ = writeln!(cg.fns, "define internal i64 @loimi_panic_msg_len() {{\n  ret i64 0\n}}");
        }
        return;
    }
    let _ = writeln!(cg.globals, "%loimi.handler = type {{ ptr, ptr, i64, {} }}", buf_ty());
    let _ = writeln!(cg.globals, "@loimi.handler_top = internal thread_local(initialexec) global ptr null");
    let _ = writeln!(cg.globals, "@loimi.panic_msg = internal thread_local(initialexec) global ptr null");
    let _ = writeln!(cg.globals, "@loimi.panic_len = internal thread_local(initialexec) global i64 0");
    let _ = writeln!(cg.fns, "define internal ptr @loimi_panic_msg_ptr() {{\n  %v = load ptr, ptr @loimi.panic_msg\n  ret ptr %v\n}}");
    let _ = writeln!(cg.fns, "define internal i64 @loimi_panic_msg_len() {{\n  %v = load i64, ptr @loimi.panic_len\n  ret i64 %v\n}}");
    emit_find(cg);
    for (name, kind) in [("loimi_protect", KIND_THROW), ("loimi_catch_panic", KIND_PANIC)] {
        cg.decls.0.remove(name);
        // `f` has no effects, so it cannot enter a vm: no depths to save.
        let body = handler_body(kind, &[], "call void %f(ptr %data)", "  ret i32 0\n").replace("RESULT", "i32 1");
        let _ = writeln!(cg.fns, "define internal i32 @{}(ptr %f, ptr %data) nounwind noinline {{\n{}}}\n", name, body);
    }
    // The same for a function that may collect or run a vm (`std.test`
    // runs tests through it): a panic in a nested run leaves its `Vm.depth`
    // counted up, so restore them.
    if cg.decls.0.remove("loimi_catch_panic_reenter").is_some() {
        let depths = depth_globals(cg);
        let body = handler_body(KIND_PANIC, &depths, "call void %f(ptr %data)", "  ret i32 0\n").replace("RESULT", "i32 1");
        let _ = writeln!(cg.fns, "define internal i32 @loimi_catch_panic_reenter(ptr %f, ptr %data) nounwind noinline {{\n{}}}\n", body);
    }
    if cg.decls.0.remove("loimi_throw").is_some() {
        let msg = "rt.throw outside rt.protect\n";
        let _ = writeln!(
            cg.globals,
            "@loimi.throw_msg = private unnamed_addr constant [{} x i8] c\"{}\\0A\"",
            msg.len(),
            &msg[..msg.len() - 1]
        );
        let _ = writeln!(
            cg.fns,
            "define internal void @loimi_throw() noreturn nounwind noinline {{\n\
             entry:\n  \
               %h = call ptr @loimi.find_handler(i64 {})\n  \
               %none = icmp eq ptr %h, null\n  \
               br i1 %none, label %orphan, label %jump\n\
             orphan:\n  \
               call void @loimi.panic(ptr @loimi.throw_msg, i64 {})\n  \
               unreachable\n\
             jump:\n  \
               %buf = getelementptr inbounds %loimi.handler, ptr %h, i32 0, i32 3\n\
             {}  \
               unreachable\n\
             }}\n",
            KIND_THROW,
            msg.len(),
            longjmp_ir()
        );
    }
    emit_catch_wrappers(cg);
    declare_sjlj(cg);
}

/// `#[catch_panic(v)]` exports: the code generator emitted the body as
/// the internal `@"sym.body"`; `@sym` installs a panic handler, calls it,
/// and returns `v` if a panic unwound to the handler.
fn emit_catch_wrappers(cg: &mut Codegen) {
    let p = cg.p;
    let depths = depth_globals(cg);
    for f in p.fns.iter().filter(|f| f.body.is_some() && f.linkage == Linkage::Export) {
        let Some(v) = &f.catch_panic else { continue };
        let params: Vec<String> =
            f.param_tys.iter().enumerate().map(|(i, t)| format!("{} {}%a{}", llty(p, t), c_abi_attr(t), i)).collect();
        let args: Vec<String> = f.param_tys.iter().enumerate().map(|(i, t)| format!("{} %a{}", llty(p, t), i)).collect();
        let rt = ret_llty(p, &f.ret);
        let (call, ok, caught) = if rt == "void" {
            (format!("call void @\"{}.body\"({})", f.symbol, args.join(", ")), "void".to_string(), "  ret void\n".to_string())
        } else {
            (
                format!("%v = call {} @\"{}.body\"({})", rt, f.symbol, args.join(", ")),
                format!("{} %v", rt),
                format!("  ret {} {}\n", rt, v),
            )
        };
        let body = handler_body(KIND_PANIC, &depths, &call, &caught).replace("RESULT", &ok);
        let _ = writeln!(
            cg.fns,
            "define {}{} @\"{}\"({}) nounwind {{\n{}}}\n",
            c_abi_attr(&f.ret),
            rt,
            f.symbol,
            params.join(", "),
            body
        );
    }
}

/// The body of `@loimi.panic(ptr %msg, i64 %len)`: with handlers, jump
/// to the innermost panic handler if there is one, after recording the
/// message; otherwise print it and abort.
pub fn panic_body(cg: &Codegen) -> String {
    let die = "  call i64 @write(i32 2, ptr %msg, i64 %len)\n  call void @abort()\n  unreachable\n";
    if !cg.panic_handlers {
        return die.to_string();
    }
    format!(
        "entry:\n  \
           %h = call ptr @loimi.find_handler(i64 {})\n  \
           %none = icmp eq ptr %h, null\n  \
           br i1 %none, label %die, label %jump\n\
         jump:\n  \
           store ptr %msg, ptr @loimi.panic_msg\n  \
           store i64 %len, ptr @loimi.panic_len\n  \
           %buf = getelementptr inbounds %loimi.handler, ptr %h, i32 0, i32 3\n\
         {}  \
           unreachable\n\
         die:\n{}",
        KIND_PANIC, longjmp_ir(), die
    )
}
