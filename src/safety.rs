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

/// A type whose values hand out a raw pointer when read or destructured:
/// `*T`, `?*T`, and tuples, arrays and options of them, and function
/// types that take or return one (a callback stored in a struct expects
/// that module's pointers). Structs and enums are not followed: their
/// own fields and payloads are guarded where they are read (a foreign
/// struct's fields, a foreign enum's payloads).
pub fn holds_ptr(t: &Ty) -> bool {
    match t {
        Ty::Ptr(_) => true,
        Ty::Option(i) | Ty::Array(i, _) => holds_ptr(i),
        Ty::Result(a, b) => holds_ptr(a) || holds_ptr(b),
        Ty::Tuple(ts) => ts.iter().any(holds_ptr),
        Ty::Fn(ps, r, _) | Ty::Closure(ps, r, _) => ps.iter().any(holds_ptr) || holds_ptr(r),
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
    ///
    /// "Has a raw-pointer field" is transitive through fields of struct
    /// and enum type (`Outer { inner: Inner }` with `Inner { p: *u8 }`,
    /// a struct holding a `Vec`), since the outer module's invariants may
    /// tie the inner pointers to its other fields.
    ///
    /// An `object` of another module counts too, whatever its fields: its
    /// module's code relies on them (a `std.dict.Dict`'s `used` and
    /// `popped`, an inline cache's shape), and objects have no other way
    /// to keep them private.
    pub fn foreign_raw_struct(&self, cx: &FnCx, t: &Ty) -> bool {
        let Ty::Struct(sid) = t else { return false };
        let Some(&m) = self.struct_module.get(sid) else { return false };
        if m == cx.module {
            return false;
        }
        let sd = &self.prog.structs[*sid as usize];
        sd.object.is_some() || sd.fields.iter().any(|(_, t)| self.deep_raw(t, 0))
    }

    /// The `object` type whose field or trailing array place `e` writes
    /// directly (`o.f`, `o.f.g`, `o.f[i]`, `o.items[i]`, `o.items[a..b][i]`),
    /// if any.
    fn written_object(e: &TExpr) -> Option<&Ty> {
        match &e.kind {
            TK::Field(b, _) => match &b.kind {
                TK::Deref(o) if matches!(o.ty, Ty::Gc(_)) => Some(&b.ty),
                _ if matches!(b.ty, Ty::Struct(_) | Ty::Tuple(_) | Ty::Array(..)) => Self::written_object(b),
                _ => None,
            },
            TK::Index(b, _) | TK::SliceRange(b, _, _) => match &b.kind {
                TK::Trailing(o) => match &o.ty {
                    Ty::Gc(t) => Some(&**t),
                    _ => None,
                },
                TK::SliceRange(..) => Self::written_object(b),
                _ if matches!(b.ty, Ty::Array(..)) => Self::written_object(b),
                _ => None,
            },
            _ => None,
        }
    }

    /// A literal of an `object` of another, `#![safe]` module (`new(h,
    /// dict.Dict { .. })`): its fields are that module's to set.
    pub fn check_safe_object_literal(&self, cx: &FnCx, sid: u32, span: Span) -> Res<()> {
        if cx.unsafe_depth > 0 || cx.vm.is_some() || self.generated_files.contains(&span.file) || self.prog.structs[sid as usize].object.is_none() {
            return Ok(());
        }
        let Some(&m) = self.struct_module.get(&sid) else { return Ok(()) };
        if m == cx.module || !self.modules[m].safe {
            return Ok(());
        }
        let mname = self.modules[m].path.join(".");
        Err(Diag::new(
            span,
            format!("building a `{}`: the objects of `{}` (a `#![safe]` module) are made and written only by its own code", self.ty_name(&Ty::Struct(sid)), mname),
        )
        .note(span, format!("use the functions of `{}`", mname)))
    }

    /// Writing place `e` (or taking its address) from another module
    /// than the one declaring the `object` it lies in, when that module is
    /// `#![safe]`: its objects' fields and trailing arrays are written
    /// only by its own code (or in `unsafe { }`), so it can rely on them.
    /// In every module.
    pub fn check_safe_object_write(&self, cx: &FnCx, e: &TExpr, span: Span) -> Res<()> {
        if cx.unsafe_depth > 0 || cx.vm.is_some() || self.generated_files.contains(&span.file) {
            return Ok(());
        }
        let Some(Ty::Struct(sid)) = Self::written_object(e) else { return Ok(()) };
        let Some(&m) = self.struct_module.get(sid) else { return Ok(()) };
        if m == cx.module || !self.modules[m].safe {
            return Ok(());
        }
        let what = match &e.kind {
            TK::Field(b, idx) if matches!(b.kind, TK::Deref(_)) => self.field_name(&b.ty, *idx),
            _ => format!("a field or element of `{}`", self.ty_name(&Ty::Struct(*sid))),
        };
        let mname = self.modules[m].path.join(".");
        Err(Diag::new(span, format!("writing {}: the objects of `{}` (a `#![safe]` module) are written only by its own code", what, mname))
            .note(span, format!("use the functions of `{}`, which relies on its objects' fields for memory safety (or write it in `unsafe {{ }}`)", mname)))
    }

    /// Whether `t` contains a raw pointer anywhere by value: through
    /// tuples, arrays, options, struct fields and enum payloads (not
    /// behind a pointer, a slice or a `Gc`).
    pub fn deep_raw(&self, t: &Ty, depth: u32) -> bool {
        if depth > 32 {
            return false;
        }
        match t {
            _ if holds_ptr(t) => true,
            Ty::Option(i) | Ty::Array(i, _) => self.deep_raw(i, depth + 1),
            Ty::Result(a, b) => self.deep_raw(a, depth + 1) || self.deep_raw(b, depth + 1),
            Ty::Tuple(ts) => ts.iter().any(|t| self.deep_raw(t, depth + 1)),
            Ty::Struct(s) => self.prog.structs[*s as usize].fields.iter().any(|(_, t)| self.deep_raw(t, depth + 1)),
            Ty::Enum(e) => self.prog.enums[*e as usize].variants.iter().any(|v| v.fields.iter().any(|t| self.deep_raw(t, depth + 1))),
            _ => false,
        }
    }

    /// Whether the zero value of `t` (`..zero`, a `var` without
    /// initializer) holds a null reference that is not an option: a `*T`,
    /// a function, a `Gc`, through arrays, tuples, struct fields and the
    /// first variant's payload (`?*T`'s zero is None).
    pub fn zero_is_null_ptr(&self, t: &Ty, depth: u32) -> bool {
        if depth > 32 {
            return false;
        }
        match t {
            Ty::Ptr(_) | Ty::Fn(..) | Ty::Closure(..) | Ty::Gc(_) => true,
            Ty::Array(i, _) => self.zero_is_null_ptr(i, depth + 1),
            Ty::Tuple(ts) => ts.iter().any(|t| self.zero_is_null_ptr(t, depth + 1)),
            Ty::Struct(s) => self.prog.structs[*s as usize].fields.iter().any(|(_, t)| self.zero_is_null_ptr(t, depth + 1)),
            Ty::Enum(e) => self.prog.enums[*e as usize].variants.first().is_some_and(|v| v.fields.iter().any(|t| self.zero_is_null_ptr(t, depth + 1))),
            _ => false,
        }
    }

    /// Why the all-zero bytes are not a valid value of `t`, if they are
    /// not: what `new(h, X { .. }, n)` zero-fills a trailing array with.
    /// Zero is valid for numbers, `bool`, options (None), slices (empty),
    /// raw pointers outside `#![safe]` modules (null, as `..zero` makes
    /// them), and aggregates of those; for an enum, the variant with
    /// discriminant 0 if its payload is; for a `value` type, the variant
    /// that the word 0 encodes (a nanbox's unboxed 0.0, a low-tag
    /// integer), if its payload is not a reference. Type parameters are
    /// checked per instance.
    pub fn zero_invalid(&self, t: &Ty, safe: bool, depth: u32) -> Option<String> {
        if depth > 32 {
            return None;
        }
        let n = |c: &Self, t: &Ty| c.ty_name(t);
        match t {
            Ty::Gc(_) => Some(format!("a null `{}`", n(self, t))),
            Ty::Fn(..) | Ty::Closure(..) | Ty::Block(..) => Some(format!("a null function (`{}`)", n(self, t))),
            Ty::Ptr(_) if safe => Some(format!("a null `{}` (in a `#![safe]` module)", n(self, t))),
            Ty::Array(i, _) => self.zero_invalid(i, safe, depth + 1),
            Ty::Tuple(ts) => ts.iter().find_map(|t| self.zero_invalid(t, safe, depth + 1)),
            Ty::Result(a, b) => self.zero_invalid(a, safe, depth + 1).or_else(|| self.zero_invalid(b, safe, depth + 1)),
            Ty::Struct(sid) => {
                let sd = &self.prog.structs[*sid as usize];
                if let Some((f, _)) = sd.fields.iter().find(|(f, _)| f.starts_with("__")) {
                    return Some(format!("`{}` with its private field `{}` zeroed", n(self, t), f));
                }
                sd.fields.iter().find_map(|(_, t)| self.zero_invalid(t, safe, depth + 1))
            }
            Ty::Enum(e) => {
                let ed = &self.prog.enums[*e as usize];
                match ed.variants.iter().find(|v| v.discr == 0) {
                    None => Some(format!("no variant of `{}` (none has discriminant 0)", n(self, t))),
                    Some(v) => v.fields.iter().find_map(|t| self.zero_invalid(t, safe, depth + 1)),
                }
            }
            Ty::Value(vi) => {
                let d = &self.prog.values[*vi as usize];
                match crate::values::zero_variant(d) {
                    None => Some(format!("no variant of `{}` (the word 0 encodes none)", n(self, t))),
                    Some(i) => match &d.variants[i].payload {
                        Some(pt @ (Ty::Gc(_) | Ty::Ptr(_))) => {
                            Some(format!("`{}.{}` with a null `{}`", n(self, t), d.variants[i].name, n(self, pt)))
                        }
                        _ => None,
                    },
                }
            }
            _ => None,
        }
    }

    /// Whether every bit pattern of `size_of<t>()` bytes is a valid `t`
    /// (and holds no reference): numbers, vectors, and arrays, tuples and
    /// structs (without private fields) of them. What `new_uninit` leaves
    /// in a trailing array.
    pub fn any_bits_valid(&self, t: &Ty, depth: u32) -> bool {
        if depth > 32 {
            return false;
        }
        match t {
            Ty::Int(_) | Ty::F32 | Ty::F64 | Ty::Vector(..) | Ty::Unit => true,
            Ty::Array(i, _) => self.any_bits_valid(i, depth + 1),
            Ty::Tuple(ts) => ts.iter().all(|t| self.any_bits_valid(t, depth + 1)),
            Ty::Struct(sid) => {
                let sd = &self.prog.structs[*sid as usize];
                sd.object.is_none() && sd.fields.iter().all(|(f, t)| !f.starts_with("__") && self.any_bits_valid(t, depth + 1))
            }
            _ => false,
        }
    }

    /// `new(a, v)` (and `new(a, |..| ..)`) trust `a.alloc_bytes(size,
    /// align)` to return `size` writable bytes: in a `#![safe]` module,
    /// only an `alloc_bytes` marked `#[raw]` (`mem.Arena`, `mem.Malloc`)
    /// says so. A plain one could return `&one_byte`, and `new` would
    /// write past it. `call` is the checked `alloc_bytes` call.
    pub fn check_trusted_alloc(&self, cx: &FnCx, call: &TExpr, an: &str, span: Span) -> Res<()> {
        if !self.safe_here(cx) {
            return Ok(());
        }
        if let TK::Call(f, _) = &call.kind {
            if let TK::FnRef(id) = f.kind {
                if self.raw_fns.contains(&id) {
                    return Ok(());
                }
            }
        }
        self.raw_op(
            cx,
            span,
            "raw_call",
            || format!("`new(a, ..)` with an allocator whose `alloc_bytes` is not `#[raw]` (`a` is `{}`)", an),
            "mark `alloc_bytes` `#[raw]` (its contract: it returns `size` writable bytes aligned to `align`), or use `mem.Arena` or `mem.Malloc`",
        )
    }

    /// The module declaring enum or `value` type `t`, if it is one.
    fn sum_module(&self, t: &Ty) -> Option<usize> {
        match t {
            Ty::Enum(e) => self.enum_module.get(e).copied(),
            Ty::Value(v) => self.value_module.get(v).copied(),
            _ => None,
        }
    }

    /// Building variant `idx` of `t` (`lib.E.Buf(p, n)`): a raw-pointer
    /// operation if `t` is another module's enum or `value` type and the
    /// variant's payload holds a raw pointer (it means what that module
    /// says: a buffer, a sentinel).
    pub fn check_variant_build(&self, cx: &FnCx, t: &Ty, fields: &[Ty], span: Span) -> Res<()> {
        if !self.safe_here(cx) || !fields.iter().any(holds_ptr) || self.sum_module(t).is_none_or(|m| m == cx.module) {
            return Ok(());
        }
        let n = self.ty_name(t);
        self.raw_op(
            cx,
            span,
            "struct_lit",
            || format!("building a variant of `{}` (another module's type) with a raw-pointer payload", n),
            "use its module's functions to make one",
        )
    }

    /// Binding a payload of type `ft` from a variant of `t` in a pattern
    /// (`match e { lib.E.Buf(p, n) => .. }`): reading a raw pointer that
    /// another module's enum holds, as `v.ptr` is for a struct.
    pub fn check_variant_bind(&self, cx: &FnCx, t: &Ty, ft: &Ty, span: Span) -> Res<()> {
        if !self.safe_here(cx) || !holds_ptr(ft) || self.sum_module(t).is_none_or(|m| m == cx.module) {
            return Ok(());
        }
        let n = self.ty_name(t);
        self.raw_op(
            cx,
            span,
            "ptr_field",
            || format!("binding a raw-pointer payload of `{}` (another module's type)", n),
            "use `_` for the pointer, or the methods of its module",
        )
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
        self.check_safe_object_write(cx, e, span)?;
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
                            || format!("writing {} (a field of another module's object, or struct with raw pointers)", f),
                            "use the methods of its module (`v.clear()`, not `v.len = 0`)",
                        );
                    }
                    e = b;
                }
                TK::Index(b, _) if matches!(b.ty, Ty::Array(..)) => e = b,
                // `o.items[i]`: an element of another module's object.
                TK::Index(b, _) | TK::SliceRange(b, _, _) if matches!(b.kind, TK::Trailing(_) | TK::SliceRange(..)) => e = b,
                TK::Trailing(o) => {
                    let Ty::Gc(t) = &o.ty else { return Ok(()) };
                    if self.foreign_raw_struct(cx, t) {
                        let n = self.ty_name(t);
                        return self.raw_op(
                            cx,
                            span,
                            "field_write",
                            || format!("writing the trailing array of `{}` (another module's object)", n),
                            "use the methods of its module",
                        );
                    }
                    return Ok(());
                }
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
