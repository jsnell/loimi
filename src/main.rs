mod ast;
mod traits;
mod check;
mod codegen;
mod diag;
mod gcgen;
mod generics;
mod layout;
mod lexer;
mod parser;
mod tir;
mod tirwalk;
mod values;
mod vmast;
mod vmcheck;
mod vmexpand;
mod vmgen;
mod vmparse;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use diag::{Diag, SourceMap, Span};

struct Opts {
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    emit_llvm: bool,
    release: bool,
    run: bool,
    run_args: Vec<String>,
    dispatch: Option<tir::Dispatch>,
    keep_ll: bool,
    vm_stats: bool,
    /// `--sanitize=address`: build with AddressSanitizer.
    sanitize_address: bool,
}

const USAGE: &str = "\
usage: loimi build <file.lm> [-o out] [--release] [--emit-llvm] [--dispatch=tailcall|goto|switch] [--vm-stats]
                  [--sanitize=address]
       loimi run <file.lm> [--release] [--dispatch=...] [-- args...]";

fn std_root() -> PathBuf {
    if let Ok(p) = std::env::var("LOIMI_STD") {
        return PathBuf::from(p);
    }
    // Next to the executable's project (development layout), else cwd.
    let exe = std::env::current_exe().ok();
    if let Some(mut d) = exe.and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        for _ in 0..4 {
            if d.join("std").is_dir() {
                return d;
            }
            if !d.pop() {
                break;
            }
        }
    }
    PathBuf::from(".")
}

/// Load the root module and everything it imports.
fn load(sm: &mut SourceMap, root: &Path) -> Result<Vec<ast::Module>, Diag> {
    let root_dir = root.parent().unwrap_or(Path::new(".")).to_path_buf();
    let stdr = std_root();
    let mut modules: Vec<ast::Module> = Vec::new();
    let mut queue: Vec<(Vec<String>, PathBuf, Span)> = vec![(vec!["main".into()], root.to_path_buf(), Span::default())];
    while let Some((path, file, from)) = queue.pop() {
        if modules.iter().any(|m| m.path == path) {
            continue;
        }
        let text = std::fs::read_to_string(&file)
            .map_err(|e| Diag::new(from, format!("cannot read {}: {}", file.display(), e)))?;
        let fid = sm.add(file.display().to_string(), text.clone());
        let toks = lexer::lex(fid, &text)?;
        let mut p = parser::Parser::new(toks);
        let (items, _attrs) = p.parse_module()?;
        for it in &items {
            if let ast::Item::Import(imp) = it {
                let ipath: Vec<String> = imp.path.iter().map(|i| i.name.clone()).collect();
                let rel: PathBuf = ipath.iter().collect();
                let base = if ipath[0] == "std" { stdr.clone() } else { root_dir.clone() };
                queue.push((ipath, base.join(rel).with_extension("lm"), imp.span));
            }
        }
        modules.push(ast::Module { path, file: fid, items });
    }
    Ok(modules)
}

fn compile(sm: &mut SourceMap, opts: &Opts) -> Result<String, Diag> {
    let input = opts.input.as_ref().unwrap();
    let mut modules = load(sm, input)?;
    vmexpand::expand(sm, &mut modules)?;
    let mut c = check::Checker::new(&modules);
    c.dispatch_override = opts.dispatch;
    c.run()?;
    let prog = c.prog;
    let cg = codegen::Codegen::new(&prog, sm, codegen::Options { overflow_checks: !opts.release, vm_stats: opts.vm_stats });
    let ir = cg.generate();
    Ok(if opts.sanitize_address { sanitize_address(&ir) } else { ir })
}

/// The ASan runtime to link explicitly, if clang's own is missing.
/// `Ok(None)`: clang links its runtime itself. Otherwise GCC's shared
/// libasan (same instrumentation ABI), or `LOIMI_ASAN_RUNTIME`.
fn asan_runtime(clang: &str) -> Result<Option<PathBuf>, String> {
    if let Ok(p) = std::env::var("LOIMI_ASAN_RUNTIME") {
        return Ok(Some(PathBuf::from(p)));
    }
    let query = |prog: &str, arg: &str| -> Option<PathBuf> {
        let out = Command::new(prog).arg(arg).output().ok()?;
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() && !s.is_empty() { Some(PathBuf::from(s)) } else { None }
    };
    if let Some(dir) = query(clang, "-print-runtime-dir") {
        if dir.join("libclang_rt.asan.a").exists() {
            return Ok(None);
        }
    }
    if let Some(p) = query("gcc", "-print-file-name=libasan.so") {
        if p.is_absolute() && p.exists() {
            return Ok(Some(p));
        }
    }
    Err("--sanitize=address: no ASan runtime found (install libclang-rt-19-dev or GCC's libasan, \
         or set LOIMI_ASAN_RUNTIME to a libasan.so)"
        .to_string())
}

/// ASan instruments only functions that carry the `sanitize_address`
/// attribute. Clang adds it to C functions, but IR input has none, so
/// mark every definition. Leak checking is off by default: programs
/// leave their heaps to the OS (ASAN_OPTIONS=detect_leaks=1 turns it on).
fn sanitize_address(ir: &str) -> String {
    let mut out = String::with_capacity(ir.len() + ir.len() / 16);
    out.push_str(
        "@loimi.asan_options = private constant [15 x i8] c\"detect_leaks=0\\00\"\n\
         define ptr @__asan_default_options() {\n  ret ptr @loimi.asan_options\n}\n",
    );
    for line in ir.lines() {
        match line.strip_suffix('{') {
            Some(head) if line.starts_with("define ") => {
                out.push_str(head);
                out.push_str("sanitize_address {");
            }
            _ => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

fn parse_args() -> Result<Opts, String> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().ok_or(USAGE)?;
    let mut o = Opts {
        input: None,
        output: None,
        emit_llvm: false,
        release: false,
        run: false,
        run_args: vec![],
        dispatch: None,
        keep_ll: false,
        vm_stats: false,
        sanitize_address: false,
    };
    match cmd.as_str() {
        "build" => {}
        "run" => o.run = true,
        _ => return Err(USAGE.into()),
    }
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => o.output = Some(PathBuf::from(args.next().ok_or("-o needs a path")?)),
            "--release" | "-O" => o.release = true,
            "--emit-llvm" => o.emit_llvm = true,
            "--keep-ll" => o.keep_ll = true,
            "--vm-stats" => o.vm_stats = true,
            "--sanitize=address" => o.sanitize_address = true,
            s if s.starts_with("--sanitize=") => {
                return Err(format!("unknown sanitizer `{}` (supported: address)", &s["--sanitize=".len()..]))
            }
            "--" => {
                o.run_args = args.by_ref().collect();
            }
            s if s.starts_with("--dispatch=") => {
                o.dispatch = Some(match &s["--dispatch=".len()..] {
                    "tailcall" => tir::Dispatch::TailCall,
                    "goto" => tir::Dispatch::Goto,
                    "switch" => tir::Dispatch::Switch,
                    d => return Err(format!("unknown dispatch mode `{}`", d)),
                })
            }
            s if s.starts_with('-') => return Err(format!("unknown option `{}`", s)),
            s => {
                if o.input.is_some() {
                    o.run_args.push(s.to_string());
                } else {
                    o.input = Some(PathBuf::from(s));
                }
            }
        }
    }
    if o.input.is_none() {
        return Err(USAGE.into());
    }
    Ok(o)
}

fn main() -> ExitCode {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(2);
        }
    };
    let mut sm = SourceMap::default();
    let ir = match compile(&mut sm, &opts) {
        Ok(ir) => ir,
        Err(d) => {
            eprint!("{}", sm.render(&d));
            return ExitCode::from(1);
        }
    };
    let input = opts.input.as_ref().unwrap();
    let stem = input.file_stem().unwrap().to_string_lossy().to_string();
    if opts.emit_llvm {
        let out = opts.output.clone().unwrap_or_else(|| PathBuf::from(format!("{}.ll", stem)));
        if out.as_os_str() == "-" {
            print!("{}", ir);
        } else if let Err(e) = std::fs::write(&out, ir) {
            eprintln!("error: cannot write {}: {}", out.display(), e);
            return ExitCode::from(1);
        }
        return ExitCode::SUCCESS;
    }
    let tmpdir = std::env::temp_dir().join(format!("loimi-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmpdir);
    let ll = tmpdir.join(format!("{}.ll", stem));
    if let Err(e) = std::fs::write(&ll, &ir) {
        eprintln!("error: cannot write {}: {}", ll.display(), e);
        return ExitCode::from(1);
    }
    let exe = opts.output.clone().unwrap_or_else(|| {
        if opts.run { tmpdir.join(&stem) } else { PathBuf::from(&stem) }
    });
    let clang = std::env::var("LOIMI_CLANG").unwrap_or_else(|_| "clang-19".into());
    let mut cmd = Command::new(&clang);
    cmd.arg(if opts.release { "-O3" } else { "-O1" })
        .arg("-Wno-override-module")
        // Executables: absolute addressing lets dispatch index the table directly.
        .arg("-fno-pic")
        .arg("-no-pie");
    let asan_lib = if opts.sanitize_address {
        match asan_runtime(&clang) {
            Ok(lib) => lib,
            Err(e) => {
                eprintln!("error: {}", e);
                return ExitCode::from(1);
            }
        }
    } else {
        None
    };
    if opts.sanitize_address {
        // Use-after-return detection puts frames on a fake stack with a
        // dynamic-alloca fallback, which LLVM cannot combine with stack
        // realignment in `preserve_none` handlers (tail-call dispatch).
        cmd.arg("-fsanitize=address")
            .arg("-fsanitize-address-use-after-return=never")
            .arg("-fno-omit-frame-pointer");
        if asan_lib.is_some() {
            cmd.arg("-fno-sanitize-link-runtime");
        }
    }
    cmd.arg("-x").arg("ir").arg(&ll);
    if let Some(lib) = &asan_lib {
        cmd.arg("-x").arg("none").arg(lib);
    }
    let status = cmd.arg("-o").arg(&exe).arg("-lm").status();
    match status {
        Ok(s) if s.success() => {}
        Ok(_) => {
            eprintln!("error: {} failed on generated IR (kept at {})", clang, ll.display());
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("error: cannot run {}: {}", clang, e);
            return ExitCode::from(1);
        }
    }
    if opts.keep_ll {
        eprintln!("IR kept at {}", ll.display());
    } else {
        let _ = std::fs::remove_file(&ll);
    }
    if opts.run {
        let status = Command::new(&exe).args(&opts.run_args).status();
        let _ = std::fs::remove_dir_all(&tmpdir);
        return match status {
            Ok(s) => match s.code() {
                Some(c) => ExitCode::from(c as u8),
                None => {
                    eprintln!("program terminated by a signal");
                    ExitCode::from(134)
                }
            },
            Err(e) => {
                eprintln!("error: cannot run program: {}", e);
                ExitCode::from(1)
            }
        };
    }
    let _ = std::fs::remove_dir_all(&tmpdir);
    ExitCode::SUCCESS
}
