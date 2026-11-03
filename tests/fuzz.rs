//! Smoke fuzzing: a few seconds of the bytecode fuzzers, without ASan, so
//! that verifier regressions show up in `cargo test`. The fuzzers check
//! more than crashes (the dispatch modes agree, quickened and optimized
//! code still verifies). Long runs under ASan: tests/fuzz/run.sh.

use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const MEM_LIMIT_KB: u64 = 2 * 1024 * 1024;

fn limited(prog: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(format!("ulimit -v {} && exec \"$0\" \"$@\"", MEM_LIMIT_KB)).arg(prog);
    c
}

fn build(src: &Path, exe: &Path) {
    build_with(src, exe, &[]);
}

fn build_with(src: &Path, exe: &Path, flags: &[&str]) {
    let out = limited(env!("CARGO_BIN_EXE_loimi"))
        .env("LOIMI_STD", env!("CARGO_MANIFEST_DIR"))
        .arg("build")
        .arg(src)
        .args(flags)
        .arg("-o")
        .arg(exe)
        .output()
        .unwrap();
    assert!(out.status.success(), "building {} failed:\n{}", src.display(), String::from_utf8_lossy(&out.stderr));
}

/// Run seeds first..first+count. A process killed by SIGALRM (a Mini
/// program past its time limit) resumes after the seed it was running.
fn run(exe: &Path, first: u64, count: u64) {
    let mut seed = first;
    let end = first + count;
    while seed < end {
        let out = limited(exe).arg(seed.to_string()).arg((end - seed).to_string()).arg("-v").output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let last = stderr.lines().filter_map(|l| l.strip_prefix("seed ")).last().and_then(|s| s.parse::<u64>().ok());
        if out.status.success() {
            return;
        }
        let failures: Vec<&str> = stdout.lines().filter(|l| l.starts_with("FAIL")).collect();
        if !failures.is_empty() {
            panic!("{}: {}\nreproduce: {} <seed> 1 -d", exe.display(), failures.join("\n"), exe.display());
        }
        match (out.status.signal(), last) {
            (Some(14), Some(s)) => seed = s + 1,
            _ => panic!(
                "{} crashed ({:?}) at seed {:?}; reproduce: {} <seed> 1 -d\n{}",
                exe.display(),
                out.status,
                last,
                exe.display(),
                stderr.lines().filter(|l| !l.starts_with("seed ") && *l != "ran").collect::<Vec<_>>().join("\n")
            ),
        }
    }
}

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("loimi-fuzz-{}-{}", std::process::id(), name))
}

#[test]
fn fuzz_test_vm() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let exe = tmp("fuzzvm");
    build(&root.join("tests/fuzz/fuzzvm.lm"), &exe);
    run(&exe, 0, 20000);
    let _ = std::fs::remove_file(&exe);
}

/// Mini's verify clauses, through tests/fuzz/minifuzz.lm, which imports
/// Mini's modules from a directory of links.
#[test]
fn fuzz_mini() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = tmp("mini-src");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for m in ["value", "vm", "lib", "coro"] {
        std::os::unix::fs::symlink(root.join(format!("examples/mini/{m}.lm")), dir.join(format!("{m}.lm"))).unwrap();
    }
    std::os::unix::fs::symlink(root.join("tests/fuzz/minifuzz.lm"), dir.join("minifuzz.lm")).unwrap();
    std::os::unix::fs::symlink(root.join("tests/fuzz/mini_holes.lm"), dir.join("mini_holes.lm")).unwrap();
    // The verify holes the fuzzer found stay rejected.
    let holes = tmp("mini-holes");
    build(&dir.join("mini_holes.lm"), &holes);
    let out = limited(&holes).output().unwrap();
    assert!(out.status.success(), "mini_holes: {}", String::from_utf8_lossy(&out.stdout));
    let _ = std::fs::remove_file(&holes);
    let exe = tmp("minifuzz");
    build(&dir.join("minifuzz.lm"), &exe);
    run(&exe, 0, 3000);
    let _ = std::fs::remove_file(&exe);
    let _ = std::fs::remove_dir_all(&dir);
}

/// tests/fuzz/fuzzcombo.lm: superinstructions against their parts run
/// alone, built in each dispatch mode and with the JIT, normally and
/// under GC stress. Every build must report the same outcome hash.
#[test]
fn fuzz_vm_combinations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = root.join("tests/fuzz/fuzzcombo.lm");
    let jit = cfg!(all(target_arch = "x86_64", target_os = "linux"));
    let mut builds: Vec<(&str, Vec<&str>, Vec<(&str, &str)>)> = vec![
        ("tailcall", vec!["--dispatch=tailcall"], vec![]),
        ("goto", vec!["--dispatch=goto"], vec![]),
        ("switch", vec!["--dispatch=switch"], vec![]),
    ];
    if jit {
        builds.push(("jit", vec!["--jit-stencils"], vec![("LOIMI_JIT", "1"), ("LOIMI_JIT_HOT", "1")]));
    }
    let mut hashes: Vec<(String, String)> = Vec::new();
    for (name, flags, env) in &builds {
        let exe = tmp(&format!("fuzzcombo-{}", name));
        build_with(&src, &exe, flags);
        for (count, extra) in [("3000", None), ("1000", Some("stress"))] {
            let mut c = limited(&exe);
            c.arg("0").arg(count).envs(env.iter().copied());
            if let Some(x) = extra {
                c.arg(x);
            }
            if !env.is_empty() {
                c.arg("jit");
            }
            let out = c.output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            assert!(
                out.status.success(),
                "fuzzcombo ({} {:?}) failed ({:?}):\n{}\nreproduce: {} <seed> 1 -d",
                name,
                extra,
                out.status,
                stdout,
                exe.display()
            );
            let h = stdout.lines().last().and_then(|l| l.split("outcome hash ").nth(1)).unwrap_or("").to_string();
            hashes.push((format!("{} {:?}", name, extra), h));
        }
        let _ = std::fs::remove_file(&exe);
    }
    for (name, h) in &hashes {
        let base = &hashes[if name.contains("stress") { 1 } else { 0 }];
        assert_eq!(h, &base.1, "fuzzcombo: {} computes other results than {}", name, base.0);
    }
}
