//! Golden tests: compile and run every program under tests/run and compare
//! its output with the `// expect:` lines; every program under tests/fail
//! must fail to compile with the `// error:` message.
//!
//! Directives (in comments):
//!   // expect: <line>          expected stdout line, in order
//!   // expect-exit: <code>     expected exit status (default 0)
//!   // expect-stderr: <text>   substring of stderr
//!   // args: <flags>           extra compiler flags (e.g. --release)
//!   // matrix: dispatch        run once per dispatch mode
//!   // error: <text>           (tests/fail) substring of the compile error

use std::path::{Path, PathBuf};
use std::process::Command;

fn files(dir: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join(dir))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "lm"))
        .collect();
    v.sort();
    v
}

fn directives(text: &str, key: &str) -> Vec<String> {
    let pat = format!("// {}:", key);
    text.lines()
        .filter_map(|l| l.trim_start().strip_prefix(&pat).map(|r| r.strip_prefix(' ').unwrap_or(r).to_string()))
        .collect()
}

fn loimi() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_loimi"));
    c.env("LOIMI_STD", env!("CARGO_MANIFEST_DIR"));
    c
}

#[test]
fn run_programs() {
    let mut failures = Vec::new();
    for f in files("tests/run") {
        let text = std::fs::read_to_string(&f).unwrap();
        let expect: Vec<String> = directives(&text, "expect");
        let exit: i32 = directives(&text, "expect-exit").first().map(|s| s.trim().parse().unwrap()).unwrap_or(0);
        let stderr_want = directives(&text, "expect-stderr");
        let extra: Vec<String> = directives(&text, "args").iter().flat_map(|s| s.split_whitespace().map(String::from).collect::<Vec<_>>()).collect();
        let modes: Vec<Vec<String>> = if directives(&text, "matrix").iter().any(|m| m.trim() == "dispatch") {
            ["tailcall", "goto", "switch"].iter().map(|d| vec![format!("--dispatch={}", d)]).collect()
        } else {
            vec![vec![]]
        };
        for mode in modes {
            let out = loimi().arg("run").arg(&f).args(&extra).args(&mode).output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let got: Vec<&str> = stdout.lines().collect();
            let name = format!("{} {}", f.file_name().unwrap().to_string_lossy(), mode.join(" "));
            if got != expect.iter().map(|s| s.as_str()).collect::<Vec<_>>() {
                failures.push(format!("{}: stdout mismatch\n--- expected\n{}\n--- got\n{}\n--- stderr\n{}", name, expect.join("\n"), stdout, stderr));
                continue;
            }
            if out.status.code() != Some(exit) {
                failures.push(format!("{}: exit {:?}, expected {}\n{}", name, out.status.code(), exit, stderr));
                continue;
            }
            for s in &stderr_want {
                if !stderr.contains(s.as_str()) {
                    failures.push(format!("{}: stderr lacks `{}`:\n{}", name, s, stderr));
                }
            }
        }
    }
    if !failures.is_empty() {
        panic!("{} failure(s):\n\n{}", failures.len(), failures.join("\n\n"));
    }
}

#[test]
fn compile_errors() {
    let mut failures = Vec::new();
    for f in files("tests/fail") {
        let text = std::fs::read_to_string(&f).unwrap();
        let want = directives(&text, "error");
        let out = loimi().arg("build").arg(&f).arg("--emit-llvm").arg("-o").arg("-").output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        if out.status.success() {
            failures.push(format!("{}: compiled, but should fail", name));
            continue;
        }
        for w in &want {
            if !stderr.contains(w.as_str()) {
                failures.push(format!("{}: error lacks `{}`:\n{}", name, w, stderr));
            }
        }
    }
    if !failures.is_empty() {
        panic!("{} failure(s):\n\n{}", failures.len(), failures.join("\n\n"));
    }
}
