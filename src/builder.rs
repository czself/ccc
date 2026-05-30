use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone)]
pub enum Compiler {
    System { name: String },
    Bundled { path: PathBuf },
}

pub struct BuildResult {
    pub success: bool,
    pub output: String,
    pub command_line: String,
}

pub struct CompilerInfo {
    pub cc: Option<Compiler>,
    pub cxx: Option<Compiler>,
    pub problem: Option<String>,
}

fn probe_system_compiler(candidates: &[&str]) -> Option<Compiler> {
    for name in candidates {
        let mut cmd = Command::new(name);
        if name == &"cl.exe" { cmd.arg("/?"); } else { cmd.arg("--version"); }
        if cmd.output().is_ok() {
            return Some(Compiler::System { name: name.to_string() });
        }
    }
    None
}

fn cache_dir() -> PathBuf {
    let base = if cfg!(target_os = "windows") {
        std::env::var("LOCALAPPDATA")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string())
    } else {
        std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string())
    };
    let sub = if cfg!(target_os = "windows") { "tinyvim" } else { ".cache/tinyvim" };
    PathBuf::from(base).join(sub)
}

fn download_file(url: &str, dest: &Path) -> io::Result<()> {
    let resp = ureq::get(url).call()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("HTTP: {}", e)))?;
    let mut body: Vec<u8> = Vec::new();
    resp.into_body().into_reader().read_to_end(&mut body)?;
    std::fs::write(dest, &body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755)).ok();
    }
    Ok(())
}

fn download_tcc() -> Option<PathBuf> {
    let cache = cache_dir();
    let (url, exe_name) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64")   => ("https://github.com/sz3/tinycc/releases/download/v0.9.27/tcc-x86_64-linux", "tcc"),
        ("linux", "aarch64")  => ("https://github.com/sz3/tinycc/releases/download/v0.9.27/tcc-aarch64-linux", "tcc"),
        ("macos", "x86_64")   => ("https://github.com/sz3/tinycc/releases/download/v0.9.27/tcc-x86_64-macos", "tcc"),
        ("macos", "aarch64")  => ("https://github.com/sz3/tinycc/releases/download/v0.9.27/tcc-aarch64-macos", "tcc"),
        ("windows", "x86_64") => ("https://github.com/sz3/tinycc/releases/download/v0.9.27/tcc-x86_64-win32.exe", "tcc.exe"),
        _ => return None,
    };

    std::fs::create_dir_all(&cache).ok()?;
    let exe_path = cache.join(exe_name);
    if exe_path.exists() { return Some(exe_path); }
    download_file(url, &exe_path).ok()?;
    Some(exe_path)
}

fn download_mingw() -> Option<PathBuf> {
    let cache = cache_dir();
    let mingw_root = cache.join("w64devkit");
    let gpp_path = mingw_root.join("bin").join("g++.exe");

    if gpp_path.exists() {
        return Some(gpp_path);
    }

    std::fs::create_dir_all(&cache).ok()?;

    let zip_url = "https://github.com/skeeto/w64devkit/releases/download/v2.0.0/w64devkit-2.0.0.zip";
    let zip_path = cache.join("w64devkit.zip");

    if !zip_path.exists() {
        download_file(zip_url, &zip_path).ok()?;
    }

    let file = std::fs::File::open(&zip_path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).ok()?;
        let name = entry.name().to_string();
        let out_path = cache.join(&name);

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).ok();
            continue;
        }

        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let mut out = std::fs::File::create(&out_path).ok()?;
        std::io::copy(&mut entry, &mut out).ok()?;
    }

    std::fs::remove_file(&zip_path).ok();
    if gpp_path.exists() { Some(gpp_path) } else { None }
}

fn resolve_compiler(info: &CompilerInfo, ext: &str) -> (Option<Compiler>, Option<String>) {
    let (c_cands, cxx_cands): (&[&str], &[&str]) = if cfg!(target_os = "windows") {
        (&["gcc", "clang", "cl.exe", "cc"], &["g++", "clang++", "cl.exe", "c++"])
    } else {
        (&["gcc", "clang", "cc"], &["g++", "clang++", "c++"])
    };
    let candidates: &[&str] = match ext {
        "c" => c_cands,
        "cpp" | "cc" | "cxx" | "c++" => cxx_cands,
        _ => return (None, Some(format!("Unsupported file type: .{}", ext))),
    };

    // Check cached info first
    let from_info = match ext {
        "c" => info.cc.clone(),
        _ => info.cxx.clone(),
    };
    if let Some(c) = from_info {
        return (Some(c), None);
    }

    // Probe system compiler
    if let Some(c) = probe_system_compiler(candidates) {
        return (Some(c), None);
    }

    // Auto-download fallback
    if ext == "c" {
        if let Some(path) = download_tcc() {
            return (Some(Compiler::Bundled { path }), None);
        }
    }

    if cfg!(target_os = "windows") && matches!(ext, "cpp" | "cc" | "cxx" | "c++") {
        if let Some(path) = download_mingw() {
            return (Some(Compiler::Bundled { path }), None);
        }
    }

    let hint = match (ext, std::env::consts::OS) {
        ("c", "linux") => "No C compiler. Install: sudo apt install gcc | sudo dnf install gcc | sudo pacman -S gcc",
        ("c", "macos") => "No C compiler. Install: xcode-select --install | brew install gcc",
        ("c", "windows") => "No C compiler found.",
        (_, "linux") => "No C++ compiler. Install: sudo apt install g++ | sudo dnf install gcc-c++ | sudo pacman -S gcc",
        (_, "macos") => "No C++ compiler. Install: xcode-select --install | brew install gcc",
        (_, "windows") => "No C++ compiler found.",
        _ => "No compiler found. Please install gcc/g++ or clang.",
    };
    (None, Some(hint.to_string()))
}

pub fn probe_compilers() -> CompilerInfo {
    let empty = CompilerInfo { cc: None, cxx: None, problem: None };
    let (cc, _) = resolve_compiler(&empty, "c");
    let (cxx, _) = resolve_compiler(&CompilerInfo { cc: cc.clone(), cxx: None, problem: None }, "cpp");
    CompilerInfo { cc, cxx, problem: None }
}

fn compiler_exe(c: &Compiler) -> &Path {
    match c {
        Compiler::System { name } => Path::new(name),
        Compiler::Bundled { path } => path.as_path(),
    }
}

fn is_tcc(c: &Compiler) -> bool {
    matches!(c, Compiler::Bundled { .. })
}

fn output_path(source_path: &str) -> String {
    let ext = if cfg!(target_os = "windows") { ".exe" } else { ".out" };
    let p = std::path::Path::new(source_path);
    format!("{}{}", p.with_extension("").display(), ext)
}

pub fn compile(source_path: &str, compiler: &Compiler) -> io::Result<BuildResult> {
    let out = output_path(source_path);
    let compiler_str = compiler_exe(compiler).to_string_lossy().to_string();

    let flags: &[&str] = if is_tcc(compiler) {
        &["-o", &out, source_path]
    } else {
        &["-Wall", "-o", &out, source_path]
    };

    let cmd_line = format!("{} {}", compiler_str, flags.join(" "));
    let result = Command::new(compiler_exe(compiler)).args(flags).output()?;

    Ok(BuildResult {
        success: result.status.success(),
        output: String::from_utf8_lossy(&result.stderr).to_string(),
        command_line: cmd_line,
    })
}

pub fn compile_and_run(source_path: &str, compiler: &Compiler) -> io::Result<BuildResult> {
    let build = compile(source_path, compiler)?;
    if !build.success { return Ok(build); }

    let out = output_path(source_path);
    let run_path = if cfg!(target_os = "windows") {
        format!(".\\{}", out)
    } else {
        format!("./{}", out)
    };
    let run_result = Command::new(&run_path).output()?;

    let mut full_output = build.output.clone();
    if !full_output.is_empty() { full_output.push_str("\n---\n"); }
    full_output.push_str(&format!("$ {}\n", run_path));
    full_output.push_str(&String::from_utf8_lossy(&run_result.stdout));
    if !run_result.stderr.is_empty() {
        full_output.push_str(&String::from_utf8_lossy(&run_result.stderr));
    }

    Ok(BuildResult {
        success: run_result.status.success(),
        output: full_output,
        command_line: build.command_line.clone(),
    })
}

pub fn select_compiler(info: &CompilerInfo, ext: &str) -> (Option<Compiler>, Option<String>) {
    resolve_compiler(info, ext)
}
