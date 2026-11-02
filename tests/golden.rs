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

/// Per-process virtual memory cap (KiB) for everything the tests spawn, so a
/// runaway test program fails instead of exhausting the machine.
const MEM_LIMIT_KB: u64 = 2 * 1024 * 1024;

/// `prog`, run under the memory cap.
fn limited(prog: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(format!("ulimit -v {} && exec \"$0\" \"$@\"", MEM_LIMIT_KB)).arg(prog);
    c
}

fn loimi() -> Command {
    let mut c = limited(env!("CARGO_BIN_EXE_loimi"));
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

/// The Mini interpreter (examples/mini): run every script in
/// examples/mini/tests normally and with GC stress (collect at every
/// safepoint), comparing with `-- expect:` lines.
#[test]
fn mini_scripts() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let exe = std::env::temp_dir().join(format!("loimi-mini-{}", std::process::id()));
    let out = loimi().arg("build").arg(root.join("examples/mini/main.lm")).arg("-o").arg(&exe).output().unwrap();
    assert!(out.status.success(), "building mini failed:\n{}", String::from_utf8_lossy(&out.stderr));
    let mut failures = Vec::new();
    let mut scripts: Vec<PathBuf> = std::fs::read_dir(root.join("examples/mini/tests"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "mini"))
        .collect();
    scripts.sort();
    for f in scripts {
        let text = std::fs::read_to_string(&f).unwrap();
        let lua_directives = |key: &str| -> Vec<String> {
            let pat = format!("-- {}:", key);
            text.lines()
                .filter_map(|l| l.trim_start().strip_prefix(&pat).map(|r| r.strip_prefix(' ').unwrap_or(r).to_string()))
                .collect()
        };
        let expect = lua_directives("expect");
        let exit: i32 = lua_directives("expect-exit").first().map(|s| s.trim().parse().unwrap()).unwrap_or(0);
        let stderr_want = lua_directives("expect-stderr");
        for stress in [false, true] {
            let mut cmd = limited(&exe);
            cmd.arg(&f);
            if stress {
                cmd.arg("stress");
            }
            let out = cmd.output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let name = format!("{}{}", f.file_name().unwrap().to_string_lossy(), if stress { " (gc stress)" } else { "" });
            let got: Vec<&str> = stdout.lines().collect();
            if got != expect.iter().map(|s| s.as_str()).collect::<Vec<_>>() {
                failures.push(format!("{}: stdout mismatch\n--- expected\n{}\n--- got\n{}\n--- stderr\n{}", name, expect.join("\n"), stdout, stderr));
                continue;
            }
            if out.status.code() != Some(exit) {
                failures.push(format!("{}: exit {:?}, expected {}\n{}", name, out.status.code(), exit, stderr));
            }
            for s in &stderr_want {
                if !stderr.contains(s.as_str()) {
                    failures.push(format!("{}: stderr lacks `{}`:\n{}", name, s, stderr));
                }
            }
        }
    }
    let _ = std::fs::remove_file(&exe);
    if !failures.is_empty() {
        panic!("{} failure(s):\n\n{}", failures.len(), failures.join("\n\n"));
    }
}

/// The clean-room conformance tests (examples/mini/tests/conformance):
/// NAME.lua must print exactly NAME.expected (output of real Lua 5.4).
/// err_*.lua must also exit with a nonzero status; their .expected holds
/// the output before the error. Run normally and with GC stress.
#[test]
fn mini_conformance() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let exe = std::env::temp_dir().join(format!("loimi-mini-conf-{}", std::process::id()));
    let out = loimi().arg("build").arg(root.join("examples/mini/main.lm")).arg("-o").arg(&exe).output().unwrap();
    assert!(out.status.success(), "building mini failed:\n{}", String::from_utf8_lossy(&out.stderr));
    let dir = root.join("examples/mini/tests/conformance");
    let mut scripts: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "lua"))
        .collect();
    scripts.sort();
    let mut failures = Vec::new();
    let mut ran = 0;
    for f in scripts {
        let stem = f.file_stem().unwrap().to_string_lossy().to_string();
        ran += 1;
        let expected = std::fs::read_to_string(f.with_extension("expected")).unwrap();
        let should_fail = stem.starts_with("err_");
        for stress in [false, true] {
            let mut cmd = limited(&exe);
            cmd.arg(&f);
            if stress {
                cmd.arg("stress");
            }
            let out = cmd.output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let name = format!("{}{}", stem, if stress { " (gc stress)" } else { "" });
            if stdout != expected {
                failures.push(format!("{}: stdout mismatch\n--- expected\n{}--- got\n{}--- stderr\n{}", name, expected, stdout, stderr));
                continue;
            }
            if should_fail == out.status.success() {
                failures.push(format!("{}: exit {:?}, expected {}\n{}", name, out.status.code(), if should_fail { "nonzero" } else { "0" }, stderr));
            }
        }
    }
    let _ = std::fs::remove_file(&exe);
    assert!(ran > 0);
    if !failures.is_empty() {
        panic!("{} failure(s):\n\n{}", failures.len(), failures.join("\n\n"));
    }
}

/// The Scheme interpreter (examples/scheme): run every program in
/// examples/scheme/tests normally and with GC stress, comparing with
/// `; expect:` lines (and `; expect-exit:` / `; expect-stderr:`).
#[test]
fn scheme_programs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let exe = std::env::temp_dir().join(format!("loimi-scheme-{}", std::process::id()));
    let out = loimi().arg("build").arg(root.join("examples/scheme/main.lm")).arg("--release").arg("-o").arg(&exe).output().unwrap();
    assert!(out.status.success(), "building scheme failed:\n{}", String::from_utf8_lossy(&out.stderr));
    let mut failures = Vec::new();
    let mut programs: Vec<PathBuf> = std::fs::read_dir(root.join("examples/scheme/tests"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "scm"))
        .collect();
    programs.sort();
    for f in programs {
        let text = std::fs::read_to_string(&f).unwrap();
        let scm_directives = |key: &str| -> Vec<String> {
            let pat = format!("; {}:", key);
            text.lines()
                .filter_map(|l| l.trim_start().strip_prefix(&pat).map(|r| r.strip_prefix(' ').unwrap_or(r).to_string()))
                .collect()
        };
        let expect = scm_directives("expect");
        let exit: i32 = scm_directives("expect-exit").first().map(|s| s.trim().parse().unwrap()).unwrap_or(0);
        let stderr_want = scm_directives("expect-stderr");
        for stress in [false, true] {
            let mut cmd = Command::new(&exe);
            cmd.arg(&f);
            if stress {
                cmd.arg("stress");
            }
            let out = cmd.output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let name = format!("{}{}", f.file_name().unwrap().to_string_lossy(), if stress { " (gc stress)" } else { "" });
            let got: Vec<&str> = stdout.lines().collect();
            if got != expect.iter().map(|s| s.as_str()).collect::<Vec<_>>() {
                failures.push(format!("{}: stdout mismatch\n--- expected\n{}\n--- got\n{}\n--- stderr\n{}", name, expect.join("\n"), stdout, stderr));
                continue;
            }
            if out.status.code() != Some(exit) {
                failures.push(format!("{}: exit {:?}, expected {}\n{}", name, out.status.code(), exit, stderr));
            }
            for s in &stderr_want {
                if !stderr.contains(s.as_str()) {
                    failures.push(format!("{}: stderr lacks `{}`:\n{}", name, s, stderr));
                }
            }
        }
    }
    let _ = std::fs::remove_file(&exe);
    if !failures.is_empty() {
        panic!("{} failure(s):\n\n{}", failures.len(), failures.join("\n\n"));
    }
}
