//! Splitting one generated LLVM module into parts that clang compiles in
//! parallel (main.rs). LLVM optimizes and generates code for a module on
//! one thread; an interpreter with hundreds of handlers is one large
//! module, so its build time was one core's.
//!
//! - Roots (handlers, large functions, non-internal functions) are cut
//!   into contiguous runs in module order, balanced by size. The parts
//!   link in order, so a section's function order (the hot handler
//!   layout, hotorder.rs) is unchanged.
//! - Small helpers are copied, `internal`, into every part that calls
//!   them, so they inline as they did in one module.
//! - Globals are defined in part 0 and copied `available_externally`
//!   into other parts that use them: constants still fold, and every
//!   address stays unique. A helper whose address is taken is defined
//!   once, in part 0, for the same reason.
//! - Symbols shared between parts are `hidden`; a static library's are
//!   made local again after a relocatable link (main.rs).
//!
//! On anything this does not recognise, `split` returns None and the
//! module is compiled whole.

use std::collections::HashMap;

/// A helper above this many lines with more than one caller is a root:
/// LLVM would not inline it into many callers anyway. (One with one
/// caller is copied into its caller's part: LLVM inlines the last call
/// of an internal function whatever its size.)
const BIG: usize = 200;
/// A function above this many lines is a root even with one caller (the
/// generated tables, `verify`): parts stay balanced.
const HUGE: usize = 5000;

enum Item<'a> {
    /// Copied into every part: types, declarations, metadata, target.
    Pre(&'a str),
    /// `@x = internal ...` or `@x = private ...` (`at` is where the
    /// linkage word starts), or (`at` None) an `appending` global, kept
    /// in part 0 only.
    /// `at` gives a length of 0 for an exported global (default linkage).
    Global { name: &'a str, line: &'a str, at: Option<(usize, usize)> },
    Func { name: &'a str, header: &'a str, body: &'a str, lines: usize, internal: bool },
}

/// The name token at `s[i..]` (`s[i]` is '@'), quotes included.
fn name_at(s: &str, i: usize) -> Option<&str> {
    let b = s.as_bytes();
    let mut j = i + 1;
    if j < b.len() && b[j] == b'"' {
        j += 1;
        while j < b.len() && b[j] != b'"' {
            j += 1;
        }
        return (j < b.len()).then(|| &s[i..j + 1]);
    }
    while j < b.len() && (b[j].is_ascii_alphanumeric() || b"._$-".contains(&b[j])) {
        j += 1;
    }
    (j > i + 1).then(|| &s[i..j])
}

/// A name without its quotes: `@"f"` and `@f` are one symbol.
fn canon(n: &str) -> &str {
    let n = &n[1..];
    n.strip_prefix('"').and_then(|m| m.strip_suffix('"')).unwrap_or(n)
}

/// Every `@name` in `s`, with whether it is a direct callee (`@f(`).
fn refs(s: &str) -> impl Iterator<Item = (&str, bool)> {
    s.match_indices('@').filter_map(move |(i, _)| {
        let n = name_at(s, i)?;
        Some((n, s.as_bytes().get(i + n.len()) == Some(&b'(')))
    })
}

/// Splits `s` at commas outside quotes and brackets.
fn split_top(s: &str) -> Vec<&str> {
    let (mut out, mut depth, mut q, mut start) = (Vec::new(), 0i32, false, 0);
    for (i, c) in s.char_indices() {
        match c {
            '"' => q = !q,
            '(' | '[' | '{' | '<' if !q => depth += 1,
            ')' | ']' | '}' | '>' if !q => depth -= 1,
            ',' if !q && depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Splits at spaces outside quotes.
fn tokens(s: &str) -> Vec<&str> {
    let (mut out, mut q, mut start) = (Vec::new(), false, None);
    for (i, c) in s.char_indices() {
        if c == '"' {
            q = !q;
        }
        if c == ' ' && !q {
            if let Some(st) = start.take() {
                out.push(&s[st..i]);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(st) = start {
        out.push(&s[st..]);
    }
    out
}

/// A declaration of the function defined by `header`.
fn declaration(header: &str, name: &str, internal: bool) -> Option<String> {
    let at = header.find(name)?;
    let prefix = if internal { &header["define internal ".len()..at] } else { &header["define ".len()..at] };
    let open = at + name.len();
    if header.as_bytes().get(open) != Some(&b'(') {
        return None;
    }
    let (mut depth, mut q, mut close) = (0i32, false, None);
    for (i, c) in header[open..].char_indices() {
        match c {
            '"' => q = !q,
            '(' | '[' | '{' | '<' if !q => depth += 1,
            ')' | ']' | '}' | '>' if !q => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let inner = &header[open + 1..close];
    let mut params = Vec::new();
    if !inner.trim().is_empty() {
        for p in split_top(inner) {
            let t = tokens(p.trim());
            params.push(match t.as_slice() {
                [.., last] if t.len() >= 2 && last.starts_with('%') => t[..t.len() - 1].join(" "),
                _ => p.trim().to_string(),
            });
        }
    }
    let tail = &header[close..];
    let mut attrs = String::new();
    for a in ["nounwind", "noreturn"] {
        if tokens(tail).contains(&a) {
            attrs.push(' ');
            attrs.push_str(a);
        }
    }
    Some(format!("declare {}{}{}({}){}", if internal { "hidden " } else { "" }, prefix, name, params.join(", "), attrs))
}

fn parse(ir: &str) -> Option<Vec<Item<'_>>> {
    let mut items = Vec::new();
    let mut rest = ir;
    while !rest.is_empty() {
        let end = rest.find('\n').map_or(rest.len(), |i| i + 1);
        let line = &rest[..end];
        let t = line.trim_end();
        if t.starts_with("define ") {
            if !t.ends_with('{') {
                return None;
            }
            let close = rest.find("\n}\n").map(|i| i + 3).or_else(|| rest.ends_with("\n}").then_some(rest.len()))?;
            let at = t.find('@')?;
            let name = name_at(t, at)?;
            let internal = t.starts_with("define internal ");
            if !internal {
                let first = t["define ".len()..].split(' ').next()?;
                if ["private", "linkonce", "linkonce_odr", "weak", "weak_odr", "common", "extern_weak", "available_externally"].contains(&first) {
                    return None;
                }
            }
            let body = &rest[end..close];
            items.push(Item::Func { name, header: line, body, lines: body.matches('\n').count(), internal });
            rest = &rest[close..];
            continue;
        }
        if t.starts_with('@') {
            let name = name_at(t, 0)?;
            let after = t[name.len()..].strip_prefix(" = ")?;
            let off = name.len() + 3;
            let word = after.split(' ').next()?;
            match word {
                "internal" | "private" => items.push(Item::Global { name, line, at: Some((off, word.len())) }),
                "appending" => items.push(Item::Global { name, line, at: None }),
                "constant" | "global" | "unnamed_addr" | "local_unnamed_addr" | "dso_local" => items.push(Item::Global { name, line, at: Some((off, 0)) }),
                w if w.starts_with("thread_local") => items.push(Item::Global { name, line, at: Some((off, 0)) }),
                "external" => items.push(Item::Pre(line)),
                _ => return None,
            }
        } else if t.is_empty()
            || t.starts_with(';')
            || t.starts_with("declare ")
            || t.starts_with("target ")
            || t.starts_with("source_filename")
            || t.starts_with("attributes ")
            || t.starts_with('!')
            || (t.starts_with('%') && t.contains(" = type "))
        {
            items.push(Item::Pre(line));
        } else {
            return None;
        }
        rest = &rest[end..];
    }
    Some(items)
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// Defined in part `owner`, declared elsewhere.
    Root(usize),
    /// Copied into each part that uses it.
    Helper,
    /// A helper whose address is taken: defined in part 0.
    Pinned,
    Global,
}

/// The parts of `ir`, at most `n`, or None (compile it whole).
pub fn split(ir: &str, n: usize) -> Option<Vec<String>> {
    if n < 2 {
        return None;
    }
    let items = parse(ir)?;
    let mut idx: HashMap<&str, usize> = HashMap::new();
    for (i, it) in items.iter().enumerate() {
        match it {
            Item::Global { name, .. } | Item::Func { name, .. } => {
                if idx.insert(canon(name), i).is_some() {
                    return None;
                }
            }
            Item::Pre(_) => {}
        }
    }
    let text = |i: usize| match &items[i] {
        Item::Global { line, .. } => *line,
        Item::Func { body, .. } => *body,
        Item::Pre(_) => "",
    };
    // Helpers whose address is taken anywhere, and each function's callers.
    let mut taken = vec![false; items.len()];
    let mut callers: Vec<Vec<usize>> = vec![Vec::new(); items.len()];
    for i in 0..items.len() {
        for (r, call) in refs(text(i)) {
            let Some(&j) = idx.get(canon(r)) else { continue };
            if !call {
                taken[j] = true;
            } else if !callers[j].contains(&i) {
                callers[j].push(i);
            }
        }
    }
    // Roots, cut into contiguous runs.
    let is_root = |i: usize| match &items[i] {
        Item::Func { header, lines, internal, .. } => {
            !internal || header.contains(" preserve_nonecc ") || *lines > HUGE || (*lines > BIG && callers[i].len() != 1)
        }
        _ => false,
    };
    // What each root pulls into its part: the helpers it reaches (copied
    // into every part that reaches them) and the globals. Part 0 also
    // holds every global and pinned helper, and what they reach.
    let size = |i: usize| match &items[i] {
        Item::Func { lines, .. } => *lines,
        Item::Global { .. } => 1,
        Item::Pre(_) => 0,
    };
    let pulled = |i: usize| matches!(items[i], Item::Global { .. }) || (matches!(items[i], Item::Func { .. }) && !is_root(i) && !taken[i]);
    let reach = |start: &mut Vec<usize>| {
        let mut seen: Vec<usize> = start.clone();
        while let Some(i) = start.pop() {
            for (r, _) in refs(text(i)) {
                if let Some(&j) = idx.get(canon(r)) {
                    if pulled(j) && !seen.contains(&j) {
                        seen.push(j);
                        start.push(j);
                    }
                }
            }
        }
        seen
    };
    let roots: Vec<usize> = (0..items.len()).filter(|&i| is_root(i)).collect();
    let base = reach(&mut (0..items.len()).filter(|&i| matches!(items[i], Item::Global { .. }) || (matches!(items[i], Item::Func { .. }) && !is_root(i) && taken[i])).collect());
    let pulls: Vec<Vec<usize>> = roots.iter().map(|&r| reach(&mut vec![r])).collect();
    // Contiguous runs, each cut before it would pass `cap`: the parts and
    // the largest. The smallest cap that gives at most n parts wins.
    let cut = |cap: usize| -> (Vec<usize>, usize, usize) {
        let mut stamp = vec![0usize; items.len()];
        for &b in &base {
            stamp[b] = 1;
        }
        let (mut part, mut acc, mut most) = (0, base.iter().map(|&b| size(b)).sum::<usize>(), 0);
        let mut of = Vec::with_capacity(roots.len());
        for p in &pulls {
            let cost: usize = p.iter().filter(|&&j| stamp[j] != part + 1).map(|&j| size(j)).sum();
            if acc > 0 && acc + cost > cap {
                most = most.max(acc);
                part += 1;
                acc = 0;
            }
            let cost: usize = p.iter().filter(|&&j| stamp[j] != part + 1).map(|&j| size(j)).sum();
            for &j in p {
                stamp[j] = part + 1;
            }
            acc += cost;
            of.push(part);
        }
        (of, part + 1, most.max(acc))
    };
    let (mut lo, mut hi) = (1, cut(usize::MAX).2);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if cut(mid).1 <= n {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    let (of, nparts, _) = cut(lo);
    // A function whose block addresses are taken is defined where the
    // table holding them is, in part 0 (`blockaddress` names a definition).
    let mut blocks = vec![false; items.len()];
    for i in 0..items.len() {
        for (at, _) in text(i).match_indices("blockaddress(") {
            if let Some(&j) = text(i)[at + 13..].starts_with('@').then(|| name_at(text(i), at + 13)).flatten().and_then(|n| idx.get(canon(n))) {
                blocks[j] = true;
            }
        }
    }
    let mut kind = vec![Kind::Global; items.len()];
    for (k, &r) in roots.iter().enumerate() {
        kind[r] = Kind::Root(if blocks[r] { 0 } else { of[k] });
    }
    for (i, it) in items.iter().enumerate() {
        if let (Item::Func { .. }, false) = (it, is_root(i)) {
            kind[i] = if taken[i] || blocks[i] { Kind::Pinned } else { Kind::Helper };
        }
    }
    if nparts < 2 {
        return None;
    }
    let mut out = Vec::with_capacity(nparts);
    for p in 0..nparts {
        // What part p defines (true) or declares (false).
        let mut need: Vec<Option<bool>> = vec![None; items.len()];
        let mut work = Vec::new();
        for i in 0..items.len() {
            let own = match kind[i] {
                Kind::Root(o) => o == p,
                Kind::Pinned => p == 0,
                Kind::Global => p == 0 && !matches!(items[i], Item::Pre(_)),
                Kind::Helper => false,
            };
            if own {
                need[i] = Some(true);
                work.push(i);
            }
        }
        while let Some(i) = work.pop() {
            for (r, _) in refs(text(i)) {
                let Some(&j) = idx.get(canon(r)) else { continue };
                if need[j] == Some(true) {
                    continue;
                }
                match kind[j] {
                    Kind::Helper | Kind::Global => {
                        // (an appending global, or one holding block
                        // addresses, lives in part 0 alone)
                        if let Item::Global { at: None, .. } = items[j] {
                            return None;
                        }
                        if matches!(items[j], Item::Global { .. }) && text(j).contains("blockaddress(") {
                            return None;
                        }
                        need[j] = Some(true);
                        work.push(j);
                    }
                    Kind::Root(_) | Kind::Pinned => need[j] = Some(false),
                }
            }
        }
        let mut s = String::with_capacity(ir.len() / nparts * 2);
        for (i, it) in items.iter().enumerate() {
            match (it, need[i]) {
                (Item::Pre(l), _) => s.push_str(l),
                (_, None) => {}
                // Exported: as is in part 0, a copy elsewhere.
                (Item::Global { line, at: Some((off, 0)), .. }, Some(_)) => {
                    s.push_str(&line[..*off]);
                    s.push_str(if p == 0 { "" } else { "available_externally " });
                    s.push_str(&line[*off..]);
                }
                (Item::Global { line, at: Some((off, len)), .. }, Some(_)) => {
                    s.push_str(&line[..*off]);
                    s.push_str(if p == 0 { "hidden" } else { "available_externally hidden" });
                    s.push_str(&line[off + len..]);
                }
                (Item::Global { line, at: None, .. }, Some(_)) => s.push_str(line),
                (Item::Func { header, body, internal, .. }, Some(true)) => {
                    if kind[i] == Kind::Helper || !internal {
                        s.push_str(header);
                    } else {
                        s.push_str("define hidden ");
                        s.push_str(&header["define internal ".len()..]);
                    }
                    s.push_str(body);
                }
                (Item::Func { header, name, internal, .. }, Some(false)) => {
                    s.push_str(&declaration(header, name, *internal)?);
                    s.push('\n');
                }
            }
        }
        out.push(s);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarations() {
        let h = "define internal preserve_nonecc void @\"vm.M.op.X\"(ptr %p0, %\"a b, c\" %p1, { i64, i1 } %p2) nounwind section \".text.hot\" {\n";
        assert_eq!(declaration(h, "@\"vm.M.op.X\"", true).unwrap(), "declare hidden preserve_nonecc void @\"vm.M.op.X\"(ptr, %\"a b, c\", { i64, i1 }) nounwind");
        assert_eq!(declaration("define i32 @main() {\n", "@main", false).unwrap(), "declare i32 @main()");
    }

    #[test]
    fn splits() {
        let mut ir = String::from("%T = type { i64 }\n@g = internal global i64 0, align 8\n@.s = private unnamed_addr constant [1 x i8] c\"a\"\ndeclare void @ext(ptr)\n");
        ir.push_str("define internal i64 @h(i64 %x) nounwind {\nentry:\n  %v = load i64, ptr @g\n  ret i64 %v\n}\n");
        ir.push_str("define internal void @t() {\nentry:\n  ret void\n}\n");
        for k in 0..4 {
            ir.push_str(&format!("define internal preserve_nonecc void @r{k}(ptr %p) {{\nentry:\n  %a = call i64 @h(i64 1)\n  call void @ext(ptr @.s)\n  call void @ext(ptr @t)\n  ret void\n}}\n"));
        }
        let parts = split(&ir, 2).unwrap();
        assert_eq!(parts.len(), 2);
        assert!(parts[0].contains("@g = hidden global") && parts[1].contains("@g = available_externally hidden global"));
        assert!(parts[0].contains("define hidden void @t()") && parts[1].contains("declare hidden void @t()"));
        assert!(parts[1].contains("define internal i64 @h("));
        assert!(parts[0].contains("define hidden preserve_nonecc void @r0") && parts[1].contains("define hidden preserve_nonecc void @r3"));
        assert!(!parts[1].contains("@r0"));
    }
}
