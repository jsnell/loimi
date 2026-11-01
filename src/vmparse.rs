//! Parsing of `vm` and `value` declarations.

use crate::ast::*;
use crate::diag::Res;
use crate::lexer::Tok;
use crate::parser::Parser;
use crate::vmast::*;

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
            variants: vec![],
            sync: None,
            is_pub,
        };
        while !self.eat("}") {
            let item_attrs = self.parse_attrs()?;
            let kw = match self.peek() {
                Tok::Ident(s) => s.clone(),
                _ => return self.err("expected a vm item (state, operand, encoding, op, ...)"),
            };
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
                    let verify = if self.eat_kw("verify") { Some(self.parse_expr()?) } else { None };
                    self.expect(";")?;
                    vm.operands.push(OperandDecl { name: n, body, verify });
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
                    vm.caches.push(CacheDecl { name: n, fields });
                }
                "op" => {
                    self.bump();
                    let (name, params, format, cache) = self.parse_op_head()?;
                    let body = self.vm_block()?;
                    vm.ops.push(OpDecl { name, params, format, cache, body, family: None });
                }
                "family" => {
                    self.bump();
                    let (name, params, format, cache) = self.parse_op_head()?;
                    self.expect("{")?;
                    while !self.eat("}") {
                        let member = self.ident()?;
                        let body = self.vm_block()?;
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
                    vm.supers.push(SuperDecl { name: n, parts });
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
                    let body = self.vm_block()?;
                    vm.entries.push(EntryDecl { name: n, params, ret, body });
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
                "sync" => {
                    self.bump();
                    vm.sync = Some(self.vm_block()?);
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
                    vm.variants.push(VariantDecl { name: n, wraps, before });
                }
                _ => return self.err(format!("unknown vm item `{}`", kw)),
            }
        }
        Ok(vm)
    }

    fn vm_block(&mut self) -> Res<Block> {
        self.in_vm = true;
        let b = self.parse_block();
        self.in_vm = false;
        b
    }

    fn parse_op_head(&mut self) -> Res<(Ident, Vec<OpParam>, Ident, Option<Ident>)> {
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
        Ok((name, params, format, cache))
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
            variants.push(ValueVariant { name: vn, payload, unboxed });
            if !self.eat(",") {
                break;
            }
        }
        self.expect("}")?;
        Ok(ValueDecl { name, repr, variants, is_pub })
    }
}
