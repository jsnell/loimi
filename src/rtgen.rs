//! Runtime support for `std.rt`: `loimi_protect` and `loimi_throw`, a
//! setjmp/longjmp pair that lets a library recover from an error deep in
//! code that cannot return one (a recursive-descent parser) instead of
//! exiting the host process.
//!
//! `loimi_protect(f, data)` runs `f(data)` and returns 1, or 0 if `f`
//! called `loimi_throw`. Handlers nest through a thread-local chain. `f`
//! has type `fn(*u8)` without effects, so it cannot collect, and no
//! shadow-stack frame is live inside it; the shadow-stack top is restored
//! anyway, so a throw never leaves a dangling root frame.

use std::fmt::Write;

use crate::codegen::Codegen;

/// glibc's `jmp_buf` is 200 bytes on x86-64 and 312 on AArch64.
const JMP_BUF_BYTES: u32 = 512;

pub fn emit_runtime(cg: &mut Codegen) {
    if !cg.decls.0.contains_key("loimi_protect") && !cg.decls.0.contains_key("loimi_throw") {
        return;
    }
    cg.decls.0.remove("loimi_protect");
    cg.decls.0.remove("loimi_throw");
    // A handler: { previous handler, saved shadow-stack top, jmp_buf }.
    let _ = writeln!(cg.globals, "%loimi.handler = type {{ ptr, ptr, [{} x i8] }}", JMP_BUF_BYTES);
    let _ = writeln!(cg.globals, "@loimi.handler_top = internal thread_local(initialexec) global ptr null");
    let _ = writeln!(
        cg.fns,
        "define internal i32 @loimi_protect(ptr %f, ptr %data) nounwind noinline {{\n\
         entry:\n  \
           %h = alloca %loimi.handler, align 16\n  \
           %prev = load ptr, ptr @loimi.handler_top\n  \
           %shadow = load ptr, ptr @loimi.shadow_top\n  \
           store ptr %prev, ptr %h\n  \
           %hs = getelementptr inbounds %loimi.handler, ptr %h, i32 0, i32 1\n  \
           store ptr %shadow, ptr %hs\n  \
           %buf = getelementptr inbounds %loimi.handler, ptr %h, i32 0, i32 2\n  \
           %r = call i32 @_setjmp(ptr %buf) returns_twice\n  \
           %first = icmp eq i32 %r, 0\n  \
           br i1 %first, label %run, label %caught\n\
         run:\n  \
           store ptr %h, ptr @loimi.handler_top\n  \
           call void %f(ptr %data)\n  \
           store ptr %prev, ptr @loimi.handler_top\n  \
           ret i32 1\n\
         caught:\n  \
           %prev2 = load volatile ptr, ptr %h\n  \
           %shadow2 = load volatile ptr, ptr %hs\n  \
           store ptr %prev2, ptr @loimi.handler_top\n  \
           store ptr %shadow2, ptr @loimi.shadow_top\n  \
           ret i32 0\n\
         }}\n"
    );
    let msg = "rt.throw outside rt.protect\n";
    let _ = writeln!(cg.globals, "@loimi.throw_msg = private unnamed_addr constant [{} x i8] c\"{}\\0A\"", msg.len(), &msg[..msg.len() - 1]);
    let _ = writeln!(
        cg.fns,
        "define internal void @loimi_throw() noreturn nounwind noinline {{\n\
         entry:\n  \
           %h = load ptr, ptr @loimi.handler_top\n  \
           %none = icmp eq ptr %h, null\n  \
           br i1 %none, label %orphan, label %jump\n\
         orphan:\n  \
           call void @loimi.panic(ptr @loimi.throw_msg, i64 {})\n  \
           unreachable\n\
         jump:\n  \
           %buf = getelementptr inbounds %loimi.handler, ptr %h, i32 0, i32 2\n  \
           call void @longjmp(ptr %buf, i32 1) noreturn\n  \
           unreachable\n\
         }}\n",
        msg.len()
    );
    cg.decls.insert("declare i32 @_setjmp(ptr) returns_twice");
    cg.decls.insert("declare void @longjmp(ptr, i32) noreturn");
}
