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
    let out = limited(env!("CARGO_BIN_EXE_loimi"))
        .env("LOIMI_STD", env!("CARGO_MANIFEST_DIR"))
        .arg("build")
        .arg(src)
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
    for m in ["value", "vm", "lib"] {
        std::os::unix::fs::symlink(root.join(format!("examples/mini/{m}.lm")), dir.join(format!("{m}.lm"))).unwrap();
    }
    std::os::unix::fs::symlink(root.join("tests/fuzz/minifuzz.lm"), dir.join("minifuzz.lm")).unwrap();
    let exe = tmp("minifuzz");
    build(&dir.join("minifuzz.lm"), &exe);
    run(&exe, 0, 3000);
    let _ = std::fs::remove_file(&exe);
    let _ = std::fs::remove_dir_all(&dir);
}
