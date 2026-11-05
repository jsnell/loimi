//! Hot-handler ordering (docs/lowering.md §12, "Handler order"): lay out
//! a vm's handlers by a dispatch profile, the hottest first, packed
//! together in a section of their own.
//!
//! A profile is a text file named by `#[profile("file")]` on the vm (the
//! path relative to the file declaring it) or by `--vm-profile=FILE`. Two
//! forms:
//!
//! - the stderr of a `--vm-stats` build: every `== vm stats: NAME ==`
//!   section of the vm counts, each scaled to fractions of its own
//!   dispatches, so that runs concatenated into one file weigh the same
//!   however long each ran; other lines are ignored;
//! - with no such section, a list of op names, one per line (`//`
//!   comments), hottest first.

/// The order to lay out a vm's handlers in: `order` holds every op index
/// once; the first `hot` of them are the ones the profile saw, hottest
/// first, and go to the hot section.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub order: Vec<usize>,
    pub hot: usize,
}

/// Each op's weight in `text` for the vm named `vm` (ops by name, in
/// declaration order), and the names it has that are not ops of the vm.
pub fn weights(text: &str, vm: &str, ops: &[&str]) -> (Vec<f64>, Vec<String>) {
    let index = |n: &str| ops.iter().position(|o| *o == n);
    let mut w = vec![0.0; ops.len()];
    let mut unknown = Vec::new();
    let head = format!("== vm stats: {} ==", vm);
    if text.lines().any(|l| l.trim_start().starts_with("== vm stats: ")) {
        let mut lines = text.lines().peekable();
        while let Some(l) = lines.next() {
            if l.trim() != head {
                continue;
            }
            // Rows `count  pct%  name` up to the first other line.
            let mut rows = Vec::new();
            while let Some(r) = lines.peek() {
                let f: Vec<&str> = r.split_whitespace().collect();
                if f.len() == 3 && f[1].ends_with('%') {
                    if let Ok(c) = f[0].parse::<u64>() {
                        rows.push((c, f[2]));
                        lines.next();
                        continue;
                    }
                }
                if f == ["count", "%", "op"] {
                    lines.next();
                    continue;
                }
                break;
            }
            let total: u64 = rows.iter().map(|(c, _)| c).sum();
            for (c, n) in rows {
                match index(n) {
                    Some(k) => w[k] += c as f64 / total.max(1) as f64,
                    None => unknown.push(n.to_string()),
                }
            }
        }
    } else {
        let names: Vec<&str> = text
            .lines()
            .map(|l| l.split("//").next().unwrap_or("").trim())
            .filter(|l| !l.is_empty())
            .collect();
        for (i, n) in names.iter().enumerate() {
            match index(n) {
                Some(k) if w[k] == 0.0 => w[k] = (names.len() - i) as f64,
                Some(_) => {}
                None => unknown.push(n.to_string()),
            }
        }
    }
    unknown.sort();
    unknown.dedup();
    (w, unknown)
}

/// The layout for these weights: ops with weight first, heaviest first
/// (ties in declaration order), then the rest in declaration order.
pub fn layout(w: &[f64]) -> Layout {
    let mut order: Vec<usize> = (0..w.len()).filter(|&k| w[k] > 0.0).collect();
    order.sort_by(|&a, &b| w[b].partial_cmp(&w[a]).unwrap().then(a.cmp(&b)));
    let hot = order.len();
    order.extend((0..w.len()).filter(|&k| w[k] <= 0.0));
    Layout { order, hot }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_sections_add_up_as_fractions() {
        let text = "noise\n\n== vm stats: V ==\n         count       %  op\n            30   75.0%  B\n            10   25.0%  A\n\ncold blocks entered:\n\n== vm stats: W ==\n             5  100.0%  A\n\n== vm stats: V ==\n             1  100.0%  A\n";
        let (w, unknown) = weights(text, "V", &["A", "B", "C"]);
        assert_eq!(w, vec![1.25, 0.75, 0.0]);
        assert!(unknown.is_empty());
        let l = layout(&w);
        assert_eq!(l.order, vec![0, 1, 2]);
        assert_eq!(l.hot, 2);
    }

    #[test]
    fn a_list_is_hottest_first() {
        let (w, unknown) = weights("// hot ops\nC\nA // the loop\n\nZ\n", "V", &["A", "B", "C"]);
        assert_eq!(unknown, vec!["Z".to_string()]);
        let l = layout(&w);
        assert_eq!(l.order, vec![2, 0, 1]);
        assert_eq!(l.hot, 2);
    }
}
