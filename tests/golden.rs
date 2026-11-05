//! Golden tests: compile and run every program under tests/run and compare
//! its output with the `// expect:` lines; every program under tests/fail
//! must fail to compile with the `// error:` message.
//!
//! Directives (in comments):
//!   // expect: <line>          expected stdout line, in order
//!   // expect-exit: <code>     expected exit status (default 0)
//!   // expect-stderr: <text>   substring of stderr
//!   // expect-no-stderr: <text> text that must not appear in stderr
//!   // args: <flags>           extra compiler flags (e.g. --release; also
//!                              for tests/fail)
//!   // matrix: dispatch        run once per dispatch mode
//!   // matrix: jit             run without the JIT, and (on x86-64 Linux)
//!                              built with --jit-stencils and run with
//!                              LOIMI_JIT=1 LOIMI_JIT_HOT=1: same output
//!   // matrix: check-roots     run as is and built with --gc-check-roots
//!   // env: K=V ...            environment of the program (e.g. LOIMI_JIT=1)
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

/// `prog`, run under the memory cap, without core dumps (qemu writes its own).
fn limited(prog: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(format!("ulimit -v {} && ulimit -c 0 && exec \"$0\" \"$@\"", MEM_LIMIT_KB)).arg(prog);
    c
}

/// A built program `exe`, run under the memory cap, and under
/// `LOIMI_RUNNER` (an emulator) if that is set.
fn program(exe: impl AsRef<std::ffi::OsStr>) -> Command {
    program_under(&std::env::var("LOIMI_RUNNER").unwrap_or_default(), exe)
}

/// `exe` under the memory cap and the command `runner` (none if empty).
fn program_under(runner: &str, exe: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut words = runner.split_whitespace();
    match words.next() {
        Some(r) => {
            let mut c = limited(r);
            c.args(words).arg(exe);
            c
        }
        None => limited(exe),
    }
}

/// Whether the suite is cross-compiled: `LOIMI_TARGET` is set (loimi
/// reads it as `--target`, with `LOIMI_SYSROOT` as `--sysroot`) and
/// programs run under `LOIMI_RUNNER`. For example, for AArch64 under
/// qemu (after bench/cross/install.sh, with absolute paths):
///
///     LOIMI_TARGET=aarch64-linux-gnu LOIMI_SYSROOT=$S \
///     LOIMI_RUNNER="bench/cross/install/bin/qemu-aarch64 -L $S" cargo test --release
///
/// Tests of the x86-64 JIT and of ASan skip.
fn cross() -> bool {
    std::env::var_os("LOIMI_TARGET").is_some_and(|t| !t.is_empty())
}

/// The C compiler `clang` (for hosts of Loimi libraries), with the
/// target, sysroot and linker of a cross-compiled suite.
fn cc(clang: &str) -> Command {
    let mut c = limited(clang);
    if let Ok(t) = std::env::var("LOIMI_TARGET") {
        if !t.is_empty() {
            c.arg(format!("--target={}", t)).arg("-fuse-ld=lld");
            if let Ok(s) = std::env::var("LOIMI_SYSROOT") {
                c.arg(format!("--sysroot={}", s));
            }
        }
    }
    c
}

/// How the interpreters' test programs are run: normally, with GC stress
/// (collect at every safepoint), and with a generational heap under
/// stress that verifies the write barriers at every collection (the
/// interpreters' second argument).
const GC_MODES: [&str; 3] = ["", "stress", "verify"];

fn gc_mode_name(mode: &str) -> &'static str {
    match mode {
        "stress" => " (gc stress)",
        "verify" => " (gc verify)",
        _ => "",
    }
}

fn loimi() -> Command {
    let mut c = limited(env!("CARGO_BIN_EXE_loimi"));
    c.env("LOIMI_STD", env!("CARGO_MANIFEST_DIR"));
    c
}

#[test]
fn run_programs() {
    run_programs_with(&[], |_, _| true);
}

/// The tests/run programs that `select(file name, text)` picks, with
/// `xenv` in loimi's environment (`LOIMI_TARGET`, `LOIMI_RUNNER`, ...).
fn run_programs_with(xenv: &[(String, String)], select: impl Fn(&str, &str) -> bool) {
    let cross = cross() || xenv.iter().any(|(k, _)| k == "LOIMI_TARGET");
    let mut failures = Vec::new();
    for f in files("tests/run") {
        let text = std::fs::read_to_string(&f).unwrap();
        if !select(&f.file_name().unwrap().to_string_lossy(), &text) {
            continue;
        }
        let expect: Vec<String> = directives(&text, "expect");
        let exit: i32 = directives(&text, "expect-exit").first().map(|s| s.trim().parse().unwrap()).unwrap_or(0);
        let stderr_want = directives(&text, "expect-stderr");
        let stderr_not = directives(&text, "expect-no-stderr");
        let extra: Vec<String> = directives(&text, "args").iter().flat_map(|s| s.split_whitespace().map(String::from).collect::<Vec<_>>()).collect();
        let env: Vec<(String, String)> = directives(&text, "env")
            .iter()
            .flat_map(|s| s.split_whitespace().filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect::<Vec<_>>())
            .collect();
        if cross && extra.iter().any(|a| a == "--jit-stencils") {
            continue;
        }
        let matrix = directives(&text, "matrix");
        let modes: Vec<(Vec<String>, Vec<(String, String)>)> = if matrix.iter().any(|m| m.trim() == "dispatch") {
            ["tailcall", "goto", "switch"].iter().map(|d| (vec![format!("--dispatch={}", d)], vec![])).collect()
        } else if matrix.iter().any(|m| m.trim() == "check-roots") {
            // `--gc-check-roots`: every reference the collector marks is validated.
            vec![(vec![], vec![]), (vec!["--gc-check-roots".to_string()], vec![])]
        } else if matrix.iter().any(|m| m.trim() == "jit") {
            let mut v = vec![(vec![], vec![])];
            if cfg!(all(target_arch = "x86_64", target_os = "linux")) && !cross {
                let env = [("LOIMI_JIT", "1"), ("LOIMI_JIT_HOT", "1")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
                v.push((vec!["--jit-stencils".to_string()], env));
            }
            v
        } else {
            vec![(vec![], vec![])]
        };
        for (mode, menv) in modes {
            let out = loimi()
                .arg("run")
                .arg(&f)
                .args(&extra)
                .args(&mode)
                .envs(xenv.iter().chain(env.iter()).chain(menv.iter()).map(|(k, v)| (k, v)))
                .output()
                .unwrap();
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
            for s in &stderr_not {
                if stderr.contains(s.as_str()) {
                    failures.push(format!("{}: stderr has `{}`:\n{}", name, s, stderr));
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
        let extra: Vec<String> = directives(&text, "args").iter().flat_map(|s| s.split_whitespace().map(String::from).collect::<Vec<_>>()).collect();
        if cross() && extra.iter().any(|a| a == "--jit-stencils") {
            continue;
        }
        let out = loimi().arg("build").arg(&f).args(&extra).arg("--emit-llvm").arg("-o").arg("-").output().unwrap();
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

/// `loimi ir-diff`: tests/irdiff/b is tests/irdiff/a with lines moved
/// and a table and a function renamed (the same code: exit 0); c changes
/// one constant (exit 1, naming the function and the line).
#[test]
fn ir_diff() {
    if cross() {
        return;
    }
    let run = |b: &str, extra: &[&str]| {
        let out = loimi().arg("ir-diff").arg("tests/irdiff/a/main.lm").arg(format!("tests/irdiff/{}/main.lm", b)).args(extra).output().unwrap();
        (out.status.code(), String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr))
    };
    for flags in [&["--release", "--asm"][..], &[]] {
        let (code, text) = run("b", flags);
        assert_eq!(code, Some(0), "ir-diff a b {:?}:\n{}", flags, text);
        assert!(text.contains("result: same code"), "{}", text);
    }
    let (code, text) = run("c", &["--release", "--asm"]);
    assert_eq!(code, Some(1), "ir-diff a c:\n{}", text);
    assert!(text.contains("differs: main ") && text.contains("+  %v15 = add nsw i64 %v14, -9") && text.contains("+\taddq\t$-9"), "{}", text);
    // `.ll` inputs (from `build --emit-llvm`) compare the same way.
    let dir = std::env::temp_dir().join(format!("loimi-irdiff-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for x in ["a", "b"] {
        let st = loimi().args(["build", &format!("tests/irdiff/{}/main.lm", x), "--release", "--emit-llvm", "-o"]).arg(dir.join(format!("{}.ll", x))).status().unwrap();
        assert!(st.success());
    }
    let out = loimi().arg("ir-diff").arg(dir.join("a.ll")).arg(dir.join("b.ll")).arg("--release").output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stdout));
}

/// `loimi test` (std.test): run each tests/test/NAME.lm from its
/// directory with the `// args:` as filters; stdout must equal
/// NAME.expected and the exit status `// expect-exit:` (default 0).
#[test]
fn std_test() {
    std_test_with(&[]);
}

/// `std_test` with `xenv` in loimi's environment.
fn std_test_with(xenv: &[(String, String)]) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test");
    let mut failures = Vec::new();
    for f in files("tests/test") {
        let text = std::fs::read_to_string(&f).unwrap();
        let args: Vec<String> = directives(&text, "args").iter().flat_map(|s| s.split_whitespace().map(String::from).collect::<Vec<_>>()).collect();
        let exit: i32 = directives(&text, "expect-exit").first().map(|s| s.trim().parse().unwrap()).unwrap_or(0);
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let out = loimi().envs(xenv.iter().map(|(k, v)| (k, v))).current_dir(&dir).arg("test").arg(&name).args(&args).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let want = std::fs::read_to_string(f.with_extension("expected")).unwrap();
        if stdout != want {
            failures.push(format!("{}: stdout mismatch\n--- expected\n{}\n--- got\n{}\n--- stderr\n{}", name, want, stdout, String::from_utf8_lossy(&out.stderr)));
        } else if out.status.code() != Some(exit) {
            failures.push(format!("{}: exit {:?}, expected {}", name, out.status.code(), exit));
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
    mini_scripts_with(&[], &[], |_| true);
}

/// The copy-and-patch JIT prototype (`--jit-stencils`, x86-64 Linux):
/// every script and conformance test with the JIT on and its hot
/// threshold at 1, so code is compiled at its first call or backward
/// jump (and again after each rewrite of its words), then again with
/// recompiles off (`LOIMI_JIT_RECOMPILE=1`): rewrites of compiled code
/// then make out-of-line copies and retarget instructions instead.
#[test]
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
fn mini_jit_stencils() {
    if cross() {
        return;
    }
    let env = [("LOIMI_JIT", "1"), ("LOIMI_JIT_HOT", "1")];
    mini_scripts_with(&["--jit-stencils"], &env, |_| true);
    mini_conformance_with(&["--jit-stencils"], &env, |_| true);
    let env = [("LOIMI_JIT", "1"), ("LOIMI_JIT_HOT", "1"), ("LOIMI_JIT_RECOMPILE", "1")];
    mini_scripts_with(&["--jit-stencils"], &env, |_| true);
    mini_conformance_with(&["--jit-stencils"], &env, |_| true);
}

/// The Mini debugger (`--cfg debug`, examples/mini/debugger.lm): each
/// examples/mini/tests/debug/NAME.mini run as `mini --debug NAME.mini`
/// with NAME.cmds on stdin must print the transcript NAME.out, in every
/// dispatch mode. The debug build also runs the scripts that re-enter
/// the vm (sort comparators, coroutines) without the debugger.
#[test]
fn mini_debugger() {
    if cross() {
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("examples/mini/tests/debug");
    let mut tests: Vec<PathBuf> =
        std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "mini")).collect();
    tests.sort();
    assert!(!tests.is_empty());
    let mut failures = Vec::new();
    for mode in ["tailcall", "goto", "switch"] {
        let exe = std::env::temp_dir().join(format!("loimi-mini-debug-{}-{}", std::process::id(), mode));
        let out = loimi()
            .args(["build", "-W", "error", "--cfg", "debug", &format!("--dispatch={mode}")])
            .arg(root.join("examples/mini/main.lm"))
            .arg("-o")
            .arg(&exe)
            .output()
            .unwrap();
        assert!(out.status.success(), "building mini --cfg debug ({mode}) failed:\n{}", String::from_utf8_lossy(&out.stderr));
        for t in &tests {
            let cmds = std::fs::File::open(t.with_extension("cmds")).unwrap();
            let want = std::fs::read_to_string(t.with_extension("out")).unwrap();
            let out = limited(&exe).arg("--debug").arg(t).stdin(cmds).output().unwrap();
            let got = String::from_utf8_lossy(&out.stdout);
            if got != want {
                failures.push(format!(
                    "{} ({mode}): transcript mismatch\n--- expected\n{want}\n--- got\n{got}\n--- stderr\n{}",
                    t.display(),
                    String::from_utf8_lossy(&out.stderr)
                ));
            }
        }
        let _ = std::fs::remove_file(&exe);
    }
    if !failures.is_empty() {
        panic!("{} failure(s):\n\n{}", failures.len(), failures.join("\n\n"));
    }
    mini_scripts_with(&["--cfg", "debug"], &[], |name| ["sort", "coroutine", "functions", "closures"].iter().any(|p| name.starts_with(p)));
}

/// Re-entry (table.sort's comparator runs as a nested run of the
/// interpreter) and coroutines (switches inside the run, and resumes
/// from natives as nested runs) in the goto and switch dispatch modes: the sort and
/// coroutine scripts with a Mini built in each, and the coroutine
/// conformance test (each in every GC mode).
#[test]
fn mini_reentry_dispatch_modes() {
    for mode in ["--dispatch=goto", "--dispatch=switch"] {
        mini_scripts_with(&[mode], &[], |name| ["sort", "err_sort", "coroutine", "err_coroutine"].iter().any(|p| name.starts_with(p)));
        mini_conformance_with(&[mode], &[], |name| name.starts_with("coroutine"));
    }
}

/// AArch64 Linux under qemu user-mode, when the cross toolchain is
/// installed (bench/cross/install.sh; otherwise skipped with a message):
/// a fast subset, cross-compiled with `--target=aarch64-linux-gnu`. The
/// tests/run programs that run in every dispatch mode, the GC ones, the
/// wide (multi-word arithmetic) ones, the vector (`simd_`) ones and those
/// using `std.rt`; the `std.test` tests (`catch_panic`'s setjmp);
/// Mini in every dispatch mode on scripts that collect, re-enter and
/// throw, with GC stress and barrier verification; and Stak's suite
/// (examples/stack/check.sh: every dispatch mode, GC stress). The whole
/// suite runs cross-compiled with `LOIMI_TARGET` (see `cross`).
#[test]
fn aarch64_qemu() {
    if cross() {
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let qemu = root.join("bench/cross/install/bin/qemu-aarch64");
    let sysroot = root.join("bench/cross/sysroot-aarch64");
    if !qemu.exists() || !sysroot.join("usr/lib/aarch64-linux-gnu/libc.so.6").exists() {
        eprintln!("aarch64_qemu: skipped: no AArch64 cross toolchain (run bench/cross/install.sh)");
        return;
    }
    let runner = format!("{} -L {}", qemu.display(), sysroot.display());
    let sysroot = sysroot.display().to_string();
    let env = [("LOIMI_TARGET", "aarch64-linux-gnu"), ("LOIMI_SYSROOT", sysroot.as_str()), ("LOIMI_RUNNER", runner.as_str())];
    let xenv: Vec<(String, String)> = env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    // Mostly clang and qemu time: three parts side by side.
    std::thread::scope(|s| {
        s.spawn(|| {
            run_programs_with(&xenv, |name, text| {
                directives(text, "matrix").iter().any(|m| m.trim() == "dispatch") || name.starts_with("gc_") || name.starts_with("wide_") || name.starts_with("simd_") || text.contains("import std.rt")
            })
        });
        s.spawn(|| {
            for mode in ["--dispatch=tailcall", "--dispatch=goto", "--dispatch=switch"] {
                mini_scripts_with(&[mode], &env, |name| {
                    ["sort", "err_sort", "coroutine", "err_coroutine", "gc","lib_gc", "err_syntax", "closures", "tables", "arith_quicken"].iter().any(|p| name.starts_with(p))
                });
            }
        });
        std_test_with(&xenv);
        let out = Command::new("bash").arg(root.join("examples/stack/check.sh")).env("LOIMI", env!("CARGO_BIN_EXE_loimi")).env("LOIMI_STD", root).envs(env).output().unwrap();
        assert!(out.status.success(), "examples/stack/check.sh (aarch64) failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    });
}

/// The vector (`simd_`) programs built for x86-64-v2 and x86-64-v3
/// (`--cpu`, through `LOIMI_CPU`): `swizzle` becomes `pshufb` and the
/// float rounding methods `roundps`/`roundpd` there (docs/lowering.md,
/// "Vectors"). v3 only on a host with AVX2.
#[test]
fn simd_cpu_levels() {
    if cross() || !cfg!(target_arch = "x86_64") {
        return;
    }
    let avx2 = std::fs::read_to_string("/proc/cpuinfo").is_ok_and(|s| s.contains(" avx2"));
    for cpu in ["x86-64-v2", "x86-64-v3"] {
        if cpu == "x86-64-v3" && !avx2 {
            eprintln!("simd_cpu_levels: x86-64-v3 skipped: the host has no AVX2");
            continue;
        }
        run_programs_with(&[("LOIMI_CPU".to_string(), cpu.to_string())], |name, _| name.starts_with("simd_"));
    }
}

/// The runner in `env` (`LOIMI_RUNNER`), else the process's.
fn runner_in(env: &[(&str, &str)]) -> String {
    env.iter().find(|(k, _)| *k == "LOIMI_RUNNER").map(|(_, v)| v.to_string()).unwrap_or_else(|| std::env::var("LOIMI_RUNNER").unwrap_or_default())
}

/// Distinguishes the executables of a cross build (`LOIMI_TARGET` in
/// `env`) from the host's built with the same flags.
fn target_tag(env: &[(&str, &str)]) -> String {
    env.iter().find(|(k, _)| *k == "LOIMI_TARGET").map(|(_, v)| format!("-{}", v)).unwrap_or_default()
}

/// The Mini scripts that `select` picks, with Mini built with `flags`.
/// `env` is the environment of the build and of the scripts' runs
/// (`LOIMI_TARGET`, `LOIMI_SYSROOT` and `LOIMI_RUNNER` cross-compile).
fn mini_scripts_with(flags: &[&str], env: &[(&str, &str)], select: impl Fn(&str) -> bool) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let exe = std::env::temp_dir().join(format!("loimi-mini-{}{}{}", std::process::id(), flags.join("").replace(['-', '='], ""), target_tag(env)));
    let out = loimi().envs(env.iter().copied()).arg("build").args(["-W", "error"]).args(flags).arg(root.join("examples/mini/main.lm")).arg("-o").arg(&exe).output().unwrap();
    assert!(out.status.success(), "building mini {:?} failed:\n{}", flags, String::from_utf8_lossy(&out.stderr));
    let mut failures = Vec::new();
    let mut scripts: Vec<PathBuf> = std::fs::read_dir(root.join("examples/mini/tests"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "mini") && select(&p.file_name().unwrap().to_string_lossy()))
        .collect();
    scripts.sort();
    assert!(!scripts.is_empty());
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
        for mode in GC_MODES {
            let mut cmd = program_under(&runner_in(env), &exe);
            cmd.arg(&f);
            cmd.envs(env.iter().copied());
            if !mode.is_empty() {
                cmd.arg(mode);
            }
            let out = cmd.output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let name = format!("{}{} {}", f.file_name().unwrap().to_string_lossy(), gc_mode_name(mode), flags.join(" "));
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
    mini_conformance_with(&[], &[], |_| true);
}

/// The conformance scripts whose name `select` picks, with Mini built
/// with `flags` and run in `env`.
fn mini_conformance_with(flags: &[&str], env: &[(&str, &str)], select: impl Fn(&str) -> bool) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let exe = std::env::temp_dir().join(format!("loimi-mini-conf-{}{}", std::process::id(), flags.join("").replace(['-', '='], "")));
    let out = loimi().arg("build").args(["-W", "error"]).args(flags).arg(root.join("examples/mini/main.lm")).arg("-o").arg(&exe).output().unwrap();
    assert!(out.status.success(), "building mini failed:\n{}", String::from_utf8_lossy(&out.stderr));
    let dir = root.join("examples/mini/tests/conformance");
    let mut scripts: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "lua") && select(&p.file_name().unwrap().to_string_lossy()))
        .collect();
    scripts.sort();
    let mut failures = Vec::new();
    let mut ran = 0;
    for f in scripts {
        let stem = f.file_stem().unwrap().to_string_lossy().to_string();
        ran += 1;
        let expected = std::fs::read_to_string(f.with_extension("expected")).unwrap();
        let should_fail = stem.starts_with("err_");
        for mode in GC_MODES {
            let mut cmd = program(&exe);
            cmd.arg(&f);
            cmd.envs(env.iter().copied());
            if !mode.is_empty() {
                cmd.arg(mode);
            }
            let out = cmd.output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let name = format!("{}{}", stem, gc_mode_name(mode));
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
    scheme_programs_with(&[], &[]);
}

/// The Scheme interpreter with the copy-and-patch JIT forced on (as
/// `mini_jit_stencils`).
#[test]
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
fn scheme_jit_stencils() {
    if cross() {
        return;
    }
    scheme_programs_with(&["--jit-stencils"], &[("LOIMI_JIT", "1"), ("LOIMI_JIT_HOT", "1")]);
    scheme_programs_with(&["--jit-stencils"], &[("LOIMI_JIT", "1"), ("LOIMI_JIT_HOT", "1"), ("LOIMI_JIT_RECOMPILE", "1")]);
}

fn scheme_programs_with(flags: &[&str], env: &[(&str, &str)]) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let exe = std::env::temp_dir().join(format!("loimi-scheme-{}{}", std::process::id(), flags.join("").replace(['-', '='], "")));
    let out = loimi().arg("build").args(["-W", "error"]).args(flags).arg(root.join("examples/scheme/main.lm")).arg("--release").arg("-o").arg(&exe).output().unwrap();
    assert!(out.status.success(), "building scheme {:?} failed:\n{}", flags, String::from_utf8_lossy(&out.stderr));
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
        for mode in GC_MODES {
            let mut cmd = program(&exe);
            cmd.arg(&f);
            cmd.envs(env.iter().copied());
            if !mode.is_empty() {
                cmd.arg(mode);
            }
            let out = cmd.output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let name = format!("{}{} {}", f.file_name().unwrap().to_string_lossy(), gc_mode_name(mode), flags.join(" "));
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

/// The MiniJS interpreter (examples/minijs): every NAME.js in
/// examples/minijs/tests and examples/minijs/tests/conformance must print
/// exactly NAME.expected (node's output). err_*.js must also exit with a
/// nonzero status; their .expected holds the output before the uncaught
/// exception. Run normally and with GC stress.
#[test]
fn minijs_programs() {
    minijs_programs_with(&[], &[]);
}

/// The MiniJS interpreter with the copy-and-patch JIT forced on (as
/// `mini_jit_stencils`).
#[test]
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
fn minijs_jit_stencils() {
    if cross() {
        return;
    }
    minijs_programs_with(&["--jit-stencils"], &[("LOIMI_JIT", "1"), ("LOIMI_JIT_HOT", "1")]);
    // Rewrites of compiled code without recompiles: out-of-line copies,
    // retargets, and sites left to the interpreter.
    minijs_programs_with(&["--jit-stencils"], &[("LOIMI_JIT", "1"), ("LOIMI_JIT_HOT", "1"), ("LOIMI_JIT_RECOMPILE", "1")]);
}

fn minijs_programs_with(flags: &[&str], env: &[(&str, &str)]) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let exe = std::env::temp_dir().join(format!("loimi-minijs-{}{}", std::process::id(), flags.join("").replace(['-', '='], "")));
    let out = loimi().arg("build").args(["-W", "error"]).args(flags).arg(root.join("examples/minijs/main.lm")).arg("--release").arg("-o").arg(&exe).output().unwrap();
    assert!(out.status.success(), "building minijs {:?} failed:\n{}", flags, String::from_utf8_lossy(&out.stderr));
    let mut programs: Vec<PathBuf> = Vec::new();
    for dir in ["examples/minijs/tests", "examples/minijs/tests/conformance"] {
        programs.extend(
            std::fs::read_dir(root.join(dir))
                .unwrap()
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "js")),
        );
    }
    programs.sort();
    let mut failures = Vec::new();
    for f in &programs {
        let stem = f.file_stem().unwrap().to_string_lossy().to_string();
        let expected = std::fs::read(f.with_extension("expected")).unwrap();
        let should_fail = stem.starts_with("err_");
        for mode in GC_MODES {
            let mut cmd = program(&exe);
            cmd.arg(f);
            cmd.envs(env.iter().copied());
            if !mode.is_empty() {
                cmd.arg(mode);
            }
            let out = cmd.output().unwrap();
            let name = format!("{}{} {}", stem, gc_mode_name(mode), flags.join(" "));
            if out.stdout != expected {
                failures.push(format!(
                    "{}: stdout mismatch\n--- expected\n{}--- got\n{}--- stderr\n{}",
                    name,
                    String::from_utf8_lossy(&expected),
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                ));
                continue;
            }
            if should_fail == out.status.success() {
                failures.push(format!("{}: exit {:?}, expected {}\n{}", name, out.status.code(), if should_fail { "nonzero" } else { "0" }, String::from_utf8_lossy(&out.stderr)));
            }
        }
    }
    let _ = std::fs::remove_file(&exe);
    assert!(programs.len() > 50);
    if !failures.is_empty() {
        panic!("{} failure(s):\n\n{}", failures.len(), failures.join("\n\n"));
    }
}

/// The Lox VM (examples/lox): every tests/NAME.lox prints its
/// `// expect: ` lines (the official suite's format), exits with
/// `// expect-exit:` (default 0) and has each `// expect-stderr:` text in
/// its stderr; run normally and with `--gc-stress`. The official suite is
/// bench/lox/runtests.py (not in the repository).
#[test]
fn lox_programs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let exe = std::env::temp_dir().join(format!("loimi-lox-{}", std::process::id()));
    let out = loimi().arg("build").args(["-W", "error"]).arg(root.join("examples/lox/main.lm")).arg("--release").arg("-o").arg(&exe).output().unwrap();
    assert!(out.status.success(), "building lox failed:\n{}", String::from_utf8_lossy(&out.stderr));
    let mut programs: Vec<PathBuf> = std::fs::read_dir(root.join("examples/lox/tests"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "lox"))
        .collect();
    programs.sort();
    let mut failures = Vec::new();
    for f in &programs {
        let text = std::fs::read_to_string(f).unwrap();
        let expect: Vec<String> = text.lines().filter_map(|l| l.split_once("// expect: ").map(|(_, r)| r.to_string())).collect();
        let exit: i32 = directives(&text, "expect-exit").first().map(|s| s.trim().parse().unwrap()).unwrap_or(0);
        let stderr_want = directives(&text, "expect-stderr");
        for mode in ["", "--gc-stress"] {
            let mut cmd = program(&exe);
            cmd.arg(f);
            if !mode.is_empty() {
                cmd.arg(mode);
            }
            let out = cmd.output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let name = format!("{} {}", f.file_name().unwrap().to_string_lossy(), mode);
            if stdout.lines().collect::<Vec<_>>() != expect.iter().map(|s| s.as_str()).collect::<Vec<_>>() {
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
    assert!(programs.len() >= 5);
    if !failures.is_empty() {
        panic!("{} failure(s):\n\n{}", failures.len(), failures.join("\n\n"));
    }
}

/// The Lox VM's unit tests (`loimi test examples/lox/lox_test.lm`, which
/// also runs small programs in-process with GC stress).
#[test]
fn lox_unit_tests() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = loimi().current_dir(root.join("examples/lox")).arg("test").arg("lox_test.lm").output().unwrap();
    assert!(
        out.status.success(),
        "loimi test examples/lox/lox_test.lm failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Stak, the stack VM (examples/stack): every tests/NAME.stk prints
/// tests/NAME.expected (stdout and stderr) in every dispatch mode, with and
/// without GC stress, built with `-W error` (examples/stack/check.sh).
#[test]
fn stack_programs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new("bash")
        .arg(root.join("examples/stack/check.sh"))
        .env("LOIMI", env!("CARGO_BIN_EXE_loimi"))
        .env("LOIMI_STD", root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "examples/stack/check.sh failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The Prolog interpreter (examples/prolog/check.sh -q): a tail-call build
/// with `-W error`, every tests/NAME.pl prints tests/NAME.expected
/// (SWI-Prolog 10's output, committed), normally and under GC stress.
/// Needs no reference Prolog.
#[test]
fn prolog_programs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new("bash")
        .arg(root.join("examples/prolog/check.sh"))
        .arg("-q")
        .env("LOIMI", env!("CARGO_BIN_EXE_loimi"))
        .env("LOIMI_STD", root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "examples/prolog/check.sh -q failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The Prolog interpreter built with `--jit-stencils` (check.sh -j): every
/// test with the JIT on and code compiled at its second call
/// (`LOIMI_JIT_HOT=2`), normally and under GC stress.
#[test]
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
fn prolog_jit_stencils() {
    if cross() {
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new("bash")
        .arg(root.join("examples/prolog/check.sh"))
        .arg("-j")
        .env("LOIMI", env!("CARGO_BIN_EXE_loimi"))
        .env("LOIMI_STD", root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "examples/prolog/check.sh -j failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The Python-subset interpreter (examples/python/check.sh -q): a tail-call
/// build, every tests/NAME.py prints tests/NAME.expected (CPython 3.13's
/// output, committed), normally and under GC stress. Needs no reference
/// interpreter.
#[test]
fn python_programs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new("bash")
        .arg(root.join("examples/python/check.sh"))
        .arg("-q")
        .env("LOIMI", env!("CARGO_BIN_EXE_loimi"))
        .env("LOIMI_STD", root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "examples/python/check.sh -q failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The Python-subset debugger (`--cfg debug`, examples/python/debugger.lm):
/// each examples/python/tests/debug/NAME.py run as `python-loimi --debug
/// NAME.py` with NAME.cmds on stdin must print the transcript NAME.out, in
/// every dispatch mode. The tail-call debug build also runs every
/// examples/python/tests program without the debugger.
#[test]
fn python_debugger() {
    if cross() {
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = root.join("examples/python/tests/debug");
    let mut tests: Vec<PathBuf> =
        std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "py")).collect();
    tests.sort();
    assert!(!tests.is_empty());
    let mut failures = Vec::new();
    for mode in ["tailcall", "goto", "switch"] {
        let exe = std::env::temp_dir().join(format!("loimi-python-debug-{}-{}", std::process::id(), mode));
        let out = loimi()
            .args(["build", "-W", "error", "--release", "--cfg", "debug", &format!("--dispatch={mode}")])
            .arg(root.join("examples/python/main.lm"))
            .arg("-o")
            .arg(&exe)
            .output()
            .unwrap();
        assert!(out.status.success(), "building python --cfg debug ({mode}) failed:\n{}", String::from_utf8_lossy(&out.stderr));
        for t in &tests {
            let cmds = std::fs::File::open(t.with_extension("cmds")).unwrap();
            let want = std::fs::read_to_string(t.with_extension("out")).unwrap();
            let out = limited(&exe).current_dir(&dir).arg("--debug").arg(t.file_name().unwrap()).stdin(cmds).output().unwrap();
            let got = String::from_utf8_lossy(&out.stdout);
            if got != want {
                failures.push(format!(
                    "{} ({mode}): transcript mismatch\n--- expected\n{want}\n--- got\n{got}\n--- stderr\n{}",
                    t.display(),
                    String::from_utf8_lossy(&out.stderr)
                ));
            }
        }
        if mode == "tailcall" {
            let progs = root.join("examples/python/tests");
            let mut ps: Vec<PathBuf> = std::fs::read_dir(&progs)
                .unwrap()
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "py") && p.with_extension("expected").exists())
                .collect();
            ps.sort();
            for p in &ps {
                let want = std::fs::read_to_string(p.with_extension("expected")).unwrap();
                let out = limited(&exe).arg(p).output().unwrap();
                if String::from_utf8_lossy(&out.stdout) != want {
                    failures.push(format!("{} (--cfg debug build, no debugger): output differs", p.display()));
                }
            }
        }
        let _ = std::fs::remove_file(&exe);
    }
    if !failures.is_empty() {
        panic!("{} failure(s):\n\n{}", failures.len(), failures.join("\n\n"));
    }
}

/// std.bigint against Python's int (tests/bigint/diff.py): 3000 random
/// cases through the owned and GC APIs, then 1000 more with the GC
/// collecting at every safepoint. Uses CPython from bench/python/install
/// if it was built, else `python3` (int semantics are the same in every
/// Python 3); skipped when there is neither.
#[test]
fn bigint_vs_python() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let built = root.join("bench/python/install/bin/python3");
    let python = if built.exists() { built.to_string_lossy().to_string() } else { "python3".to_string() };
    if Command::new(&python).arg("-c").arg("pass").output().is_err() {
        eprintln!("bigint_vs_python: no python3, skipped");
        return;
    }
    for extra in [&["--seed", "1"][..], &["--seed", "2", "--cases", "1000", "--stress"][..]] {
        let out = Command::new(&python)
            .arg(root.join("tests/bigint/diff.py"))
            .arg(env!("CARGO_BIN_EXE_loimi"))
            .args(extra)
            .env("LOIMI_STD", root)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "tests/bigint/diff.py {:?} failed:\n{}{}",
            extra,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// The WebAssembly interpreter (examples/wasm/check.sh --fast): a tail-call
/// build with `-W error`, the C programs in tests/ compiled to wasm32 (when
/// clang-19 and wasm-ld are there) and a subset of the official spec tests
/// (when bench/wasm/install.sh and spec.sh have run; skipped otherwise).
#[test]
fn wasm_interpreter() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new("bash")
        .arg(root.join("examples/wasm/check.sh"))
        .arg("--fast")
        .env("LOIMI", env!("CARGO_BIN_EXE_loimi"))
        .env("LOIMI_STD", root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "examples/wasm/check.sh failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The regex library (examples/regex/check.sh, fast): its C host's demo,
/// the bytecode fuzzer, the verifier test, random byte patterns on both
/// engines, and both engines against Python's `re` on the hand-written
/// cases and 100 random ones (fixed seed).
#[test]
fn regex_library() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out_dir = std::env::temp_dir().join(format!("loimi-regex-{}", std::process::id()));
    let out = Command::new("sh")
        .arg(root.join("examples/regex/check.sh"))
        .args(["--fast", "--random", "100", "--seed", "1"])
        .env("LOIMI", env!("CARGO_BIN_EXE_loimi"))
        .env("LOIMI_STD", root)
        .env("RX_OUT", &out_dir)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&out_dir);
    assert!(
        out.status.success(),
        "examples/regex/check.sh failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The tutorial (docs/tutorial, examples/tutorial): every step builds and
/// passes its tests in every dispatch mode, with and without GC stress
/// (examples/tutorial/check.sh), and every ```loimi block in the chapters
/// appears in a step's sources. A block may elide code with a line `...`
/// or `// ...`; each part between elisions must match consecutive lines
/// of one file, ignoring indentation.
#[test]
fn tutorial_steps() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new("bash")
        .arg(root.join("examples/tutorial/check.sh"))
        .env("LOIMI", env!("CARGO_BIN_EXE_loimi"))
        .env("LOIMI_STD", root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "examples/tutorial/check.sh failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    fn sources(dir: &Path, out: &mut Vec<(PathBuf, Vec<String>)>) {
        for e in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                sources(&p, out);
            } else if p.extension().is_some_and(|x| x == "lm") {
                let lines = std::fs::read_to_string(&p).unwrap().lines().map(|l| l.trim().to_string()).collect();
                out.push((p, lines));
            }
        }
    }
    let mut files = Vec::new();
    sources(&root.join("examples/tutorial"), &mut files);
    let mut failures = Vec::new();
    let mut docs: Vec<PathBuf> = std::fs::read_dir(root.join("docs/tutorial"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    docs.sort();
    let mut blocks = 0;
    for doc in &docs {
        let text = std::fs::read_to_string(doc).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            if lines[i].trim() != "```loimi" {
                i += 1;
                continue;
            }
            let start = i + 1;
            let mut end = start;
            while end < lines.len() && lines[end].trim() != "```" {
                end += 1;
            }
            blocks += 1;
            let body: Vec<String> = lines[start..end].iter().map(|l| l.trim().to_string()).collect();
            for part in body.split(|l| l == "..." || l == "// ...") {
                let a = part.iter().position(|l| !l.is_empty());
                let b = part.iter().rposition(|l| !l.is_empty());
                let (Some(a), Some(b)) = (a, b) else { continue };
                let part = &part[a..=b];
                let found = files.iter().any(|(_, fl)| fl.windows(part.len()).any(|w| w == part));
                if !found {
                    failures.push(format!("{}:{}: snippet not found in examples/tutorial:\n{}", doc.display(), start + 1, part.join("\n")));
                }
            }
            i = end + 1;
        }
    }
    assert!(blocks > 50, "only {} loimi blocks found", blocks);
    if !failures.is_empty() {
        panic!("{} failure(s):\n\n{}", failures.len(), failures.join("\n\n"));
    }
}

/// `variant .. wraps all` in goto mode: the tutorial's step 9 with every
/// op wrapped used to take minutes in clang (entries that never switch
/// tables carried every wrapper block, and LLVM's loop strength
/// reduction blew up on them). It must build in well under a minute.
#[test]
fn variant_wraps_all_goto_build_time() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = std::env::temp_dir().join(format!("loimi-wraps-all-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for e in std::fs::read_dir(root.join("examples/tutorial/step9")).unwrap().filter_map(|e| e.ok()) {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "lm") {
            let mut text = std::fs::read_to_string(&p).unwrap();
            if let Some(a) = text.find("variant budget wraps [") {
                let b = a + text[a..].find("] {").expect("end of the wraps list") + 3;
                text.replace_range(a..b, "variant budget wraps all {");
            }
            std::fs::write(dir.join(p.file_name().unwrap()), text).unwrap();
        }
    }
    let vm = std::fs::read_to_string(dir.join("vm.lm")).unwrap();
    assert!(vm.contains("variant budget wraps all {"), "step 9 has no `variant budget wraps [..]`");
    let start = std::time::Instant::now();
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!("ulimit -v {} && exec timeout 300 \"$0\" \"$@\"", MEM_LIMIT_KB))
        .arg(env!("CARGO_BIN_EXE_loimi"))
        .args(["build", "main.lm", "--dispatch=goto", "-o", "pip"])
        .current_dir(&dir)
        .env("LOIMI_STD", root)
        .output()
        .unwrap();
    let took = start.elapsed();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "build failed after {:?}:\n{}", took, String::from_utf8_lossy(&out.stderr));
    assert!(took.as_secs() < 60, "building step 9 with `wraps all` in goto mode took {:?}", took);
}

/// `--keep-ll` keeps the IR next to the output, where it survives the
/// temporary build directory (it used to be printed as kept and then
/// deleted with that directory), and as `<stem>.ll` without `-o`.
#[test]
fn keep_ll() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = std::env::temp_dir().join(format!("loimi-keep-ll-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let hello = root.join("tests/run/hello.lm");
    let out = loimi().args(["build", "--keep-ll", "-o"]).arg(dir.join("hi")).arg(&hello).output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "build failed:\n{}", stderr);
    let ll = dir.join("hi.ll");
    assert!(stderr.contains(&format!("IR kept at {}", ll.display())), "{}", stderr);
    assert!(std::fs::read_to_string(&ll).unwrap().contains("define "), "{} is not IR", ll.display());
    assert!(dir.join("hi").exists());
    let out = loimi().args(["run", "--keep-ll"]).arg(&hello).current_dir(&dir).output().unwrap();
    assert!(out.status.success(), "run failed:\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(dir.join("hello.ll").exists(), "`run --keep-ll` kept no hello.ll");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Build `lib` as a static archive and as a shared object (with the C
/// header from `--header`), compile `host` against each with clang, and
/// compare the host's output with `expected`. `header_golden`: the header
/// must equal this file.
fn c_host_run(name: &str, lib: &Path, host: &Path, expected: &Path, header_golden: Option<&Path>) {
    c_host_run_with(name, lib, host, expected, header_golden, &[]);
}

/// `c_host_run` with extra build flags for the library (`--cfg gil`).
fn c_host_run_with(name: &str, lib: &Path, host: &Path, expected: &Path, header_golden: Option<&Path>, extra: &[&str]) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = std::env::temp_dir().join(format!("loimi-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let want = std::fs::read_to_string(expected).unwrap();
    let clang = std::env::var("LOIMI_CLANG").unwrap_or_else(|_| "clang-19".into());
    let variants: [(&str, &[&str]); 4] = [
        ("static", &["--lib"]),
        ("static-release", &["--lib", "--release"]),
        ("shared", &["--shared"]),
        ("shared-initial-exec", &["--shared", "--release", "--tls=initial-exec"]),
    ];
    let mut failures = Vec::new();
    for (variant, flags) in variants {
        let vdir = dir.join(variant);
        std::fs::create_dir_all(&vdir).unwrap();
        let shared = flags.contains(&"--shared");
        let libfile = vdir.join(if shared { "liblm.so" } else { "liblm.a" });
        let header = vdir.join(header_file(host));
        let out = loimi().arg("build").arg(lib).args(flags).args(extra).arg("-o").arg(&libfile).arg("--header").arg(&header).output().unwrap();
        if !out.status.success() {
            failures.push(format!("{} {}: build failed:\n{}", name, variant, String::from_utf8_lossy(&out.stderr)));
            continue;
        }
        if let Some(g) = header_golden {
            let got = std::fs::read_to_string(&header).unwrap();
            if got != std::fs::read_to_string(g).unwrap() {
                failures.push(format!("{} {}: header differs from {}:\n{}", name, variant, g.display(), got));
            }
        }
        let exe = vdir.join("host");
        let mut cc = cc(&clang);
        cc.arg("-Wall").arg("-Werror").arg("-pthread").arg("-I").arg(&vdir).arg(host);
        if shared {
            cc.arg("-L").arg(&vdir).arg("-llm").arg(format!("-Wl,-rpath,{}", vdir.display()));
        } else {
            cc.arg(&libfile);
        }
        let out = cc.arg("-lm").arg("-o").arg(&exe).output().unwrap();
        if !out.status.success() {
            failures.push(format!("{} {}: compiling {} failed:\n{}", name, variant, host.display(), String::from_utf8_lossy(&out.stderr)));
            continue;
        }
        let out = program(&exe).current_dir(root).output().unwrap();
        let got = String::from_utf8_lossy(&out.stdout);
        if !out.status.success() || got != want {
            failures.push(format!(
                "{} {}: exit {:?}\n--- expected\n{}--- got\n{}--- stderr\n{}",
                name,
                variant,
                out.status.code(),
                want,
                got,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    if !failures.is_empty() {
        panic!("{} failure(s):\n\n{}", failures.len(), failures.join("\n\n"));
    }
}

/// The header a host includes: the first `#include "..."`.
fn header_file(host: &Path) -> String {
    let text = std::fs::read_to_string(host).unwrap();
    text.lines()
        .find_map(|l| l.trim().strip_prefix("#include \"").and_then(|r| r.strip_suffix('"')))
        .expect("host includes a generated header")
        .to_string()
}

/// `export "C"` functions called from C through the generated header:
/// every mapped type, `std.rt` protect/throw, static and shared builds.
#[test]
fn c_library() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let d = root.join("tests/embed");
    c_host_run("c-library", &d.join("api.lm"), &d.join("host.c"), &d.join("host.expected"), Some(&d.join("api.h")));
}

/// Panics in a library become error returns: `#[catch_panic]` exports
/// (bounds check, `unwrap`, `trap()`, division by zero, `rt.panic`, out of
/// memory), `rt.catch_panic` under `rt.protect`, panics unwinding through
/// host callbacks, GC roots on the shadow stack, panics in nested
/// (re-entrant) vm runs restoring `Vm.depth`; an uncaught one aborts.
#[test]
fn c_library_panics() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let d = root.join("tests/embed");
    c_host_run("c-library-panics", &d.join("panic.lm"), &d.join("panic_host.c"), &d.join("panic_host.expected"), None);
}

/// `--vm-stats` in a static library: the stats runtime goes into the
/// archive, and the report prints when the host exits, its suggestions
/// overlapping neither each other nor the vm's supers; two such
/// libraries link into one host.
#[test]
fn vm_stats_library() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let rx = root.join("examples/regex");
    let dir = std::env::temp_dir().join(format!("loimi-stats-lib-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let lib = dir.join("librx.a");
    let out = loimi()
        .arg("build")
        .arg(rx.join("lib.lm"))
        .args(["--lib", "--release", "--vm-stats", "-o"])
        .arg(&lib)
        .arg("--header")
        .arg(dir.join("rx.h"))
        .output()
        .unwrap();
    assert!(out.status.success(), "build failed:\n{}", String::from_utf8_lossy(&out.stderr));
    let exe = dir.join("rxhost");
    let clang = std::env::var("LOIMI_CLANG").unwrap_or_else(|_| "clang-19".into());
    let out = cc(&clang).arg("-I").arg(&dir).arg(rx.join("host.c")).arg(&lib).args(["-lm", "-o"]).arg(&exe).output().unwrap();
    assert!(out.status.success(), "host: {}", String::from_utf8_lossy(&out.stderr));
    let out = program(&exe).arg("demo").output().unwrap();
    let report = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "demo failed:\n{}", report);
    assert!(report.contains("== vm stats: Rx =="), "no report at exit:\n{}", report);
    assert!(report.contains("suggested superinstructions"), "no suggestions:\n{}", report);
    // A second `--vm-stats` library in the same host, whole-archive so
    // that both copies of the stats runtime are linked: no clash.
    let lib2 = dir.join("libp.a");
    let out = loimi()
        .arg("build")
        .arg(root.join("tests/embed/panic.lm"))
        .args(["--lib", "--vm-stats", "-o"])
        .arg(&lib2)
        .arg("--header")
        .arg(dir.join("panic.h"))
        .output()
        .unwrap();
    assert!(out.status.success(), "build failed:\n{}", String::from_utf8_lossy(&out.stderr));
    let exe = dir.join("two");
    let out = cc(&clang)
        .args(["-Wall", "-Werror", "-I"])
        .arg(&dir)
        .arg(root.join("tests/embed/two_stats_host.c"))
        .arg("-Wl,--whole-archive")
        .arg(&lib)
        .arg(&lib2)
        .args(["-Wl,--no-whole-archive", "-lm", "-o"])
        .arg(&exe)
        .output()
        .unwrap();
    assert!(out.status.success(), "two libraries: {}", String::from_utf8_lossy(&out.stderr));
    let out = program(&exe).output().unwrap();
    let report = String::from_utf8_lossy(&out.stderr).to_string();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "two libraries failed:\n{}", report);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "rx 1\nrvm 6\n");
    assert!(report.contains("== vm stats: Rx ==") && report.contains("== vm stats: R =="), "reports:\n{}", report);
}

/// Stale pointers into std's containers under `--sanitize=address`
/// (docs/design/safety.md section 7): each tests/asan program built with
/// ASan must be reported with its `// expect-stderr:` text (a nonzero
/// exit), or print its `// expect:` lines and exit 0. Skipped under
/// `ulimit -v` (ASan reserves terabytes of shadow memory) and when
/// cross-compiling.
#[test]
fn asan_stale_pointers() {
    if cross() {
        return;
    }
    let limits = std::fs::read_to_string("/proc/self/limits").unwrap_or_default();
    if limits.lines().any(|l| l.starts_with("Max address space") && l.split_whitespace().nth(3) != Some("unlimited")) {
        eprintln!("asan_stale_pointers: skipped under an address-space limit (ulimit -v)");
        return;
    }
    let dir = std::env::temp_dir().join(format!("loimi-asan-stale-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut failures = Vec::new();
    for f in files("tests/asan") {
        let text = std::fs::read_to_string(&f).unwrap();
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let exe = dir.join(name.trim_end_matches(".lm"));
        let out = loimi().arg("build").arg(&f).args(["--sanitize=address", "-o"]).arg(&exe).output().unwrap();
        if !out.status.success() {
            failures.push(format!("{}: build failed:\n{}", name, String::from_utf8_lossy(&out.stderr)));
            continue;
        }
        let mut c = Command::new("sh");
        c.arg("-c").arg("ulimit -c 0 && exec \"$0\"").arg(&exe);
        let out = c.env("ASAN_OPTIONS", "hard_rss_limit_mb=2000:detect_leaks=0").output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let want_err = directives(&text, "expect-stderr");
        if want_err.is_empty() {
            let expect = directives(&text, "expect");
            if !out.status.success() || stdout.lines().collect::<Vec<_>>() != expect.iter().map(|s| s.as_str()).collect::<Vec<_>>() {
                failures.push(format!("{}: exit {:?}, stdout:\n{}\nstderr:\n{}", name, out.status.code(), stdout, stderr));
            }
        } else if out.status.success() || !want_err.iter().all(|w| stderr.contains(w.as_str())) {
            failures.push(format!("{}: expected an ASan report with {:?}; exit {:?}, stderr:\n{}", name, want_err, out.status.code(), stderr));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    if !failures.is_empty() {
        panic!("{} failure(s):\n\n{}", failures.len(), failures.join("\n\n"));
    }
}

/// `--sanitize=address` with `--lib` and `--shared`: the library carries
/// the instrumentation, the host links ASan's runtime (gcc's libasan,
/// as `loimi build --sanitize=address` does for executables). Caught
/// panics unwind cleanly under ASan. Not under `ulimit -v`, which ASan
/// cannot run with: its RSS limit bounds it instead.
#[test]
fn c_library_asan() {
    if cross() {
        return;
    }
    // ASan reserves terabytes of shadow memory: skip under `ulimit -v`.
    let limits = std::fs::read_to_string("/proc/self/limits").unwrap_or_default();
    if limits.lines().any(|l| l.starts_with("Max address space") && l.split_whitespace().nth(3) != Some("unlimited")) {
        eprintln!("c_library_asan: skipped under an address-space limit (ulimit -v)");
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let d = root.join("tests/embed");
    let dir = std::env::temp_dir().join(format!("loimi-asan-lib-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let want = std::fs::read_to_string(d.join("panic_host.expected")).unwrap();
    for (variant, flag, lib) in [("static", "--lib", "libp.a"), ("shared", "--shared", "libp.so")] {
        let libfile = dir.join(lib);
        let out = loimi()
            .arg("build")
            .arg(d.join("panic.lm"))
            .args([flag, "--sanitize=address", "-o"])
            .arg(&libfile)
            .arg("--header")
            .arg(dir.join("panic.h"))
            .output()
            .unwrap();
        assert!(out.status.success(), "{}: build failed:\n{}", variant, String::from_utf8_lossy(&out.stderr));
        let exe = dir.join(format!("host-{}", variant));
        let mut cc = Command::new("gcc");
        cc.args(["-fsanitize=address", "-I"]).arg(&dir).arg(d.join("panic_host.c"));
        if flag == "--shared" {
            cc.arg("-L").arg(&dir).arg("-lp").arg(format!("-Wl,-rpath,{}", dir.display()));
        } else {
            cc.arg(&libfile);
        }
        let out = cc.arg("-lm").arg("-o").arg(&exe).output().unwrap();
        assert!(out.status.success(), "{}: gcc failed:\n{}", variant, String::from_utf8_lossy(&out.stderr));
        let out = Command::new(&exe)
            .env("ASAN_OPTIONS", "hard_rss_limit_mb=2000:allocator_may_return_null=1:detect_leaks=0")
            .output()
            .unwrap();
        let got = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && got == want,
            "{}: exit {:?}\n--- got\n{}--- stderr\n{}",
            variant,
            out.status.code(),
            got,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Mini embedded in a C host (examples/embed/host.c): two states alive at
/// once, globals both ways, C natives, syntax and runtime errors,
/// natives calling back into Mini (nested, failing, under GC stress), the
/// host calling Mini functions, an allocation-heavy script under GC stress,
/// and a native of one state running the other with both under GC stress
/// (the collector must not mark the other heap's objects on the shared
/// shadow stack).
#[test]
fn mini_embedded() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let e = root.join("examples/embed");
    c_host_run("mini-embed", &root.join("examples/mini/embed.lm"), &e.join("host.c"), &e.join("host.expected"), None);
}

/// Independent Mini states on four threads at once (examples/embed/threads.c):
/// allocation-heavy scripts with nested runs, two threads under GC stress,
/// a caught internal error (panic) on each thread while the others run,
/// then every state run by a thread other than the one that created it.
/// Static and shared builds (initial-exec and local-dynamic TLS).
#[test]
fn mini_threads() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let e = root.join("examples/embed");
    c_host_run("mini-threads", &root.join("examples/mini/embed.lm"), &e.join("threads.c"), &e.join("threads.expected"), None);
}

/// One Mini state shared by five threads, one at a time (`--cfg gil`,
/// examples/embed/shared.c): a hand-over at a loop back-edge, then four
/// threads calling the same functions on shared tables under GC stress,
/// releasing the lock in a native and between calls.
#[test]
fn mini_shared() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let e = root.join("examples/embed");
    c_host_run_with(
        "mini-shared",
        &root.join("examples/mini/embed.lm"),
        &e.join("shared.c"),
        &e.join("shared.expected"),
        None,
        &["--cfg", "gil"],
    );
}

/// Mini's debugging API (`--cfg debug`, the end of examples/mini/embed.lm)
/// from a C host (examples/embed/debug.c): stop on entry, pending
/// breakpoints, conditions and hit conditions, stepping, frames, locals
/// and globals, lazy table expansion, evaluation with breakpoints
/// suspended, break on error, a pause from another thread; under GC
/// stress.
#[test]
fn mini_debug_api() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let e = root.join("examples/embed");
    c_host_run_with(
        "mini-debug-api",
        &root.join("examples/mini/embed.lm"),
        &e.join("debug.c"),
        &e.join("debug.expected"),
        None,
        &["--cfg", "debug"],
    );
}

/// The DAP adapter for Mini (examples/mini/dap/mini_dap.c, over the
/// debugging API) driven through a scripted session by
/// examples/mini/dap/test_dap.py: breakpoints (moved off a blank line,
/// conditional, hit-counted, set while the program runs), stepping,
/// stack traces, scopes, nested table expansion, evaluation, a pause,
/// break on error, the program's output as events. The transcript must
/// be examples/mini/tests/dap/session.out, in every dispatch mode.
#[test]
fn mini_dap() {
    if cross() || Command::new("python3").arg("--version").output().is_err() {
        eprintln!("mini_dap: skipped (cross-compiling, or no python3)");
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dap = root.join("examples/mini/dap");
    let want = std::fs::read_to_string(root.join("examples/mini/tests/dap/session.out")).unwrap();
    let dir = std::env::temp_dir().join(format!("loimi-mini-dap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let clang = std::env::var("LOIMI_CLANG").unwrap_or_else(|_| "clang-19".into());
    let mut failures = Vec::new();
    for mode in ["tailcall", "goto", "switch"] {
        let lib = dir.join(format!("libmini-{mode}.a"));
        let header = dir.join("mini.h");
        let out = loimi()
            .args(["build", "-W", "error", "--cfg", "debug", "--lib", &format!("--dispatch={mode}")])
            .arg(root.join("examples/mini/embed.lm"))
            .arg("-o")
            .arg(&lib)
            .arg("--header")
            .arg(&header)
            .output()
            .unwrap();
        assert!(out.status.success(), "building the debug library ({mode}) failed:\n{}", String::from_utf8_lossy(&out.stderr));
        let exe = dir.join(format!("mini-dap-{mode}"));
        let out = cc(&clang)
            .args(["-Wall", "-Wextra", "-Werror", "-pthread", "-I"])
            .arg(&dir)
            .arg(dap.join("mini_dap.c"))
            .arg(&lib)
            .args(["-lm", "-o"])
            .arg(&exe)
            .output()
            .unwrap();
        assert!(out.status.success(), "compiling mini_dap.c failed:\n{}", String::from_utf8_lossy(&out.stderr));
        let out = limited("python3").arg(dap.join("test_dap.py")).arg(&exe).output().unwrap();
        let got = String::from_utf8_lossy(&out.stdout);
        if !out.status.success() || got != want {
            failures.push(format!(
                "{mode}: transcript mismatch (exit {:?})\n--- expected\n{want}\n--- got\n{got}\n--- stderr\n{}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    if !failures.is_empty() {
        panic!("{} failure(s):\n\n{}", failures.len(), failures.join("\n\n"));
    }
}

/// Mini built with the lock (`--cfg gil`) and run by one thread: every
/// script and conformance test, in every GC mode (the polls at back-edges
/// and calls never hand over).
#[test]
fn mini_gil_single_thread() {
    mini_scripts_with(&["--cfg", "gil"], &[], |_| true);
    mini_conformance_with(&["--cfg", "gil"], &[], |_| true);
}

/// The same under ThreadSanitizer: `--sanitize=thread` instruments the
/// library, gcc links its libtsan into the host. No data race may be
/// reported. Skipped under `ulimit -v` (TSan maps terabytes of shadow
/// memory) and where gcc has no libtsan.
#[test]
fn mini_threads_tsan() {
    tsan_host("threads", &[]);
}

/// `mini_shared` under ThreadSanitizer: the lock's hand-overs, and the
/// collector reading other threads' saved state, race-free.
#[test]
fn mini_shared_tsan() {
    tsan_host("shared", &["--cfg", "gil"]);
}

/// Build examples/mini/embed.lm with `--sanitize=thread` (and `extra`),
/// link examples/embed/<host>.c with gcc's libtsan, and compare its output
/// with <host>.expected; no race may be reported.
fn tsan_host(host: &str, extra: &[&str]) {
    if cross() {
        return;
    }
    let limits = std::fs::read_to_string("/proc/self/limits").unwrap_or_default();
    if limits.lines().any(|l| l.starts_with("Max address space") && l.split_whitespace().nth(3) != Some("unlimited")) {
        eprintln!("mini {} tsan: skipped under an address-space limit (ulimit -v)", host);
        return;
    }
    let has_tsan = Command::new("gcc")
        .arg("-print-file-name=libtsan.so")
        .output()
        .is_ok_and(|o| Path::new(String::from_utf8_lossy(&o.stdout).trim()).is_absolute());
    if !has_tsan {
        eprintln!("mini {} tsan: skipped, gcc has no libtsan", host);
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let e = root.join("examples/embed");
    let dir = std::env::temp_dir().join(format!("loimi-tsan-{}-{}", host, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let lib = dir.join("libmini.a");
    let out = loimi()
        .arg("build")
        .arg(root.join("examples/mini/embed.lm"))
        .args(["--lib", "--release", "--sanitize=thread"])
        .args(extra)
        .arg("-o")
        .arg(&lib)
        .arg("--header")
        .arg(dir.join("mini.h"))
        .output()
        .unwrap();
    assert!(out.status.success(), "build failed:\n{}", String::from_utf8_lossy(&out.stderr));
    let exe = dir.join(host);
    let out = Command::new("gcc")
        .args(["-fsanitize=thread", "-pthread", "-I"])
        .arg(&dir)
        .arg(e.join(format!("{}.c", host)))
        .arg(&lib)
        .args(["-lm", "-o"])
        .arg(&exe)
        .output()
        .unwrap();
    assert!(out.status.success(), "gcc failed:\n{}", String::from_utf8_lossy(&out.stderr));
    let out = Command::new(&exe).env("TSAN_OPTIONS", "halt_on_error=1:exitcode=66:history_size=2").output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let got = String::from_utf8_lossy(&out.stdout);
    let want = std::fs::read_to_string(e.join(format!("{}.expected", host))).unwrap();
    assert!(
        out.status.success() && got == want,
        "exit {:?}\n--- got\n{}--- stderr\n{}",
        out.status.code(),
        got,
        String::from_utf8_lossy(&out.stderr)
    );
}
