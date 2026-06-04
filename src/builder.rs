use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

const ZIG_VERSION: &str = "0.15.2";
const TCC_WIN64_URL: &str =
    "https://download.savannah.gnu.org/releases/tinycc/tcc-0.9.27-win64-bin.zip";
const W64DEVKIT_URL: &str =
    "https://github.com/skeeto/w64devkit/releases/download/v1.22.0/w64devkit-1.22.0.zip";
const UV_INSTALL_PS1_URL: &str = "https://astral.sh/uv/install.ps1";
const UV_INSTALL_SH_URL: &str = "https://astral.sh/uv/install.sh";

#[derive(Clone)]
pub enum Compiler {
    System { name: String },
    Msvc { name: String },
    GccWithStdCxx { name: String },
    ZigCxx { path: PathBuf },
    Tcc { path: PathBuf },
    W64DevkitCxx { path: PathBuf },
    Python { name: String },
    UvPython { path: PathBuf },
}

pub struct BuildResult {
    pub success: bool,
    pub output: String,
    pub command_line: String,
}

pub struct InteractiveRun {
    pub cwd: PathBuf,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub display: String,
}

pub struct CompilerInfo {
    pub cc: Option<Compiler>,
    pub cxx: Option<Compiler>,
    pub python: Option<Compiler>,
    pub problem: Option<String>,
}

fn probe_system_compiler(candidates: &[&str]) -> Option<Compiler> {
    for name in candidates {
        let mut cmd = Command::new(name);
        if name == &"cl.exe" {
            cmd.arg("/?");
        } else {
            cmd.arg("--version");
        }
        if cmd.output().is_ok() {
            let name = name.to_string();
            return Some(if name.eq_ignore_ascii_case("cl.exe") {
                Compiler::Msvc { name }
            } else {
                Compiler::System { name }
            });
        }
    }
    None
}

fn probe_python(candidates: &[&str]) -> Option<Compiler> {
    for name in candidates {
        let output = Command::new(name).arg("--version").output();
        if output.as_ref().is_ok_and(|output| output.status.success()) {
            return Some(Compiler::Python {
                name: name.to_string(),
            });
        }
    }
    None
}

fn uv_dir() -> PathBuf {
    cache_dir().join("uv")
}

fn uv_exe_path() -> PathBuf {
    uv_dir().join(if cfg!(target_os = "windows") {
        "uv.exe"
    } else {
        "uv"
    })
}

fn download_uv() -> Result<PathBuf, String> {
    let uv_exe = uv_exe_path();
    if uv_exe.exists() {
        return Ok(uv_exe);
    }

    let cache = cache_dir();
    let uv_dir = uv_dir();
    std::fs::create_dir_all(&cache)
        .map_err(|e| format!("create cache directory {}: {}", cache.display(), e))?;
    std::fs::create_dir_all(&uv_dir)
        .map_err(|e| format!("create uv directory {}: {}", uv_dir.display(), e))?;

    let (default_url, script_name) = if cfg!(target_os = "windows") {
        (UV_INSTALL_PS1_URL, "uv-install.ps1")
    } else {
        (UV_INSTALL_SH_URL, "uv-install.sh")
    };
    let url = std::env::var("TINYVIM_UV_URL").unwrap_or_else(|_| default_url.to_string());
    let script_path = cache.join(script_name);
    if !script_path.exists() {
        download_file(&url, &script_path).map_err(|e| {
            format!(
                "download {}: {}. {} Manual fallback: download this installer to {}, or put uv executable at {}, or set TINYVIM_UV_URL to a reachable mirror URL, then press F5/F6 again.",
                url,
                e,
                download_failure_hint(&url, &cache),
                script_path.display(),
                uv_exe.display()
            )
        })?;
    }

    let status = if cfg!(target_os = "windows") {
        Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script_path)
            .env("UV_UNMANAGED_INSTALL", &uv_dir)
            .status()
            .map_err(|e| format!("run PowerShell uv installer: {}", e))?
    } else {
        Command::new("sh")
            .arg(&script_path)
            .env("UV_UNMANAGED_INSTALL", &uv_dir)
            .status()
            .map_err(|e| format!("run uv installer: {}", e))?
    };

    if status.success() && uv_exe.exists() {
        Ok(uv_exe)
    } else {
        Err(format!(
            "uv installer exited with status {} and {} was not created. Manual fallback: install Python normally, place a working uv installer at {}, or put uv executable directly at {}, then press F5/F6 again.",
            status,
            uv_exe.display(),
            script_path.display(),
            uv_exe.display()
        ))
    }
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
    let sub = if cfg!(target_os = "windows") {
        "tinyvim"
    } else {
        ".cache/tinyvim"
    };
    PathBuf::from(base).join(sub)
}

fn download_file(url: &str, dest: &Path) -> io::Result<()> {
    let resp = ureq::get(url)
        .call()
        .map_err(|e| io::Error::other(format!("HTTP: {}", e)))?;
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

fn try_open_url(url: &str) -> bool {
    let result = if cfg!(target_os = "windows") {
        Command::new("cmd").args(["/C", "start", "", url]).spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()
    } else {
        Command::new("xdg-open").arg(url).spawn()
    };
    result.is_ok()
}

fn try_open_folder(path: &Path) -> bool {
    let result = if cfg!(target_os = "windows") {
        Command::new("explorer").arg(path).spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(path).spawn()
    } else {
        Command::new("xdg-open").arg(path).spawn()
    };
    result.is_ok()
}

fn download_failure_hint(url: &str, cache: &Path) -> String {
    let opened_url = try_open_url(url);
    let opened_cache = try_open_folder(cache);
    let url_status = if opened_url {
        "opened download URL"
    } else {
        "could not open download URL automatically"
    };
    let cache_status = if opened_cache {
        "opened cache folder"
    } else {
        "could not open cache folder automatically"
    };
    format!("TinyVim {url_status} and {cache_status}.")
}

fn unzip_archive(zip_path: &Path, out_dir: &Path, label: &str) -> Result<(), String> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| format!("open downloaded archive {}: {}", zip_path.display(), e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("read {} zip: {}", label, e))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("read {} zip entry {}: {}", label, i, e))?;
        let name = entry.name().to_string();
        let out_path = out_dir.join(&name);

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| format!("create directory {}: {}", out_path.display(), e))?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create directory {}: {}", parent.display(), e))?;
        }
        let mut out = std::fs::File::create(&out_path)
            .map_err(|e| format!("create file {}: {}", out_path.display(), e))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|e| format!("extract file {}: {}", out_path.display(), e))?;
    }

    Ok(())
}

fn download_tcc() -> Result<PathBuf, String> {
    let cache = cache_dir();
    let (default_url, exe_name, is_zip) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => (
            "https://github.com/sz3/tinycc/releases/download/v0.9.27/tcc-x86_64-linux",
            "tcc",
            false,
        ),
        ("linux", "aarch64") => (
            "https://github.com/sz3/tinycc/releases/download/v0.9.27/tcc-aarch64-linux",
            "tcc",
            false,
        ),
        ("macos", "x86_64") => (
            "https://github.com/sz3/tinycc/releases/download/v0.9.27/tcc-x86_64-macos",
            "tcc",
            false,
        ),
        ("macos", "aarch64") => (
            "https://github.com/sz3/tinycc/releases/download/v0.9.27/tcc-aarch64-macos",
            "tcc",
            false,
        ),
        ("windows", "x86_64") => (TCC_WIN64_URL, "tcc.exe", true),
        _ => {
            return Err(format!(
                "no bundled TCC build for {} {}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ))
        }
    };
    let url = std::env::var("TINYVIM_TCC_URL").unwrap_or_else(|_| default_url.to_string());

    std::fs::create_dir_all(&cache)
        .map_err(|e| format!("create cache directory {}: {}", cache.display(), e))?;
    if let Some(path) = find_file_recursive(&cache, exe_name) {
        return Ok(path);
    }

    if is_zip {
        let zip_path = cache.join("tcc.zip");
        if !zip_path.exists() {
            download_file(&url, &zip_path).map_err(|e| {
                format!(
                    "download {}: {}. {} Manual fallback: download this TCC package to {}, or set TINYVIM_TCC_URL to a reachable mirror URL, then press F5/F6 again.",
                    url,
                    e,
                    download_failure_hint(&url, &cache),
                    zip_path.display()
                )
            })?;
        }
        unzip_archive(&zip_path, &cache, "TCC")?;
        std::fs::remove_file(&zip_path).ok();
        find_file_recursive(&cache, exe_name).ok_or_else(|| {
            format!(
                "TCC extracted, but {} was not found under {}",
                exe_name,
                cache.display()
            )
        })
    } else {
        let exe_path = cache.join(exe_name);
        download_file(&url, &exe_path).map_err(|e| {
            format!(
                "download {}: {}. {} Manual fallback: download this TCC executable to {}, or set TINYVIM_TCC_URL to a reachable mirror URL, then press F5/F6 again.",
                url,
                e,
                download_failure_hint(&url, &cache),
                exe_path.display()
            )
        })?;
        Ok(exe_path)
    }
}

fn cached_tcc() -> Option<PathBuf> {
    let cache = cache_dir();
    let exe_name = if cfg!(target_os = "windows") {
        "tcc.exe"
    } else {
        "tcc"
    };
    find_file_recursive(&cache, exe_name)
}

fn find_file_recursive(root: &Path, file_name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_recursive(&path, file_name) {
                return Some(found);
            }
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(file_name))
        {
            return Some(path);
        }
    }
    None
}

fn cached_mingw_gpp(cache: &Path) -> Option<PathBuf> {
    let direct = cache.join("w64devkit").join("bin").join("g++.exe");
    if direct.exists() {
        return Some(direct);
    }
    find_file_recursive(cache, "g++.exe").filter(|path| {
        path.components().any(|component| {
            component
                .as_os_str()
                .to_string_lossy()
                .contains("w64devkit")
        })
    })
}

fn download_mingw() -> Result<PathBuf, String> {
    let cache = cache_dir();

    if let Some(gpp_path) = cached_mingw_gpp(&cache) {
        return Ok(gpp_path);
    }

    std::fs::create_dir_all(&cache)
        .map_err(|e| format!("create cache directory {}: {}", cache.display(), e))?;

    let zip_url =
        std::env::var("TINYVIM_W64DEVKIT_URL").unwrap_or_else(|_| W64DEVKIT_URL.to_string());
    let zip_path = cache.join("w64devkit.zip");

    if !zip_path.exists() {
        download_file(&zip_url, &zip_path).map_err(|e| {
            format!(
                "download {}: {}. {} Manual fallback: download this w64devkit zip to {}, or set TINYVIM_W64DEVKIT_URL to a reachable mirror URL, then press F5/F6 again.",
                zip_url,
                e,
                download_failure_hint(&zip_url, &cache),
                zip_path.display()
            )
        })?;
    }

    unzip_archive(&zip_path, &cache, "w64devkit")?;
    std::fs::remove_file(&zip_path).ok();
    cached_mingw_gpp(&cache).ok_or_else(|| {
        format!(
            "w64devkit extracted, but g++.exe was not found under {}",
            cache.display()
        )
    })
}

fn zig_dir() -> PathBuf {
    cache_dir().join("zig")
}

fn zig_archive_name() -> Option<String> {
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-linux",
        ("linux", "aarch64") => "aarch64-linux",
        ("macos", "x86_64") => "x86_64-macos",
        ("macos", "aarch64") => "aarch64-macos",
        _ => return None,
    };
    Some(format!("zig-{}-{}", target, ZIG_VERSION))
}

fn find_cached_zig() -> Option<PathBuf> {
    let root = zig_dir();
    let direct = root.join("zig");
    if direct.exists() {
        return Some(direct);
    }
    let expected = root.join(zig_archive_name()?).join("zig");
    if expected.exists() {
        return Some(expected);
    }
    for entry in std::fs::read_dir(&root).ok()?.flatten() {
        let candidate = entry.path().join("zig");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn download_zig() -> Result<PathBuf, String> {
    if cfg!(target_os = "windows") {
        return Err("Zig C++ fallback is not used on Windows".to_string());
    }
    if let Some(path) = find_cached_zig() {
        return Ok(path);
    }

    let cache = zig_dir();
    std::fs::create_dir_all(&cache)
        .map_err(|e| format!("create Zig cache directory {}: {}", cache.display(), e))?;
    let archive_stem = zig_archive_name().ok_or_else(|| {
        format!(
            "no bundled Zig build for {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let default_url = format!(
        "https://ziglang.org/download/{}/{}.tar.xz",
        ZIG_VERSION, archive_stem
    );
    let url = std::env::var("TINYVIM_ZIG_URL").unwrap_or(default_url);
    let archive_path = cache.join("zig.tar.xz");
    if !archive_path.exists() {
        download_file(&url, &archive_path).map_err(|e| {
            format!(
                "download {}: {}. {} Manual fallback: download this Zig archive to {}, or set TINYVIM_ZIG_URL to a reachable mirror URL, then press F5/F6 again.",
                url,
                e,
                download_failure_hint(&url, &cache),
                archive_path.display()
            )
        })?;
    }

    let status = Command::new("tar")
        .args(["-xJf"])
        .arg(&archive_path)
        .arg("-C")
        .arg(&cache)
        .status()
        .map_err(|e| {
            format!(
                "run tar to extract Zig archive {}: {}",
                archive_path.display(),
                e
            )
        })?;
    if !status.success() {
        return Err(format!(
            "extract Zig archive {} failed with status {}",
            archive_path.display(),
            status
        ));
    }

    let zig = find_cached_zig().ok_or_else(|| {
        format!(
            "Zig extracted, but zig executable was not found under {}",
            cache.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&zig, std::fs::Permissions::from_mode(0o755)).ok();
    }
    Ok(zig)
}

fn resolve_compiler_with_download(
    info: &CompilerInfo,
    ext: &str,
    allow_download: bool,
) -> (Option<Compiler>, Option<String>) {
    let c_cands: &[&str] = if cfg!(target_os = "windows") {
        &["gcc", "clang", "cl.exe", "cc"]
    } else {
        &["gcc", "clang", "cc"]
    };
    let cxx_cands: &[&str] = if cfg!(target_os = "windows") {
        &["g++", "clang++", "cl.exe", "c++"]
    } else {
        &["g++", "clang++", "c++"]
    };
    let py_cands: &[&str] = if cfg!(target_os = "windows") {
        &["python", "py", "python3"]
    } else {
        &["python3", "python"]
    };

    // Check cached info first
    let from_info = match ext {
        "c" => info.cc.clone(),
        "cpp" | "cc" | "cxx" | "c++" => info.cxx.clone(),
        "py" | "pyw" => info.python.clone(),
        "" => {
            return (
                None,
                Some("Save the file with a .c, .cpp, or .py extension first.".to_string()),
            );
        }
        _ => return (None, Some(format!("Unsupported file type: .{}", ext))),
    };
    if let Some(c) = from_info {
        return (Some(c), None);
    }

    let mut download_error = None;

    match ext {
        "c" => {
            if let Some(c) = probe_system_compiler(c_cands) {
                return (Some(c), None);
            }
            if let Some(path) = cached_tcc() {
                return (Some(Compiler::Tcc { path }), None);
            }
        }
        "cpp" | "cc" | "cxx" | "c++" => {
            // First try dedicated C++ compilers
            if let Some(c) = probe_system_compiler(cxx_cands) {
                return (Some(c), None);
            }
            // If g++ not found but gcc exists, use gcc with -lstdc++
            let gcc_candidates: &[&str] = if cfg!(target_os = "windows") {
                &["gcc"]
            } else {
                &["gcc", "clang"]
            };
            if let Some(name) = gcc_candidates.iter().find(|&&name| {
                let mut cmd = Command::new(name);
                if name == "cl.exe" {
                    cmd.arg("/?");
                } else {
                    cmd.arg("--version");
                }
                cmd.output().is_ok()
            }) {
                return (
                    Some(Compiler::GccWithStdCxx {
                        name: name.to_string(),
                    }),
                    None,
                );
            }
            if cfg!(target_os = "windows") {
                if let Some(path) = cached_mingw_gpp(&cache_dir()) {
                    return (Some(Compiler::W64DevkitCxx { path }), None);
                }
            } else if let Some(path) = find_cached_zig() {
                return (Some(Compiler::ZigCxx { path }), None);
            }
        }
        "py" | "pyw" => {
            if let Some(py) = probe_python(py_cands) {
                return (Some(py), None);
            }
            let uv_exe = uv_exe_path();
            if uv_exe.exists() {
                return (Some(Compiler::UvPython { path: uv_exe }), None);
            }
            if allow_download {
                match download_uv() {
                    Ok(path) => return (Some(Compiler::UvPython { path }), None),
                    Err(e) => download_error = Some(e),
                }
            }
        }
        _ => return (None, Some(format!("Unsupported file type: .{}", ext))),
    }

    // Auto-download fallback. Startup probing passes allow_download=false, so
    // network work only happens when the user actually builds or runs.
    if allow_download && ext == "c" {
        match download_tcc() {
            Ok(path) => return (Some(Compiler::Tcc { path }), None),
            Err(e) => download_error = Some(e),
        }
    }

    if allow_download && cfg!(target_os = "windows") && matches!(ext, "cpp" | "cc" | "cxx" | "c++")
    {
        match download_mingw() {
            Ok(path) => return (Some(Compiler::W64DevkitCxx { path }), None),
            Err(e) => download_error = Some(e),
        }
    }

    if allow_download && !cfg!(target_os = "windows") && matches!(ext, "cpp" | "cc" | "cxx" | "c++")
    {
        match download_zig() {
            Ok(path) => return (Some(Compiler::ZigCxx { path }), None),
            Err(e) => download_error = Some(e),
        }
    }

    let hint = match (ext, std::env::consts::OS) {
        ("py", os) => {
            if let Some(error) = download_error {
                return (
                    None,
                    Some(format!(
                        "No Python interpreter, and automatic uv download failed: {}. Install Python, put uv installer or uv.exe in the shown cache path, or set TINYVIM_UV_URL to a reachable uv installer. Cache: {}",
                        error,
                        cache_dir().display()
                    )),
                );
            }
            match os {
                "linux" => "No Python interpreter. Install: sudo apt install python3 | sudo dnf install python3 | sudo pacman -S python",
                "macos" => "No Python interpreter. Install: xcode-select --install | brew install python",
                "windows" => "No Python interpreter. Install Python from python.org or Microsoft Store.",
                _ => "No Python interpreter found.",
            }
        }
        ("c", os) => {
            if let Some(error) = download_error {
                return (
                    None,
                    Some(format!(
                        "No C compiler, and automatic TCC download failed: {}. Install gcc/clang/MSVC, put the TCC package/executable in the shown cache path, or set TINYVIM_TCC_URL to a reachable TCC package. Cache: {}",
                        error,
                        cache_dir().display()
                    )),
                );
            }
            match os {
                "linux" => "No C compiler. Install: sudo apt install gcc | sudo dnf install gcc | sudo pacman -S gcc",
                "macos" => "No C compiler. Install: xcode-select --install | brew install gcc",
                "windows" => "No C compiler found.",
                _ => "No C compiler found.",
            }
        }
        (_, "linux") => {
            if let Some(error) = download_error {
                return (
                    None,
                    Some(format!(
                        "No C++ compiler, and automatic Zig download failed: {}. Install g++, put zig.tar.xz in the shown cache path, or set TINYVIM_ZIG_URL to a reachable Zig archive. Cache: {}",
                        error,
                        zig_dir().display()
                    )),
                );
            }
            "No C++ compiler. Install: sudo apt install g++ | sudo dnf install gcc-c++ | sudo pacman -S gcc"
        }
        (_, "macos") => {
            if let Some(error) = download_error {
                return (
                    None,
                    Some(format!(
                        "No C++ compiler, and automatic Zig download failed: {}. Install g++, put zig.tar.xz in the shown cache path, or set TINYVIM_ZIG_URL to a reachable Zig archive. Cache: {}",
                        error,
                        zig_dir().display()
                    )),
                );
            }
            "No C++ compiler. Install: xcode-select --install | brew install gcc"
        }
        (_, "windows") => {
            if let Some(error) = download_error {
                return (
                    None,
                    Some(format!(
                        "No C++ compiler, and automatic w64devkit download failed: {}. You can install g++/clang++/MSVC, put w64devkit.zip in the shown cache path, or set TINYVIM_W64DEVKIT_URL to a reachable w64devkit zip. Cache: {}",
                        error,
                        cache_dir().display()
                    )),
                );
            }
            "No C++ compiler found."
        }
        _ => "No compiler found. Please install gcc/g++ or clang.",
    };
    (None, Some(hint.to_string()))
}

fn resolve_compiler(info: &CompilerInfo, ext: &str) -> (Option<Compiler>, Option<String>) {
    resolve_compiler_with_download(info, ext, true)
}

pub fn probe_compilers() -> CompilerInfo {
    let empty = CompilerInfo {
        cc: None,
        cxx: None,
        python: None,
        problem: None,
    };
    let (cc, _) = resolve_compiler_with_download(&empty, "c", false);
    let (cxx, _) = resolve_compiler_with_download(
        &CompilerInfo {
            cc: cc.clone(),
            cxx: None,
            python: None,
            problem: None,
        },
        "cpp",
        false,
    );
    let (python, _) = resolve_compiler_with_download(
        &CompilerInfo {
            cc: cc.clone(),
            cxx: cxx.clone(),
            python: None,
            problem: None,
        },
        "py",
        false,
    );
    CompilerInfo {
        cc,
        cxx,
        python,
        problem: None,
    }
}

fn compiler_exe(c: &Compiler) -> &Path {
    match c {
        Compiler::System { name } => Path::new(name),
        Compiler::Msvc { name } => Path::new(name),
        Compiler::GccWithStdCxx { name } => Path::new(name),
        Compiler::ZigCxx { path } => path.as_path(),
        Compiler::Tcc { path } => path.as_path(),
        Compiler::W64DevkitCxx { path } => path.as_path(),
        Compiler::Python { name } => Path::new(name),
        Compiler::UvPython { path } => path.as_path(),
    }
}

fn is_tcc(c: &Compiler) -> bool {
    matches!(c, Compiler::Tcc { .. })
}

fn output_path(source_path: &str) -> String {
    let ext = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ".out"
    };
    let p = std::path::Path::new(source_path);
    format!("{}{}", p.with_extension("").display(), ext)
}

fn source_parent_dir(source_path: &str) -> PathBuf {
    let parent = Path::new(source_path)
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    parent.to_path_buf()
}

fn run_invocation(source_path: &str) -> io::Result<(PathBuf, PathBuf, String)> {
    let out = output_path(source_path);
    let out_path = Path::new(&out);
    let parent_dir = source_parent_dir(source_path);
    let exe_name = out_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid output path"))?;
    let executable_path = if out_path.is_absolute() {
        out_path.to_path_buf()
    } else {
        std::env::current_dir()?.join(out_path)
    };

    let display_path = if cfg!(target_os = "windows") {
        format!(".\\{}", exe_name)
    } else {
        format!("./{}", exe_name)
    };

    Ok((parent_dir, executable_path, display_path))
}

fn is_cpp_source(source_path: &str) -> bool {
    matches!(
        Path::new(source_path)
            .extension()
            .and_then(|ext| ext.to_str()),
        Some("cpp" | "cc" | "cxx" | "c++")
    )
}

fn is_c_source(source_path: &str) -> bool {
    matches!(
        Path::new(source_path)
            .extension()
            .and_then(|ext| ext.to_str()),
        Some("c")
    )
}

fn compiler_args(source_path: &str, compiler: &Compiler) -> Vec<String> {
    let out = output_path(source_path);
    if matches!(compiler, Compiler::Python { .. }) {
        vec!["-m".into(), "py_compile".into(), source_path.into()]
    } else if matches!(compiler, Compiler::UvPython { .. }) {
        vec![
            "run".into(),
            "--python".into(),
            "3".into(),
            "--".into(),
            "python".into(),
            "-m".into(),
            "py_compile".into(),
            source_path.into(),
        ]
    } else if matches!(compiler, Compiler::ZigCxx { .. }) {
        vec![
            "c++".into(),
            "-Wall".into(),
            "-o".into(),
            out,
            source_path.into(),
        ]
    } else if is_tcc(compiler) {
        vec!["-o".into(), out, source_path.into()]
    } else if matches!(compiler, Compiler::Msvc { .. }) {
        let mut args = vec!["/nologo".into()];
        if is_cpp_source(source_path) {
            args.push("/EHsc".into());
        }
        args.push(source_path.into());
        args.push(format!("/Fe:{}", out));
        args
    } else if matches!(compiler, Compiler::GccWithStdCxx { .. }) {
        vec![
            "-Wall".into(),
            "-o".into(),
            out,
            source_path.into(),
            "-lstdc++".into(),
        ]
    } else if is_c_source(source_path) {
        vec![
            "-Wall".into(),
            "-o".into(),
            out,
            source_path.into(),
            "-lm".into(),
        ]
    } else {
        vec!["-Wall".into(), "-o".into(), out, source_path.into()]
    }
}

fn command_line(exe: &Path, args: &[String]) -> String {
    format!("{} {}", exe.to_string_lossy(), args.join(" "))
}

fn combine_output(stdout: &[u8], stderr: &[u8]) -> String {
    let mut output = String::new();
    output.push_str(&String::from_utf8_lossy(stdout));
    if !stdout.is_empty() && !stderr.is_empty() {
        output.push('\n');
    }
    output.push_str(&String::from_utf8_lossy(stderr));
    output
}

pub fn compile(source_path: &str, compiler: &Compiler) -> io::Result<BuildResult> {
    let all_flags = compiler_args(source_path, compiler);
    let cmd_line = command_line(compiler_exe(compiler), &all_flags);
    let result = Command::new(compiler_exe(compiler))
        .args(&all_flags)
        .output()?;

    Ok(BuildResult {
        success: result.status.success(),
        output: combine_output(&result.stdout, &result.stderr),
        command_line: cmd_line,
    })
}

pub fn compile_and_prepare_run(
    source_path: &str,
    compiler: &Compiler,
) -> io::Result<(BuildResult, Option<InteractiveRun>)> {
    let build = compile(source_path, compiler)?;
    if !build.success {
        return Ok((build, None));
    }
    let run = prepare_run(source_path, compiler)?;
    Ok((build, Some(run)))
}

fn prepare_run(source_path: &str, compiler: &Compiler) -> io::Result<InteractiveRun> {
    if matches!(
        compiler,
        Compiler::Python { .. } | Compiler::UvPython { .. }
    ) {
        return prepare_python_run(source_path, compiler);
    }

    let (cwd, program, display) = run_invocation(source_path)?;
    Ok(InteractiveRun {
        cwd,
        program,
        args: Vec::new(),
        display,
    })
}

fn prepare_python_run(source_path: &str, python: &Compiler) -> io::Result<InteractiveRun> {
    let parent_dir = source_parent_dir(source_path);
    let script_name = Path::new(source_path)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid Python script path"))?;

    let args = if matches!(python, Compiler::UvPython { .. }) {
        vec![
            "run".to_string(),
            "--python".to_string(),
            "3".to_string(),
            "--".to_string(),
            "python".to_string(),
            script_name.to_string(),
        ]
    } else {
        vec![script_name.to_string()]
    };

    Ok(InteractiveRun {
        cwd: parent_dir,
        program: compiler_exe(python).to_path_buf(),
        args,
        display: python_run_display(python, script_name),
    })
}

fn python_run_display(python: &Compiler, script_name: &str) -> String {
    if matches!(python, Compiler::UvPython { .. }) {
        format!(
            "{} run --python 3 -- python {}",
            compiler_exe(python).display(),
            script_name
        )
    } else {
        format!("{} {}", compiler_exe(python).display(), script_name)
    }
}

pub fn select_compiler(info: &CompilerInfo, ext: &str) -> (Option<Compiler>, Option<String>) {
    resolve_compiler(info, ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_path_replaces_source_extension() {
        let expected = if cfg!(target_os = "windows") {
            "src/main.exe"
        } else {
            "src/main.out"
        };

        assert_eq!(output_path("src/main.c"), expected);
    }

    #[test]
    fn run_invocation_uses_source_parent_and_executable_name() {
        let (cwd, executable_path, display_path) = run_invocation("examples/hello.c").unwrap();
        let expected_run = if cfg!(target_os = "windows") {
            ".\\hello.exe"
        } else {
            "./hello.out"
        };

        assert_eq!(cwd, PathBuf::from("examples"));
        assert!(executable_path.ends_with(if cfg!(target_os = "windows") {
            "examples\\hello.exe"
        } else {
            "examples/hello.out"
        }));
        assert_eq!(display_path, expected_run);
    }

    #[test]
    fn run_invocation_uses_dot_for_current_directory_sources() {
        let (cwd, executable_path, display_path) = run_invocation("hello.cpp").unwrap();
        let expected_run = if cfg!(target_os = "windows") {
            ".\\hello.exe"
        } else {
            "./hello.out"
        };

        assert_eq!(cwd, PathBuf::from("."));
        assert!(executable_path.ends_with(if cfg!(target_os = "windows") {
            "hello.exe"
        } else {
            "hello.out"
        }));
        assert_eq!(display_path, expected_run);
    }

    #[test]
    fn select_compiler_rejects_extensionless_files() {
        let info = CompilerInfo {
            cc: Some(Compiler::System { name: "gcc".into() }),
            cxx: Some(Compiler::System { name: "g++".into() }),
            python: Some(Compiler::Python {
                name: "python3".into(),
            }),
            problem: None,
        };

        let (compiler, hint) = select_compiler(&info, "");

        assert!(compiler.is_none());
        assert_eq!(
            hint.as_deref(),
            Some("Save the file with a .c, .cpp, or .py extension first.")
        );
    }

    #[test]
    fn select_compiler_accepts_python_files() {
        let info = CompilerInfo {
            cc: None,
            cxx: None,
            python: Some(Compiler::Python {
                name: "python3".into(),
            }),
            problem: None,
        };

        let (compiler, hint) = select_compiler(&info, "py");

        assert!(matches!(compiler, Some(Compiler::Python { .. })));
        assert!(hint.is_none());
    }

    #[test]
    fn python_compile_uses_py_compile() {
        let args = compiler_args(
            "scripts/hello.py",
            &Compiler::Python {
                name: "python3".into(),
            },
        );

        assert_eq!(
            args,
            vec![
                "-m".to_string(),
                "py_compile".to_string(),
                "scripts/hello.py".to_string()
            ]
        );
    }

    #[test]
    fn uv_python_compile_uses_uv_managed_python() {
        let args = compiler_args(
            "scripts/hello.py",
            &Compiler::UvPython {
                path: PathBuf::from("uv"),
            },
        );

        assert_eq!(
            args,
            vec![
                "run".to_string(),
                "--python".to_string(),
                "3".to_string(),
                "--".to_string(),
                "python".to_string(),
                "-m".to_string(),
                "py_compile".to_string(),
                "scripts/hello.py".to_string()
            ]
        );
    }

    #[test]
    fn uv_python_run_display_invokes_python() {
        let display = python_run_display(
            &Compiler::UvPython {
                path: PathBuf::from("uv"),
            },
            "hello.py",
        );

        assert_eq!(display, "uv run --python 3 -- python hello.py");
    }

    #[test]
    fn c_compile_links_math_library() {
        let args = compiler_args("src/main.c", &Compiler::System { name: "gcc".into() });

        let expected_out = if cfg!(target_os = "windows") {
            "src/main.exe"
        } else {
            "src/main.out"
        };

        assert_eq!(
            args,
            vec![
                "-Wall".to_string(),
                "-o".to_string(),
                expected_out.to_string(),
                "src/main.c".to_string(),
                "-lm".to_string(),
            ]
        );
    }

    #[test]
    fn tcc_compile_uses_simple_output_args() {
        let args = compiler_args(
            "src/main.c",
            &Compiler::Tcc {
                path: PathBuf::from("tcc"),
            },
        );

        let expected_out = if cfg!(target_os = "windows") {
            "src/main.exe"
        } else {
            "src/main.out"
        };

        assert_eq!(
            args,
            vec![
                "-o".to_string(),
                expected_out.to_string(),
                "src/main.c".to_string(),
            ]
        );
    }

    #[test]
    fn w64devkit_cxx_compile_uses_gpp_style_args() {
        let args = compiler_args(
            "src/main.cpp",
            &Compiler::W64DevkitCxx {
                path: PathBuf::from("g++.exe"),
            },
        );

        let expected_out = if cfg!(target_os = "windows") {
            "src/main.exe"
        } else {
            "src/main.out"
        };

        assert_eq!(
            args,
            vec![
                "-Wall".to_string(),
                "-o".to_string(),
                expected_out.to_string(),
                "src/main.cpp".to_string(),
            ]
        );
    }

    #[test]
    fn zig_cxx_compile_invokes_zig_cxx() {
        let args = compiler_args(
            "src/main.cpp",
            &Compiler::ZigCxx {
                path: PathBuf::from("zig"),
            },
        );

        let expected_out = if cfg!(target_os = "windows") {
            "src/main.exe"
        } else {
            "src/main.out"
        };

        assert_eq!(
            args,
            vec![
                "c++".to_string(),
                "-Wall".to_string(),
                "-o".to_string(),
                expected_out.to_string(),
                "src/main.cpp".to_string(),
            ]
        );
    }
}
