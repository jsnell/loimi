//! Parsing of `vm` and `value` declarations.

use crate::ast::*;
use crate::diag::{Diag, Res};
use crate::lexer::Tok;
use crate::parser::Parser;
use crate::vmast::*;

/// The name of the generated breakpoint op for words of inline cache
/// layout `cache` (None: no cache), see `desugar_break`.
pub fn break_op_name(cache: Option<&str>) -> String {
    match cache {
        None => "BREAK".to_string(),
        Some(c) => format!("BREAK__{c}"),
    }
}

/// `on break { .. }`: one op per inline cache layout, plus one without a
/// cache, all running the block. `Vm.set_break` writes the one with the
/// patched instruction's (or its first part's) cache over its opcode, so
/// the breakpoint has the instruction's size: walks over the code
/// (`resume_at`, the disassembler, `deopt`, the step-over) stay on
/// instruction boundaries with breakpoints in place.
fn desugar_break(vm: &mut VmDecl) -> Res<()> {
    let Some(body) = vm.on_break.clone() else { return Ok(()) };
    let Some(fmt) = vm.formats.first() else { return Ok(()) };
    if let Some(o) = vm.ops.iter().find(|o| o.name.name == "BREAK" || o.name.name.starts_with("BREAK__")) {
        return Err(Diag::new(o.name.span, format!("`{}` is the name of a generated breakpoint op (this vm has `on break`)", o.name.name)));
    }
    if vm.ops.iter().any(|o| o.stack.is_some()) {
        return Err(Diag::new(body.span, "`on break` is not supported in stack vms yet"));
    }
    let span = body.span;
    let caches: Vec<Option<Ident>> = std::iter::once(None).chain(vm.caches.iter().map(|c| Some(c.name.clone()))).collect();
    for c in caches {
        vm.ops.push(OpDecl {
            name: Ident { name: break_op_name(c.as_ref().map(|c| c.name.as_str())), span },
            params: vec![],
            format: fmt.name.clone(),
            cache: c,
            body: body.clone(),
            family: None,
            then: None,
            stack: None,
            accumulate: false,
        });
    }
    Ok(())
}

impl Parser {
    pub fn parse_vm(&mut self, attrs: Attrs, is_pub: bool) -> Res<VmDecl> {
        self.expect_kw("vm")?;
        let name = self.ident()?;
        self.expect("{")?;
        let mut vm = VmDecl {
            name,
            attrs,
            context: None,
            state: vec![],
            operands: vec![],
            word: None,
            formats: vec![],
            caches: vec![],
            ops: vec![],
            supers: vec![],
            fns: vec![],
            entries: vec![],
            on_raise: None,
            on_break: None,
            variants: vec![],
            sync: None,
            reload: None,
            verify_ctx: vec![],
            verify_owner: None,
            checks: vec![],
            stack_entry: None,
            accs: vec![],
            acc_list: vec![],
            acc_ops: vec![],
            is_pub,
        };
        // `inline for` variables replace every matching token in the body,
        // so one named like a state variable would silently replace it.
        let mut loop_vars: Vec<Ident> = Vec::new();
        self.vm_polls.clear();
        while !self.eat("}") {
            if self.is_kw("inline") && self.is_kw_at(1, "for") {
                loop_vars.extend(self.expand_inline_for(false)?);
                continue;
            }
            // `#[cfg(NAME)]` on a vm item: parsed, then dropped when NAME
            // is not enabled (the vm is as if the item were not written).
            let keep = self.take_cfg()?;
            let saved = if keep { None } else { Some((vm.clone(), self.vm_loop_vars.len())) };
            let mark = self.allow_mark();
            let item_attrs = self.parse_attrs()?;
            let kw = match self.peek() {
                Tok::Ident(s) => s.clone(),
                _ => return self.err("expected a vm item (state, operand, encoding, op, ...)"),
            };
            let (what, known): (String, &[&str]) = match kw.as_str() {
                "fn" => ("a vm function".into(), crate::parser::FN_ATTRS),
                "super" => ("a `super`".into(), &["forward"]),
                "on" => ("`on raise` or `on break`".into(), &[]),
                "op" => ("an `op`".into(), &["accumulate", "poll"]),
                "family" => ("a `family`".into(), &["poll"]),
                "variant" => ("a `variant`".into(), &["unfused"]),
                "operand" | "entry" => (format!("an `{}`", kw), &[]),
                _ => (format!("a `{}`", kw), &[]),
            };
            crate::parser::check_attrs(&item_attrs, &what, known)?;
            match kw.as_str() {
                "context" => {
                    self.bump();
                    let n = self.ident()?;
                    self.expect(":")?;
                    let ty = self.parse_type()?;
                    self.expect(";")?;
                    vm.context = Some(Param { name: n, ty });
                }
                "state" => {
                    self.bump();
                    self.expect("{")?;
                    while !self.is("}") {
                        let n = self.ident()?;
                        self.expect(":")?;
                        let ty = self.parse_type()?;
                        vm.state.push(Param { name: n, ty });
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect("}")?;
                }
                "operand" => {
                    self.bump();
                    let n = self.ident()?;
                    self.expect("=")?;
                    let body = self.parse_expr()?;
                    let mut verify = None;
                    let mut display = None;
                    let mut konst = None;
                    loop {
                        if verify.is_none() && self.eat_kw("verify") {
                            verify = Some(self.parse_expr()?);
                        } else if konst.is_none() && self.eat_kw("const") {
                            konst = Some(self.parse_expr()?);
                        } else if display.is_none() && self.eat_kw("display") {
                            let t = self.bump();
                            let Tok::Str(s) = t.tok else {
                                return Err(Diag::new(t.span, "expected a string after `display`, like \"r{}\""));
                            };
                            let s = String::from_utf8_lossy(&s).into_owned();
                            if s.matches("{}").count() != 1 {
                                return Err(Diag::new(t.span, "a `display` string needs exactly one `{}`"));
                            }
                            display = Some(s);
                        } else {
                            break;
                        }
                    }
                    self.expect(";")?;
                    vm.operands.push(OperandDecl { name: n, body, verify, konst, display });
                }
                "encoding" => {
                    self.bump();
                    let kind = self.ident()?;
                    if kind.name != "fixed" {
                        return Err(crate::diag::Diag::new(kind.span, "only `encoding fixed(T)` is supported so far"));
                    }
                    self.expect("(")?;
                    vm.word = Some(self.parse_type()?);
                    self.expect(")")?;
                    self.expect("{")?;
                    while !self.eat("}") {
                        self.expect_kw("format")?;
                        let fname = self.ident()?;
                        self.expect("{")?;
                        let mut fields = Vec::new();
                        while !self.is("}") {
                            let f = self.ident()?;
                            self.expect(":")?;
                            let (bits, signed) = match self.bump().tok {
                                Tok::Int(n) => (n as u32, false),
                                Tok::Ident(s) if s.starts_with('s') && s[1..].parse::<u32>().is_ok() => {
                                    (s[1..].parse().unwrap(), true)
                                }
                                _ => return self.err("expected a bit width like `8` or `s16`"),
                            };
                            fields.push(FormatField { name: f, bits, signed });
                            if !self.eat(",") {
                                break;
                            }
                        }
                        self.expect("}")?;
                        vm.formats.push(FormatDecl { name: fname, fields });
                    }
                }
                "cache" => {
                    self.bump();
                    let n = self.ident()?;
                    if self.eat("=") {
                        // `cache X = S;`: the fields of struct `S`
                        // (vmexpand::cache_structs fills them in).
                        let t = self.parse_type()?;
                        self.expect(";")?;
                        vm.caches.push(CacheDecl { name: n, fields: vec![], from: Some(t) });
                    } else {
                        self.expect("{")?;
                        let mut fields = Vec::new();
                        while !self.is("}") {
                            let f = self.ident()?;
                            self.expect(":")?;
                            let weak = self.eat_kw("weak");
                            let t = self.parse_type()?;
                            fields.push((f, t, weak));
                            if !self.eat(",") {
                                break;
                            }
                        }
                        self.expect("}")?;
                        vm.caches.push(CacheDecl { name: n, fields, from: None });
                    }
                }
                "poll" => {
                    // `poll gc { .. }`: statements that `#[poll(gc)]` on
                    // an op puts at its start, and `poll gc;` anywhere in
                    // the vm's code.
                    self.bump();
                    let n = self.ident()?;
                    if let Some((p, _)) = self.vm_polls.iter().find(|(p, _)| p.name == n.name) {
                        return Err(Diag::new(n.span, format!("poll `{}` is declared twice", n.name)).note(p.span, "first declared here"));
                    }
                    let body = self.vm_block()?;
                    self.vm_polls.push((n, keep.then_some(body)));
                }
                "op" => {
                    self.bump();
                    let (name, params, format, cache, then, stack) = self.parse_op_head()?;
                    let mut body = self.vm_block()?;
                    self.add_polls(&item_attrs, &mut body)?;
                    vm.ops.push(OpDecl { name, params, format, cache, body, family: None, then, stack, accumulate: item_attrs.has("accumulate") });
                }
                "family" => {
                    self.bump();
                    let (name, params, format, cache, then, stack) = self.parse_op_head()?;
                    self.expect("{")?;
                    while !self.eat("}") {
                        let member = self.ident()?;
                        let mut body = self.vm_block()?;
                        self.add_polls(&item_attrs, &mut body)?;
                        let mname = if member.name == "generic" {
                            name.clone()
                        } else {
                            member
                        };
                        vm.ops.push(OpDecl {
                            name: mname,
                            params: params.clone(),
                            format: format.clone(),
                            cache: cache.clone(),
                            body,
                            family: Some(name.name.clone()),
                            then: then.clone(),
                            stack: stack.clone(),
                            accumulate: false,
                        });
                    }
                }
                "super" => {
                    self.bump();
                    let n = self.ident()?;
                    self.expect("=")?;
                    let mut parts = vec![self.ident()?];
                    self.expect(";")?;
                    while matches!(self.peek(), Tok::Ident(s) if s.chars().all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()))
                        && self.is_at(1, ";")
                    {
                        parts.push(self.ident()?);
                        self.expect(";")?;
                    }
                    vm.supers.push(SuperDecl { name: n, parts, forward: item_attrs.has("forward") });
                }
                "fn" => {
                    self.in_vm = true;
                    let f = self.parse_fn(item_attrs, Linkage::Internal, false);
                    self.in_vm = false;
                    vm.fns.push(f?);
                }
                "entry" => {
                    self.bump();
                    let n = self.ident()?;
                    self.expect("(")?;
                    let mut params = Vec::new();
                    while !self.is(")") {
                        let pn = self.ident()?;
                        self.expect(":")?;
                        let ty = self.parse_type()?;
                        params.push(Param { name: pn, ty });
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect(")")?;
                    self.expect("->")?;
                    let ret = self.parse_type()?;
                    let mut effects = Attrs::default();
                    if self.eat_kw("uses") {
                        loop {
                            let e = self.ident()?;
                            effects.list.push(("uses".into(), vec![e.name], e.span));
                            if !self.eat(",") {
                                break;
                            }
                        }
                    }
                    let body = self.vm_block()?;
                    vm.entries.push(EntryDecl { name: n, params, ret, body, effects });
                }
                "on" if self.is_kw_at(1, "break") => {
                    // `on break { .. }`: the handler of the generated
                    // breakpoint ops (see `desugar_break`).
                    self.bump();
                    self.bump();
                    if vm.on_break.is_some() {
                        return self.err("a vm has at most one `on break` block");
                    }
                    vm.on_break = Some(self.vm_block()?);
                }
                "on" => {
                    self.bump();
                    self.expect_kw("raise")?;
                    self.expect("(")?;
                    let pn = self.ident()?;
                    self.expect(":")?;
                    let ty = self.parse_type()?;
                    self.expect(")")?;
                    let body = self.vm_block()?;
                    vm.on_raise = Some(RaiseDecl { param: Param { name: pn, ty }, body });
                }
                "verify_context" => {
                    // `verify_context proto: *Proto, lim: *Limits;`: the
                    // verifier's parameters after the code, in order (also
                    // across several declarations).
                    self.bump();
                    loop {
                        // `owner proto: Gc<Proto>`: the object that holds
                        // the code (`code: verified [W] by Vm`).
                        let owner = if self.is_kw("owner") && matches!(self.peek_at(1), Tok::Ident(_)) {
                            Some(self.bump().span)
                        } else {
                            None
                        };
                        let n = self.ident()?;
                        self.expect(":")?;
                        let ty = self.parse_type()?;
                        if vm.verify_ctx.iter().any(|p: &Param| p.name.name == n.name) {
                            return Err(Diag::new(n.span, format!("`{}` is declared twice in `verify_context`", n.name)));
                        }
                        if let Some(sp) = owner {
                            if vm.verify_owner.is_some() {
                                return Err(Diag::new(sp, "a vm has at most one `owner` verify context"));
                            }
                            let is_gc = matches!(&ty.kind, TypeKind::Named(p, a) if p.len() == 1 && p[0].name == "Gc" && a.len() == 1);
                            if !is_gc {
                                return Err(Diag::new(ty.span, "the `owner` verify context is the object holding the code: `owner p: Gc<Proto>`"));
                            }
                            vm.verify_owner = Some(vm.verify_ctx.len());
                        }
                        vm.verify_ctx.push(Param { name: n, ty });
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect(";")?;
                }
                "verify" => {
                    // `verify handlers { .. }`: a `bool` block the verifier
                    // evaluates after checking the code, with the verify
                    // context, `code` and `is_insn_boundary(p)` in scope.
                    self.bump();
                    let n = self.ident()?;
                    if vm.checks.iter().any(|(c, _)| c.name == n.name) {
                        return Err(Diag::new(n.span, format!("`verify {}` is declared twice", n.name)));
                    }
                    let b = self.parse_block()?;
                    vm.checks.push((n, b));
                }
                "stack_entry" => {
                    // `stack_entry word.nin;`: the operand stack depth that
                    // code starts with, from the verify context.
                    self.bump();
                    if vm.stack_entry.is_some() {
                        return self.err("a vm has at most one `stack_entry`");
                    }
                    vm.stack_entry = Some(self.parse_expr()?);
                    self.expect(";")?;
                }
                "accumulator" => {
                    // `accumulator acc: u64 for ri, rq;`
                    self.bump();
                    let n = self.ident()?;
                    self.expect(":")?;
                    let ty = self.parse_type()?;
                    self.expect_kw("for")?;
                    let mut kinds = vec![self.ident()?];
                    while self.eat(",") {
                        kinds.push(self.ident()?);
                    }
                    self.expect(";")?;
                    vm.accs.push(AccDecl { name: n, ty, kinds });
                }
                "accumulate" => {
                    // `accumulate ADD, SUB;`: as `#[accumulate]` on each.
                    self.bump();
                    vm.acc_list.push(self.ident()?);
                    while self.eat(",") {
                        vm.acc_list.push(self.ident()?);
                    }
                    self.expect(";")?;
                }
                "sync" => {
                    self.bump();
                    vm.sync = Some(self.vm_block()?);
                }
                "reload" => {
                    self.bump();
                    if vm.reload.is_some() {
                        return self.err("a vm has at most one `reload` block");
                    }
                    vm.reload = Some(self.vm_block()?);
                }
                "variant" => {
                    self.bump();
                    let n = self.ident()?;
                    self.expect_kw("wraps")?;
                    let wraps = if self.eat_kw("all") {
                        None
                    } else {
                        self.expect("[")?;
                        let mut list = Vec::new();
                        while !self.is("]") {
                            list.push(self.ident()?);
                            if !self.eat(",") {
                                break;
                            }
                        }
                        self.expect("]")?;
                        Some(list)
                    };
                    self.expect("{")?;
                    self.expect_kw("before")?;
                    let before = self.vm_block()?;
                    self.expect("}")?;
                    vm.variants.push(VariantDecl { name: n, wraps, before, unfused: item_attrs.has("unfused") });
                }
                _ => return self.err(format!("unknown vm item `{}`", kw)),
            }
            self.close_allows(mark);
            if let Some((v, n)) = saved {
                vm = v;
                self.vm_loop_vars.truncate(n);
            }
        }
        loop_vars.append(&mut self.vm_loop_vars);
        desugar_break(&mut vm)?;
        crate::vmacc::desugar(&mut vm)?;
        for v in &loop_vars {
            if vm.state.iter().chain(vm.context.iter()).any(|p| p.name.name == v.name) {
                return Err(crate::diag::Diag::new(
                    v.span,
                    format!("`inline for` variable `{}` has the name of a vm state variable, which it would replace in the body", v.name),
                ));
            }
        }
        Ok(vm)
    }

    /// `#[poll(a, b)]` on an op or family: the polls' statements, in
    /// that order, before the body's.
    fn add_polls(&self, attrs: &Attrs, body: &mut Block) -> Res<()> {
        let mut pre = Vec::new();
        for (n, args, sp) in &attrs.list {
            if n != "poll" {
                continue;
            }
            if args.is_empty() {
                return Err(Diag::new(*sp, "`#[poll(..)]` names the polls to run at the start of the op: `#[poll(gc)]`"));
            }
            for a in args {
                pre.extend(self.poll_body(&Ident { name: a.clone(), span: *sp })?);
            }
        }
        if !pre.is_empty() {
            pre.append(&mut body.stmts);
            body.stmts = pre;
        }
        Ok(())
    }

    fn vm_block(&mut self) -> Res<Block> {
        self.in_vm = true;
        let b = self.parse_block();
        self.in_vm = false;
        b
    }

    #[allow(clippy::type_complexity)]
    fn parse_op_head(&mut self) -> Res<(Ident, Vec<OpParam>, Ident, Option<Ident>, Option<Ident>, Option<StackEffect>)> {
        let name = self.ident()?;
        self.expect("(")?;
        let mut params = Vec::new();
        while !self.is(")") {
            let n = self.ident()?;
            self.expect(":")?;
            let k = self.ident()?;
            params.push(OpParam { name: n, kind: k });
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        self.expect(":")?;
        let format = self.ident()?;
        let cache = if self.eat("+") { Some(self.ident()?) } else { None };
        let then = if self.eat_kw("then") { Some(self.ident()?) } else { None };
        let stack = if self.is_kw("stack") && self.is_at(1, "(") { Some(self.parse_stack_effect()?) } else { None };
        Ok((name, params, format, cache, then, stack))
    }

    /// `stack(a b -- c)`, `stack(xs[n] -- arr)`, `stack(.. --)`. Commas
    /// between items are optional.
    fn parse_stack_effect(&mut self) -> Res<StackEffect> {
        let lo = self.bump().span;
        self.expect("(")?;
        let (mut rest, mut after) = (false, false);
        let (mut ins, mut outs) = (Vec::new(), Vec::new());
        while !self.is(")") {
            if self.is("-") && self.is_at(1, "-") {
                if after {
                    return self.err("a stack effect has one `--`");
                }
                self.bump();
                self.bump();
                after = true;
                continue;
            }
            if self.eat(",") {
                continue;
            }
            if self.is("..") {
                if after || rest || !ins.is_empty() {
                    return self.err("`..` (every value on the stack) can only come first among the inputs");
                }
                self.bump();
                rest = true;
                continue;
            }
            let name = self.ident()?;
            let count = if self.eat("[") {
                let e = self.parse_expr()?;
                self.expect("]")?;
                Some(e)
            } else {
                None
            };
            if after { &mut outs } else { &mut ins }.push(StackItem { name, count });
        }
        let hi = self.bump().span;
        if !after {
            return Err(Diag::new(lo.to(hi), "a stack effect needs `--` between the inputs and the outputs: `stack(a b -- c)`"));
        }
        Ok(StackEffect { rest, ins, outs })
    }

    pub fn parse_value_decl(&mut self, is_pub: bool) -> Res<ValueDecl> {
        self.expect_kw("value")?;
        let name = self.ident()?;
        self.expect_kw("repr")?;
        let repr = self.ident()?;
        self.expect("{")?;
        let mut variants = Vec::new();
        while !self.is("}") {
            let vn = self.ident()?;
            let payload = if self.eat("(") {
                let t = self.parse_type()?;
                self.expect(")")?;
                Some(t)
            } else {
                None
            };
            let unboxed = self.eat_kw("unboxed");
            let block = if self.is_kw("block") {
                let sp = self.bump().span;
                let t = self.bump();
                let Tok::Int(n) = t.tok else {
                    return Err(Diag::new(t.span, "`block` takes the block width in bits: `Int(i32) block 48`"));
                };
                Some((n.min(1 << 16) as u32, sp.to(t.span)))
            } else {
                None
            };
            variants.push(ValueVariant { name: vn, payload, unboxed, block });
            if !self.eat(",") {
                break;
            }
        }
        self.expect("}")?;
        Ok(ValueDecl { name, repr, variants, is_pub, scan_raw: None })
    }
}
