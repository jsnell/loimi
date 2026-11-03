mod ast;
mod traits;
mod check;
mod closures;
mod codegen;
mod diag;
mod elf;
mod gcgen;
mod generics;
mod header;
mod jitgen;
mod layout;
mod lexer;
mod parser;
mod rtgen;
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
    /// `--sanitize=thread`: build with ThreadSanitizer.
    sanitize_thread: bool,
    /// `-W error` / `--deny-warnings`: warnings fail the build.
    deny_warnings: bool,
    /// `-W lint`: opt-in lints turned on.
    warn: Vec<String>,
    align_functions: Option<u32>,
    /// What to build: an executable, a static archive or a shared object.
    kind: OutKind,
    /// `--header out.h`: also write a C header for the `export "C"` functions.
    header: Option<PathBuf>,
    /// `--header-prefix p`: C names of structs and enums start with `p`.
    header_prefix: String,
    /// `--tls=`: the TLS model for runtime thread-locals (default:
    /// initial-exec, local-dynamic for `--shared`).
    tls: Option<Tls>,
    /// `--jit-stencils`: build the copy-and-patch JIT prototype in
    /// (docs/design/copy-and-patch.md); `LOIMI_JIT=1` turns it on at run time.
    jit: bool,
    /// `loimi test`: replace `main` with a runner of the `#[test]`
    /// functions (`test_main`), and run it.
    test: bool,
    /// `loimi test --update`: `test.golden` rewrites its files.
    update: bool,
    /// `--target=TRIPLE`: cross-compile (clang `--target`, linked with
    /// lld). None: the host.
    target: Option<Target>,
    /// `--sysroot=DIR` (or `LOIMI_SYSROOT`): the target's libc and crt files.
    sysroot: Option<PathBuf>,
}

/// A `--target` triple.
struct Target {
    triple: &'static str,
    arch: codegen::Arch,
}

/// The targets `--target` takes, by the names it accepts.
fn parse_target(s: &str) -> Option<Target> {
    use codegen::Arch;
    Some(match s {
        "aarch64-linux-gnu" | "aarch64-unknown-linux-gnu" | "aarch64" | "arm64" => {
            Target { triple: "aarch64-unknown-linux-gnu", arch: Arch::AArch64 }
        }
        "x86_64-linux-gnu" | "x86_64-unknown-linux-gnu" | "x86_64" | "x86-64" => {
            Target { triple: "x86_64-unknown-linux-gnu", arch: Arch::X86_64 }
        }
        _ => return None,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutKind {
    Exe,
    /// `--lib`: a static archive of position-independent code.
    Static,
    /// `--shared`: a shared object.
    Shared,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tls {
    InitialExec,
    LocalDynamic,
    GeneralDynamic,
}

const USAGE: &str = "\
usage: loimi build <file.lm> [-o out] [--release | -O] [--emit-llvm] [--dispatch=tailcall|goto|switch] [--vm-stats] [--align-functions=N]
                  [--sanitize=address|thread] [-W error | -Werror | --deny-warnings] [-W hot_gc]
                  [--lib | --shared] [--header out.h [--header-prefix p]] [--tls=initial-exec|local-dynamic|global-dynamic] [--jit-stencils]
                  [--keep-ll]   (keep the IR as <out>.ll, or <file>.ll without -o)
                  [--target=aarch64-linux-gnu|x86_64-linux-gnu [--sysroot=DIR]]   (cross-compile; links with lld)
       loimi run <file.lm> [build options] [-- args...]
                  (LOIMI_RUNNER=\"qemu-aarch64 -L DIR\" runs the program under that command)
       loimi test <file.lm> [build options] [--update] [--list] [FILTER...]";

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
/// `allows` collects the `#[allow(lint)]` regions of every file.
/// `test`: the root module gets a generated `main` (`test_main`).
fn load(sm: &mut SourceMap, root: &Path, allows: &mut Vec<(String, Span, Span)>, test: bool) -> Result<Vec<ast::Module>, Diag> {
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
        let (mut items, _attrs) = p.parse_module()?;
        allows.append(&mut p.allows);
        if test && path == ["main"] {
            test_main(sm, &mut items)?;
        }
        for it in &items {
            if let ast::Item::Import(imp) = it {
                let ipath: Vec<String> = imp.path.iter().map(|i| i.name.clone()).collect();
                let rel: PathBuf = ipath.iter().collect();
                let base = if ipath[0] == "std" { stdr.clone() } else { root_dir.clone() };
                queue.push((ipath, base.join(rel).with_extension("lm"), imp.span));
            }
        }
        modules.push(ast::Module { path, items });
    }
    Ok(modules)
}

/// `loimi test`: drop the root module's `main` and add one that runs
/// each `#[test]` function, in source order, through a `test.Runner`
/// (std/test.lm), importing `std.test` if the module does not.
fn test_main(sm: &mut SourceMap, items: &mut Vec<ast::Item>) -> Result<(), Diag> {
    let is_test = |f: &ast::FnDecl| f.attrs.has("test") && f.recv.is_none();
    let names: Vec<String> =
        items.iter().filter_map(|it| if let ast::Item::Fn(f) = it { is_test(f).then(|| f.name.name.clone()) } else { None }).collect();
    items.retain(|it| !matches!(it, ast::Item::Fn(f) if f.name.name == "main" && f.recv.is_none()));
    let imported = items.iter().any(|it| {
        matches!(it, ast::Item::Import(i) if i.names.is_none() && i.path.iter().map(|p| p.name.as_str()).eq(["std", "test"]))
    });
    let mut src = String::new();
    if !imported {
        src.push_str("import std.test;\n");
    }
    src.push_str("fn main() -> i32 uses reenter {\n    var runner = test.Runner.new();\n");
    for n in &names {
        src.push_str(&format!("    runner.run(\"{}\", {});\n", n, n));
    }
    src.push_str("    runner.finish()\n}\n");
    let fid = sm.add("<loimi test>".to_string(), src.clone());
    let toks = lexer::lex(fid, &src)?;
    let (generated, _) = parser::Parser::new(toks).parse_module()?;
    items.extend(generated);
    Ok(())
}

/// What the JIT build needs besides the program's IR: the stencil
/// functions, and the ops (opcode, name) and opcode count of the vm.
struct JitParts {
    stencils: String,
    ops: Vec<(u32, String)>,
    nops: u32,
}

/// Compile to LLVM IR, and the C header if `--header` asked for one.
fn compile(sm: &mut SourceMap, opts: &Opts) -> Result<(String, Option<String>, Option<JitParts>), Diag> {
    let input = opts.input.as_ref().unwrap();
    let mut allows = Vec::new();
    let mut modules = load(sm, input, &mut allows, opts.test)?;
    for (name, span, _) in &allows {
        if !diag::LINTS.contains(&name.as_str()) && !diag::RETIRED.contains(&name.as_str()) {
            return Err(Diag::new(*span, format!("unknown lint `{}` (known: {})", name, diag::LINTS.join(", "))));
        }
    }
    vmexpand::expand(sm, &mut modules, opts.jit)?;
    let mut c = check::Checker::new(&modules);
    c.dispatch_override = opts.dispatch;
    c.table_in_state = opts.target.as_ref().map_or(codegen::Arch::host(), |t| t.arch) == codegen::Arch::AArch64;
    c.run()?;
    // Warnings, minus those inside an `#[allow(lint)]` region.
    let allowed = |w: &Diag| {
        allows.iter().any(|(n, _, r)| {
            w.lint == Some(n.as_str()) && r.file == w.span.file && r.lo <= w.span.lo && w.span.hi <= r.hi
        })
    };
    // Opt-in lints only when turned on with `-W name`.
    let off = |w: &Diag| w.lint.is_some_and(|l| diag::OPT_IN.contains(&l) && !opts.warn.iter().any(|x| x == l));
    let warnings: Vec<&Diag> = c.warnings.iter().filter(|w| !allowed(w) && !off(w)).collect();
    for w in &warnings {
        eprint!("{}", sm.render_as(if opts.deny_warnings { "error" } else { "warning" }, w));
    }
    if opts.deny_warnings && !warnings.is_empty() {
        let n = warnings.len();
        return Err(Diag::new(
            Span { file: u32::MAX, lo: 0, hi: 0 },
            format!("{} warning{} treated as error{} (-W error)", n, if n == 1 { "" } else { "s" }, if n == 1 { "" } else { "s" }),
        ));
    }
    let prog = c.prog;
    let library = opts.kind != OutKind::Exe;
    if library {
        header::check_exports(&prog)?;
    }
    let header = match &opts.header {
        Some(path) => {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let guard: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
            let source = input.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            Some(header::generate(&prog, sm, &source, &guard, &opts.header_prefix)?)
        }
        None => None,
    };
    if !library && !opts.emit_llvm && !prog.fns.iter().any(|f| f.symbol == "main.main" && f.body.is_some()) {
        return Err(Diag::new(
            Span { file: u32::MAX, lo: 0, hi: 0 },
            "no `main` function (build a library with --lib or --shared)",
        ));
    }
    let jit = if opts.jit {
        let vi = jitgen::jit_vm(&prog).map_err(|e| Diag::new(Span { file: u32::MAX, lo: 0, hi: 0 }, e))?;
        let vd = &prog.vms[vi];
        let nops = 1u32 << vd.op_bits;
        let mut ops: Vec<(u32, String)> = vd.ops.iter().map(|o| (o.opcode, o.name.clone())).collect();
        // Stencil variants per constant tag follow the exit stencil.
        for (j, cv) in jitgen::const_variants(&prog, vi).into_iter().enumerate() {
            ops.push((nops + 1 + j as u32, cv.name));
        }
        Some((ops, nops))
    } else {
        None
    };
    let cg = codegen::Codegen::new(
        &prog,
        sm,
        codegen::Options {
            overflow_checks: !opts.release,
            vm_stats: opts.vm_stats,
            library,
            jit: opts.jit,
            arch: opts.target.as_ref().map_or(codegen::Arch::host(), |t| t.arch),
        },
    );
    let (mut ir, stencils) = cg.generate_with_stencils();
    let jit = jit.map(|(ops, nops)| JitParts { stencils, ops, nops });
    // Runtime thread-locals are emitted initial-exec: one offset from the
    // thread pointer, fixed at load time. A shared object that may be
    // dlopen'ed cannot rely on static TLS space, so it uses a dynamic
    // model (one `__tls_get_addr` call per function that touches TLS).
    let tls = opts.tls.unwrap_or(if opts.kind == OutKind::Shared { Tls::LocalDynamic } else { Tls::InitialExec });
    match tls {
        Tls::InitialExec => {}
        Tls::LocalDynamic => ir = ir.replace("thread_local(initialexec)", "thread_local(localdynamic)"),
        Tls::GeneralDynamic => ir = ir.replace("thread_local(initialexec)", "thread_local"),
    }
    // A cross build's IR names its target, so that `--emit-llvm` output
    // compiles alone. (Host builds leave it to clang's default.)
    if let Some(t) = &opts.target {
        ir.insert_str(0, &format!("target triple = \"{}\"\n", t.triple));
    }
    let ir = if opts.sanitize_address {
        sanitize(&ir, library, "sanitize_address")
    } else if opts.sanitize_thread {
        sanitize(&ir, library, "sanitize_thread")
    } else {
        ir
    };
    Ok((ir, header, jit))
}

/// `--jit-stencils`: compile the stencils to an object file, read them
/// back, and append them to the program's IR as data. Returns the C
/// runtime's path.
fn build_stencils(ir: &mut String, jit: &JitParts, clang: &str, tmpdir: &Path, stem: &str) -> Result<PathBuf, String> {
    let t0 = std::time::Instant::now();
    let sll = tmpdir.join(format!("{}.stencils.ll", stem));
    let so = tmpdir.join(format!("{}.stencils.o", stem));
    std::fs::write(&sll, jitgen::stencil_module(ir, &jit.stencils)).map_err(|e| e.to_string())?;
    let st = Command::new(clang)
        .args(["-O3", "-Wno-override-module", "-fno-pic", "-ffunction-sections", "-fdata-sections", "-fno-asynchronous-unwind-tables", "-c", "-x", "ir"])
        .args(if jitgen::split_stencils() { &["-fsplit-machine-functions"][..] } else { &[] })
        .arg(&sll)
        .arg("-o")
        .arg(&so)
        .status()
        .map_err(|e| format!("cannot run {}: {}", clang, e))?;
    if !st.success() {
        return Err(format!("{} failed on the stencil module (kept at {})", clang, sll.display()));
    }
    let obj = std::fs::read(&so).map_err(|e| e.to_string())?;
    let t = elf::stencil_tables(obj, &jit.ops, jit.nops, ir)?;
    ir.push_str(&t.ir);
    if std::env::var_os("LOIMI_JIT_REPORT").is_some() {
        eprintln!(
            "jit-stencils: {} stencils, {} bytes of code, {} helpers ({} bytes), built in {:.2} s",
            t.count,
            t.bytes,
            t.helpers,
            t.helper_bytes,
            t0.elapsed().as_secs_f64()
        );
        for (op, why) in &t.rejected {
            eprintln!("  no stencil for {}: {}", op, why);
        }
    }
    // LOIMI_KEEP_STENCILS=dir: keep the stencil module and object there.
    if let Some(d) = std::env::var_os("LOIMI_KEEP_STENCILS") {
        let _ = std::fs::copy(&sll, Path::new(&d).join(format!("{}.stencils.ll", stem)));
        let _ = std::fs::copy(&so, Path::new(&d).join(format!("{}.stencils.o", stem)));
    }
    let _ = std::fs::remove_file(&sll);
    let _ = std::fs::remove_file(&so);
    let rt = tmpdir.join("loimi_jit_rt.c");
    std::fs::write(&rt, include_str!("jit_rt.c")).map_err(|e| e.to_string())?;
    Ok(rt)
}

/// The sanitizer runtime (`asan` or `tsan`) to link explicitly, if
/// clang's own is missing. `Ok(None)`: clang links its runtime itself.
/// Otherwise GCC's shared libasan/libtsan (same instrumentation ABI), or
/// `LOIMI_ASAN_RUNTIME` / `LOIMI_TSAN_RUNTIME`.
fn sanitizer_runtime(clang: &str, san: &str) -> Result<Option<PathBuf>, String> {
    if let Ok(p) = std::env::var(format!("LOIMI_{}_RUNTIME", san.to_uppercase())) {
        return Ok(Some(PathBuf::from(p)));
    }
    let query = |prog: &str, arg: &str| -> Option<PathBuf> {
        let out = Command::new(prog).arg(arg).output().ok()?;
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() && !s.is_empty() { Some(PathBuf::from(s)) } else { None }
    };
    if let Some(dir) = query(clang, "-print-runtime-dir") {
        if dir.join(format!("libclang_rt.{}.a", san)).exists() {
            return Ok(None);
        }
    }
    if let Some(p) = query("gcc", &format!("-print-file-name=lib{}.so", san)) {
        if p.is_absolute() && p.exists() {
            return Ok(Some(p));
        }
    }
    let flag = if san == "asan" { "address" } else { "thread" };
    Err(format!(
        "--sanitize={}: no {} runtime found (install libclang-rt-19-dev or GCC's lib{}, \
         or set LOIMI_{}_RUNTIME to a lib{}.so)",
        flag,
        san,
        san,
        san.to_uppercase(),
        san
    ))
}

/// ASan and TSan instrument only functions that carry the
/// `sanitize_address` / `sanitize_thread` attribute (`attr`). Clang adds
/// it to C functions, but IR input has none, so mark every definition.
/// In ASan executables leak checking is off by default: programs leave
/// their heaps to the OS (ASAN_OPTIONS=detect_leaks=1 turns it on).
/// Libraries leave the sanitizer's options to the host.
fn sanitize(ir: &str, library: bool, attr: &str) -> String {
    let mut out = String::with_capacity(ir.len() + ir.len() / 16);
    if !library && attr == "sanitize_address" {
        out.push_str(
            "@loimi.asan_options = private constant [15 x i8] c\"detect_leaks=0\\00\"\n\
             define ptr @__asan_default_options() {\n  ret ptr @loimi.asan_options\n}\n",
        );
    }
    for line in ir.lines() {
        match line.strip_suffix('{') {
            Some(head) if line.starts_with("define ") => {
                out.push_str(head);
                out.push_str(attr);
                out.push_str(" {");
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
    if matches!(cmd.as_str(), "--help" | "-h" | "help") {
        println!("{}", USAGE);
        std::process::exit(0);
    }
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
        sanitize_thread: false,
        deny_warnings: false,
        warn: vec![],
        align_functions: None,
        kind: OutKind::Exe,
        header: None,
        header_prefix: String::new(),
        tls: None,
        jit: false,
        test: false,
        update: false,
        // `LOIMI_TARGET`: a default for `--target` (for running whole test
        // suites cross-compiled).
        target: match std::env::var("LOIMI_TARGET") {
            Ok(t) if !t.is_empty() => Some(parse_target(&t).ok_or_else(|| format!("LOIMI_TARGET: unknown target `{}`", t))?),
            _ => None,
        },
        sysroot: std::env::var_os("LOIMI_SYSROOT").filter(|s| !s.is_empty()).map(PathBuf::from),
    };
    match cmd.as_str() {
        "build" => {}
        "run" => o.run = true,
        "test" => {
            o.run = true;
            o.test = true;
        }
        _ => return Err(USAGE.into()),
    }
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => o.output = Some(PathBuf::from(args.next().ok_or("-o needs a path")?)),
            "--release" | "-O" => o.release = true,
            "--emit-llvm" => o.emit_llvm = true,
            "--keep-ll" => o.keep_ll = true,
            "--lib" => o.kind = OutKind::Static,
            "--shared" => o.kind = OutKind::Shared,
            "--header" => o.header = Some(PathBuf::from(args.next().ok_or("--header needs a path")?)),
            "--header-prefix" => o.header_prefix = args.next().ok_or("--header-prefix needs a prefix")?,
            s if s.starts_with("--tls=") => {
                o.tls = Some(match &s["--tls=".len()..] {
                    "initial-exec" => Tls::InitialExec,
                    "local-dynamic" => Tls::LocalDynamic,
                    "global-dynamic" => Tls::GeneralDynamic,
                    m => return Err(format!("unknown TLS model `{}` (initial-exec, local-dynamic, global-dynamic)", m)),
                })
            }
            s if s.starts_with("--target=") => {
                let t = &s["--target=".len()..];
                o.target = Some(parse_target(t).ok_or_else(|| {
                    format!("unknown target `{}` (supported: aarch64-linux-gnu, x86_64-linux-gnu)", t)
                })?)
            }
            s if s.starts_with("--sysroot=") => o.sysroot = Some(PathBuf::from(&s["--sysroot=".len()..])),
            "--vm-stats" => o.vm_stats = true,
            "--jit-stencils" => o.jit = true,
            "--update" if o.test => o.update = true,
            "--list" if o.test => o.run_args.push(a.clone()),
            "--sanitize=address" => o.sanitize_address = true,
            "--sanitize=thread" => o.sanitize_thread = true,
            "--deny-warnings" | "-Werror" => o.deny_warnings = true,
            "-W" => match args.next().as_deref() {
                Some("error") => o.deny_warnings = true,
                Some(l) if diag::OPT_IN.contains(&l) => o.warn.push(l.to_string()),
                _ => return Err(format!("-W takes `error` or an opt-in lint ({})", diag::OPT_IN.join(", "))),
            },
            s if s.starts_with("--align-functions=") => {
                let n: u32 = s["--align-functions=".len()..].parse().map_err(|_| "--align-functions takes a number")?;
                if !n.is_power_of_two() || n > 4096 {
                    return Err("--align-functions takes a power of two up to 4096".into());
                }
                o.align_functions = Some(n);
            }
            s if s.starts_with("--sanitize=") => {
                return Err(format!("unknown sanitizer `{}` (supported: address, thread)", &s["--sanitize=".len()..]))
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
    if o.jit && (o.kind != OutKind::Exe || o.sanitize_address || o.sanitize_thread || o.dispatch.is_some_and(|d| d != tir::Dispatch::TailCall)) {
        return Err("--jit-stencils builds executables with tail-call dispatch, without sanitizers".into());
    }
    if o.jit && !(cfg!(target_arch = "x86_64") && cfg!(target_os = "linux")) {
        return Err("--jit-stencils supports x86-64 Linux only".into());
    }
    if o.jit && o.target.as_ref().is_some_and(|t| t.arch != codegen::Arch::X86_64) {
        // The stencils are x86-64 machine code patched in place
        // (src/elf.rs reads x86-64 relocations).
        return Err("--jit-stencils supports x86-64 Linux only (not --target=aarch64)".into());
    }
    if o.sanitize_address && o.sanitize_thread {
        return Err("--sanitize=address and --sanitize=thread cannot be combined".into());
    }
    if (o.sanitize_address || o.sanitize_thread) && o.target.is_some() {
        return Err("--sanitize is not supported with --target".into());
    }
    if o.kind != OutKind::Exe {
        if o.run {
            return Err("`loimi run` builds executables; --lib and --shared need `loimi build`".into());
        }
    }
    Ok(o)
}

/// The checker and code generator recurse over expressions; large
/// programs (MiniJS) overflow the main thread's 8 MiB stack in debug
/// builds. Run the compiler on a thread with a larger one.
fn main() -> ExitCode {
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(compiler_main)
        .expect("cannot start the compiler thread")
        .join()
        .unwrap_or(ExitCode::from(101))
}

fn compiler_main() -> ExitCode {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(2);
        }
    };
    let mut sm = SourceMap::default();
    let (mut ir, header, jit) = match compile(&mut sm, &opts) {
        Ok(r) => r,
        Err(d) => {
            eprint!("{}", sm.render(&d));
            return ExitCode::from(1);
        }
    };
    let input = opts.input.as_ref().unwrap();
    let stem = input.file_stem().unwrap().to_string_lossy().to_string();
    if let (Some(path), Some(text)) = (&opts.header, &header) {
        if let Err(e) = std::fs::write(path, text) {
            eprintln!("error: cannot write {}: {}", path.display(), e);
            return ExitCode::from(1);
        }
    }
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
    let clang = std::env::var("LOIMI_CLANG").unwrap_or_else(|_| "clang-19".into());
    let jit_rt = match &jit {
        Some(j) => match build_stencils(&mut ir, j, &clang, &tmpdir, &stem) {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("error: --jit-stencils: {}", e);
                return ExitCode::from(1);
            }
        },
        None => None,
    };
    // `--vm-stats`: the runtime for sequences longer than pairs.
    let stats_rt = if opts.vm_stats {
        let p = tmpdir.join("loimi_stats_rt.c");
        if let Err(e) = std::fs::write(&p, vmgen::stats_rt_source()) {
            eprintln!("error: cannot write {}: {}", p.display(), e);
            return ExitCode::from(1);
        }
        Some(p)
    } else {
        None
    };
    let ll = tmpdir.join(format!("{}.ll", stem));
    if let Err(e) = std::fs::write(&ll, &ir) {
        eprintln!("error: cannot write {}: {}", ll.display(), e);
        return ExitCode::from(1);
    }
    let exe = opts.output.clone().unwrap_or_else(|| match opts.kind {
        OutKind::Exe if opts.run => tmpdir.join(&stem),
        OutKind::Exe => PathBuf::from(&stem),
        OutKind::Static => PathBuf::from(format!("lib{}.a", stem)),
        OutKind::Shared => PathBuf::from(format!("lib{}.so", stem)),
    });
    // Cross builds: clang's `--target`, the sysroot's libc, and lld (the
    // host's linker does not know the target).
    let mut cross: Vec<String> = Vec::new();
    if let Some(t) = &opts.target {
        cross.push(format!("--target={}", t.triple));
        cross.push("-fuse-ld=lld".into());
    }
    if let Some(s) = &opts.sysroot {
        cross.push(format!("--sysroot={}", s.display()));
    }
    let mut cmd = Command::new(&clang);
    cmd.arg(if opts.release { "-O3" } else { "-O1" }).arg("-Wno-override-module").args(&cross);
    // The object goes into the archive.
    let obj = tmpdir.join(format!("{}.o", stem));
    match opts.kind {
        // Executables: absolute addressing lets dispatch index the table directly.
        OutKind::Exe => {
            cmd.arg("-fno-pic").arg("-no-pie");
        }
        // Libraries: position-independent, so the archive links into PIE
        // executables (the default on most systems) and shared objects.
        OutKind::Static => {
            cmd.arg("-fPIC").arg("-c");
        }
        OutKind::Shared => {
            cmd.arg("-fPIC").arg("-shared");
        }
    }
    if let Some(n) = opts.align_functions {
        // Every function (vm handlers included) starts on an n-byte boundary,
        // so unrelated code changes do not shift handler placement. Useful for
        // stable before/after benchmarking.
        cmd.arg("-mllvm").arg(format!("-align-all-functions={}", n.trailing_zeros()));
    }
    // Libraries carry the instrumentation only: the host links the ASan
    // or TSan runtime (docs/language.md §10.1).
    let san = if opts.sanitize_address {
        Some("asan")
    } else if opts.sanitize_thread {
        Some("tsan")
    } else {
        None
    };
    let san_lib = if let (Some(san), OutKind::Exe) = (san, opts.kind) {
        match sanitizer_runtime(&clang, san) {
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
    }
    if opts.sanitize_thread {
        cmd.arg("-fsanitize=thread").arg("-fno-omit-frame-pointer");
    }
    if san.is_some() && (san_lib.is_some() || opts.kind != OutKind::Exe) {
        cmd.arg("-fno-sanitize-link-runtime");
    }
    cmd.arg("-x").arg("ir").arg(&ll);
    // C runtime sources. A static library compiles each to its own
    // object (`-c` takes one input per `-o`), all archived together.
    let mut extra_objs = Vec::new();
    for rt in jit_rt.iter().chain(stats_rt.iter()) {
        if opts.kind == OutKind::Static {
            let o = rt.with_extension("o");
            let ok = Command::new(&clang)
                .args([if opts.release { "-O3" } else { "-O1" }, "-fPIC", "-c", "-x", "c"])
                .args(&cross)
                .arg(rt)
                .arg("-o")
                .arg(&o)
                .status()
                .is_ok_and(|s| s.success());
            if !ok {
                eprintln!("error: {} failed on {}", clang, rt.display());
                return ExitCode::from(1);
            }
            extra_objs.push(o);
        } else {
            cmd.arg("-x").arg("c").arg(rt);
        }
    }
    if let Some(lib) = &san_lib {
        cmd.arg("-x").arg("none").arg(lib);
    }
    let target = if opts.kind == OutKind::Static { &obj } else { &exe };
    cmd.arg("-o").arg(target);
    if opts.kind != OutKind::Static {
        cmd.arg("-lm");
    }
    let status = cmd.status();
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
    if opts.kind == OutKind::Static {
        let ar = std::env::var("LOIMI_AR").unwrap_or_else(|_| "llvm-ar-19".into());
        let _ = std::fs::remove_file(&exe);
        match Command::new(&ar).arg("rcs").arg(&exe).arg(&obj).args(&extra_objs).status() {
            Ok(s) if s.success() => {}
            Ok(_) => {
                eprintln!("error: {} failed", ar);
                return ExitCode::from(1);
            }
            Err(e) => {
                eprintln!("error: cannot run {}: {} (set LOIMI_AR)", ar, e);
                return ExitCode::from(1);
            }
        }
        let _ = std::fs::remove_file(&obj);
    }
    if opts.keep_ll {
        // The temporary directory goes away below: keep the IR next to the
        // output (`-o dir/x` keeps `dir/x.ll`), else as `<stem>.ll` in the
        // current directory, like `--emit-llvm`.
        let kept = match &opts.output {
            Some(o) => {
                let mut s = o.clone().into_os_string();
                s.push(".ll");
                PathBuf::from(s)
            }
            None => PathBuf::from(format!("{}.ll", stem)),
        };
        if let Err(e) = std::fs::copy(&ll, &kept) {
            eprintln!("error: cannot write {}: {}", kept.display(), e);
            let _ = std::fs::remove_dir_all(&tmpdir);
            return ExitCode::from(1);
        }
        eprintln!("IR kept at {}", kept.display());
    }
    let _ = std::fs::remove_file(&ll);
    if opts.run {
        // `LOIMI_RUNNER="qemu-aarch64 -L sysroot"`: run the program under
        // an emulator (or any wrapper), split at whitespace.
        let runner: Vec<String> =
            std::env::var("LOIMI_RUNNER").unwrap_or_default().split_whitespace().map(String::from).collect();
        let mut cmd = match runner.split_first() {
            Some((prog, rest)) => {
                let mut c = Command::new(prog);
                c.args(rest).arg(&exe);
                c
            }
            None => Command::new(&exe),
        };
        cmd.args(&opts.run_args);
        if opts.test {
            // Tests run in their file's directory, so that `test.golden`
            // paths are relative to it.
            if let Some(dir) = input.parent().filter(|d| !d.as_os_str().is_empty()) {
                cmd.current_dir(dir);
            }
            if opts.update {
                cmd.env("LOIMI_TEST_UPDATE", "1");
            }
        }
        let status = cmd.status();
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
