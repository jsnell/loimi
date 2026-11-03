//! `#![safe]` modules (language.md section 1.3): raw-pointer operations
//! need `unsafe { }`. A check only: it never changes the typed tree.
//!
//! The rule: in a safe module a `*T` is a reference to one live `T`.
//! Following it (`*p`, `p.f`, `p.read_unaligned()`, atomics) is allowed;
//! making it point anywhere else is not: indexing (`p[i]`, `p[a..b]`),
//! arithmetic (`p + n`, `p.offset(n)`), `p.slice(n)`, casts to a pointer
//! (from an integer, another pointer type, a function or a `Gc`) and
//! casts that forge a tag or reference (integer to enum or `value`).
//! Neither is reaching around another module's invariants: the raw
//! pointer fields of a struct declared in another module (a slice's
//! `.ptr`), writing any field of such a struct, or building one with a
//! literal. `..zero` may not make a null `*T`, `assume` and
//! `unreachable` (undefined if wrong) are out, and so are calls of
//! `#[raw]` and extern functions, which take pointers to more than one
//! value or free memory. Lifetimes are not checked (the known hole).

use crate::check::{Checker, FnCx};
use crate::diag::{Diag, Res, Span};
use crate::tir::{TExpr, Ty, TK};

/// A type that holds a raw pointer directly: `*T` or `?*T`.
pub fn holds_ptr(t: &Ty) -> bool {
    match t {
        Ty::Ptr(_) => true,
        Ty::Option(i) => matches!(**i, Ty::Ptr(_)),
        _ => false,
    }
}

impl Checker<'_> {
    /// Whether raw-pointer operations are checked here: a `#![safe]`
    /// module (or any, in audit mode), outside `unsafe { }` and outside
    /// vm code (whose operand accesses the verifier vouches for).
    pub fn safe_here(&self, cx: &FnCx) -> bool {
        cx.unsafe_depth == 0 && cx.vm.is_none() && (self.modules[cx.module].safe || self.safety_audit.is_some())
    }

    /// A raw-pointer operation at `span`: an error in a safe module.
    /// `what` names it, `hint` says what to write instead.
    pub fn raw_op(&self, cx: &FnCx, span: Span, cat: &'static str, what: impl FnOnce() -> String, hint: &str) -> Res<()> {
        if !self.safe_here(cx) || self.generated_files.contains(&span.file) {
            return Ok(());
        }
        if let Some(a) = &self.safety_audit {
            a.borrow_mut().insert((self.modules[cx.module].path.join("."), cat, span.file, span.lo));
            return Ok(());
        }
        Err(Diag::new(span, format!("{} is a raw-pointer operation: not allowed in a `#![safe]` module outside `unsafe {{ }}`", what()))
            .note(span, hint.to_string()))
    }

    /// Audit mode only: count an allowed use (a dereference) by category.
    pub fn audit_count(&self, cx: &FnCx, cat: &'static str, span: Span) {
        if let Some(a) = &self.safety_audit {
            if cx.unsafe_depth == 0 && cx.vm.is_none() {
                a.borrow_mut().insert((self.modules[cx.module].path.join("."), cat, span.file, span.lo));
            }
        }
    }

    /// A struct with a raw-pointer field declared in another module: its
    /// pointers mean what that module says (a buffer, a sentinel), and
    /// its fields hold that module's invariants (`Vec.len` <= `Vec.cap`),
    /// which the module's own `unsafe { }` blocks may rely on (so this
    /// holds for structs of other safe modules too).
    pub fn foreign_raw_struct(&self, cx: &FnCx, t: &Ty) -> bool {
        let Ty::Struct(sid) = t else { return false };
        let Some(&m) = self.struct_module.get(sid) else { return false };
        if m == cx.module {
            return false;
        }
        self.prog.structs[*sid as usize].fields.iter().any(|(_, t)| holds_ptr(t))
    }

    fn field_name(&self, t: &Ty, idx: u32) -> String {
        match t {
            Ty::Struct(sid) => {
                let s = &self.prog.structs[*sid as usize];
                let sname = crate::check::short_name(&s.name).replace("__", ".");
                format!("`{}.{}`", sname, s.fields[idx as usize].0)
            }
            _ => "a field".into(),
        }
    }

    /// Writing to (or taking the address of) place `e`: an error if it is
    /// inside a foreign struct with raw pointers.
    pub fn check_raw_place(&self, cx: &FnCx, e: &TExpr, span: Span) -> Res<()> {
        if !self.safe_here(cx) {
            return Ok(());
        }
        let mut e = e;
        loop {
            match &e.kind {
                TK::Field(b, idx) => {
                    if self.foreign_raw_struct(cx, &b.ty) {
                        let f = self.field_name(&b.ty, *idx);
                        return self.raw_op(
                            cx,
                            span,
                            "field_write",
                            || format!("writing {} (a field of another module's struct with raw pointers)", f),
                            "use the methods of its module (`v.clear()`, not `v.len = 0`)",
                        );
                    }
                    e = b;
                }
                TK::Index(b, _) if matches!(b.ty, Ty::Array(..)) => e = b,
                _ => return Ok(()),
            }
        }
    }

    /// Reading field `r` (a typed `TK::Field`): a raw pointer held by a
    /// foreign struct or a slice is a raw-pointer operation.
    pub fn check_field_read(&self, cx: &FnCx, r: &TExpr) -> Res<()> {
        let TK::Field(b, idx) = &r.kind else {
            return Ok(());
        };
        if matches!(b.kind, TK::Deref(_)) {
            self.audit_count(cx, "ok_field_via_ptr", r.span);
        }
        if !holds_ptr(&r.ty) {
            return Ok(());
        }
        if matches!(b.ty, Ty::Slice(_)) {
            return self.raw_op(
                cx,
                r.span,
                "ptr_field",
                || "reading a slice's `.ptr`".into(),
                "pass the slice itself (a function that wants a pointer and a length should take a slice)",
            );
        }
        if self.foreign_raw_struct(cx, &b.ty) {
            let f = self.field_name(&b.ty, *idx);
            return self.raw_op(
                cx,
                r.span,
                "ptr_field",
                || format!("reading the pointer {} of another module's struct", f),
                "use the methods of its module (`v.items()`, `v.at(i)`)",
            );
        }
        Ok(())
    }
}

/// The cast from `from` to `to` (different types) as a raw-pointer
/// operation: its category and description, or None if it is safe.
pub fn raw_cast(from: &Ty, to: &Ty) -> Option<(&'static str, &'static str)> {
    match (from, to) {
        (Ty::Int(_), Ty::Ptr(_)) => Some(("cast_to_ptr", "casting an integer to a pointer")),
        (_, Ty::Ptr(_)) => Some(("cast_to_ptr", "a pointer cast")),
        (Ty::Int(_), Ty::Enum(_)) => Some(("cast_forge", "casting an integer to an enum")),
        (Ty::Int(_), Ty::Value(_)) => Some(("cast_forge", "casting an integer to a `value` type")),
        _ => None,
    }
}
