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

// ── GC rooting fuzzer ───────────────────────────────────────────────────

#[path = "fuzz/rootgen.rs"]
mod rootgen;

/// Where `fuzz_gc_roots*` keep generated programs and binaries.
fn rootfuzz_dir() -> PathBuf {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/rootfuzz");
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Generate the program of `seed`, build it with `--gc-check-roots` in
/// each of `modes` (dispatch modes), and run each build without
/// collections, under GC stress and under generational stress with
/// verify. Every run must exit 0 with the same output as the first, and
/// report no leaks. Returns a description of the failure.
fn gc_roots_seed(seed: u64, modes: &[&str]) -> Result<(), String> {
    let dir = rootfuzz_dir();
    let src = dir.join(format!("roots-{}.lm", seed));
    std::fs::write(&src, rootgen::generate(seed)).unwrap();
    let mut reference: Option<String> = None;
    for m in modes {
        let exe = dir.join(format!("roots-{}-{}", seed, m));
        let out = limited(env!("CARGO_BIN_EXE_loimi"))
            .env("LOIMI_STD", env!("CARGO_MANIFEST_DIR"))
            .args(["build", &format!("--dispatch={}", m)])
            // ROOTFUZZ_PLAIN: without the checks (to compare what they add).
            .args(if std::env::var("ROOTFUZZ_PLAIN").is_ok() { &[][..] } else { &["--gc-check-roots"][..] })
            .arg(&src)
            .arg("-o")
            .arg(&exe)
            .output()
            .unwrap();
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("seed {}: does not compile ({}):\n{}", seed, m, err));
        }
        for (gc, args) in [("no collections", &[][..]), ("stress", &["s"][..]), ("generational stress, verify", &["s", "g"][..])] {
            let out = limited(&exe).args(args).output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let what = format!("seed {} ({}, {})", seed, m, gc);
            if !out.status.success() {
                return Err(format!("{}: {:?}\n{}", what, out.status, String::from_utf8_lossy(&out.stderr)));
            }
            if !stdout.lines().any(|l| l == "leaks 0") {
                return Err(format!("{}: an object only out-of-scope locals referenced survived a collection\n{}", what, stdout));
            }
            match &reference {
                None => reference = Some(stdout),
                Some(r) if *r != stdout => {
                    let diff: Vec<String> = r.lines().zip(stdout.lines()).filter(|(a, b)| a != b).take(3).map(|(a, b)| format!("  {} / {}", a, b)).collect();
                    return Err(format!("{}: output differs from the first run:\n{}", what, diff.join("\n")));
                }
                _ => {}
            }
        }
        let _ = std::fs::remove_file(&exe);
    }
    if std::env::var("ROOTFUZZ_KEEP").is_err() {
        let _ = std::fs::remove_file(&src);
    }
    Ok(())
}

/// Smoke run of the rooting fuzzer (tests/fuzz/rootgen.rs): fixed seeds,
/// one dispatch mode each, in turn. Longer runs: `fuzz_gc_roots_long`.
#[test]
fn fuzz_gc_roots() {
    let modes = ["tailcall", "goto", "switch"];
    let mut failures = Vec::new();
    for seed in 0..18u64 {
        let m = seed as usize % 3;
        if let Err(e) = gc_roots_seed(seed, &modes[m..m + 1]) {
            failures.push(e);
        }
    }
    assert!(failures.is_empty(), "{}\nprograms in target/rootfuzz/roots-SEED.lm", failures.join("\n\n"));
}

/// The rooting fuzzer for a while, every seed in all three dispatch modes:
///
///     ROOTFUZZ_SECS=300 ROOTFUZZ_JOBS=2 ROOTFUZZ_FIRST=1000 \
///         cargo test --release --test fuzz fuzz_gc_roots_long -- --ignored --nocapture
///
/// (`ROOTFUZZ_SEEDS=a,b,c` runs just those seeds.) Failing programs stay
/// in target/rootfuzz/roots-SEED.lm.
#[test]
#[ignore]
fn fuzz_gc_roots_long() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    let var = |k: &str| std::env::var(k).ok();
    let secs: u64 = var("ROOTFUZZ_SECS").and_then(|s| s.parse().ok()).unwrap_or(120);
    let jobs: u64 = var("ROOTFUZZ_JOBS").and_then(|s| s.parse().ok()).unwrap_or(2);
    let first: u64 = var("ROOTFUZZ_FIRST").and_then(|s| s.parse().ok()).unwrap_or_else(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() % 1_000_000 * 1000
    });
    let list: Option<Vec<u64>> = var("ROOTFUZZ_SEEDS").map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect());
    let start = std::time::Instant::now();
    let next = Arc::new(AtomicU64::new(0));
    let ran = Arc::new(AtomicU64::new(0));
    let failures = Arc::new(Mutex::new(Vec::new()));
    let threads: Vec<_> = (0..jobs)
        .map(|_| {
            let (next, failures, ran, list) = (next.clone(), failures.clone(), ran.clone(), list.clone());
            std::thread::spawn(move || loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let seed = match &list {
                    Some(l) => match l.get(i as usize) {
                        Some(s) => *s,
                        None => break,
                    },
                    None if start.elapsed().as_secs() >= secs => break,
                    None => first + i,
                };
                ran.fetch_add(1, Ordering::SeqCst);
                if let Err(e) = gc_roots_seed(seed, &["tailcall", "goto", "switch"]) {
                    eprintln!("FAIL {}", e);
                    failures.lock().unwrap().push(seed);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let f = failures.lock().unwrap();
    eprintln!("rootfuzz: {} seeds from {} in {}s, failing: {:?}", ran.load(Ordering::SeqCst), first, start.elapsed().as_secs(), *f);
    assert!(f.is_empty());
}
