use std::io::{self, Read};
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone)]
pub enum Compiler {
    System { name: String },
    Tcc { path: PathBuf },
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
        // cl.exe uses different flag for version
        if name == &"cl.exe" {
            cmd.arg("/?");
        } else {
            cmd.arg("--version");
        }
        if cmd.output().is_ok() {
            return Some(Compiler::System { name: name.to_string() });
        }
    }
    None
}

fn tcc_cache_dir() -> PathBuf {
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

fn download_tcc() -> Option<PathBuf> {
    let cache = tcc_cache_dir();
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;

    let (url, exe_name) = match (os, arch) {
        ("linux", "x86_64") => (
            "https://github.com/sz3/tinycc/releases/download/v0.9.27/tcc-x86_64-linux",
            "tcc",
        ),
        ("linux", "aarch64") => (
            "https://github.com/sz3/tinycc/releases/download/v0.9.27/tcc-aarch64-linux",
            "tcc",
        ),
        ("macos", "x86_64") => (
            "https://github.com/sz3/tinycc/releases/download/v0.9.27/tcc-x86_64-macos",
            "tcc",
        ),
        ("macos", "aarch64") => (
            "https://github.com/sz3/tinycc/releases/download/v0.9.27/tcc-aarch64-macos",
            "tcc",
        ),
        ("windows", "x86_64") => (
            "https://github.com/sz3/tinycc/releases/download/v0.9.27/tcc-x86_64-win32.exe",
            "tcc.exe",
        ),
        _ => return None,
    };

    std::fs::create_dir_all(&cache).ok()?;
    let exe_path = cache.join(exe_name);

    if exe_path.exists() {
        return Some(exe_path);
    }

    let _ = std::fs::write(&cache.join("tcc.url"), url);

    let response = ureq::get(url).call();
    match response {
        Ok(resp) => {
            let mut body: Vec<u8> = Vec::new();
            let mut reader = resp.into_body().into_reader();
            if reader.read_to_end(&mut body).is_ok() && !body.is_empty() {
                std::fs::write(&exe_path, &body).ok()?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&exe_path, std::fs::Permissions::from_mode(0o755)).ok()?;
                }
                return Some(exe_path);
            }
            None
        }
        Err(e) => {
            let _ = std::fs::write(&cache.join("tcc.download_error"), format!("{}", e));
            None
        }
    }
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

    // Try cached info first
    let from_info = match ext {
        "c" => info.cc.clone(),
        _ => info.cxx.clone(),
    };
    if let Some(c) = from_info {
        if matches!(c, Compiler::System { .. } | Compiler::Tcc { .. }) {
            return (Some(c), None);
        }
    }

    // Probe system compiler
    if let Some(c) = probe_system_compiler(candidates) {
        return (Some(c), None);
    }

    // Auto-download TCC (C only)
    if ext == "c" {
        if let Some(path) = download_tcc() {
            return (Some(Compiler::Tcc { path }), None);
        }
    }

    let hint = match ext {
        "c" => {
            let os = std::env::consts::OS;
            match os {
                "linux" => "No C compiler. Install with: sudo apt install gcc  |  sudo dnf install gcc  |  sudo pacman -S gcc",
                "macos" => "No C compiler. Install with: xcode-select --install  |  brew install gcc",
                "windows" => "No C compiler. Download: https://winget.run/pkg/LLVM/LLVM  or  install MSVC Build Tools",
                _ => "No C compiler found. Please install gcc or clang.",
            }
        }
        _ => {
            let os = std::env::consts::OS;
            match os {
                "linux" => "No C++ compiler. Install with: sudo apt install g++  |  sudo dnf install gcc-c++  |  sudo pacman -S gcc",
                "macos" => "No C++ compiler. Install with: xcode-select --install  |  brew install gcc",
                "windows" => "No C++ compiler. Download: https://winget.run/pkg/LLVM/LLVM  or  install MSVC Build Tools",
                _ => "No C++ compiler found. Please install g++ or clang++.",
            }
        }
    };
    (None, Some(hint.to_string()))
}

pub fn probe_compilers() -> CompilerInfo {
    let (cc, cc_problem) = resolve_compiler(&CompilerInfo { cc: None, cxx: None, problem: None }, "c");
    let (cxx, cxx_problem) = resolve_compiler(&CompilerInfo { cc: cc.clone(), cxx: None, problem: None }, "cpp");
    CompilerInfo {
        cc,
        cxx,
        problem: cc_problem.or(cxx_problem),
    }
}

fn compiler_name(c: &Compiler) -> &str {
    match c {
        Compiler::System { name } => name,
        Compiler::Tcc { path } => path.to_str().unwrap_or("tcc"),
    }
}

fn build_cmd(c: &Compiler) -> Command {
    match c {
        Compiler::System { name } => Command::new(name),
        Compiler::Tcc { path } => Command::new(path),
    }
}

fn output_path(source_path: &str) -> String {
    let ext = if cfg!(target_os = "windows") { ".exe" } else { ".out" };
    source_path.trim_end_matches(|c| c == '.').to_string() + ext
}

pub fn compile(source_path: &str, compiler: &Compiler) -> io::Result<BuildResult> {
    let out = output_path(source_path);
    let compiler_str = compiler_name(compiler);

    let flags: &[&str] = match compiler {
        Compiler::Tcc { .. } => &["-o", &out, source_path],
        Compiler::System { .. } => &["-Wall", "-o", &out, source_path],
    };

    let cmd_line = format!("{} {}", compiler_str, flags.join(" "));
    let result = build_cmd(compiler).args(flags).output()?;

    let output_text = String::from_utf8_lossy(&result.stderr).to_string();
    Ok(BuildResult {
        success: result.status.success(),
        output: output_text,
        command_line: cmd_line,
    })
}

pub fn compile_and_run(source_path: &str, compiler: &Compiler) -> io::Result<BuildResult> {
    let build = compile(source_path, compiler)?;
    if !build.success {
        return Ok(build);
    }

    let out = output_path(source_path);
    let run_path = if cfg!(target_os = "windows") {
        format!(".\\{}", out)
    } else {
        format!("./{}", out)
    };
    let run_result = Command::new(&run_path).output()?;

    let mut full_output = build.output.clone();
    if !full_output.is_empty() {
        full_output.push_str("\n---\n");
    }
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
