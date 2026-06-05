use std::io::{self, BufReader, Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const ZIG_VERSION: &str = "0.15.2";
const TCC_WIN64_URL: &str =
    "https://download.savannah.gnu.org/releases/tinycc/tcc-0.9.27-win64-bin.zip";
const W64DEVKIT_URL: &str =
    "https://github.com/skeeto/w64devkit/releases/download/v1.22.0/w64devkit-1.22.0.zip";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
const BUILD_TIMEOUT: Duration = Duration::from_secs(300);
const WINDOWS_ACCESS_RETRY_ATTEMPTS: usize = 8;
const WINDOWS_ACCESS_RETRY_DELAY: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub enum Compiler {
    System { name: String },
    Msvc { name: String },
    GccWithStdCxx { name: String },
    ZigCc { path: PathBuf },
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

#[derive(Clone)]
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
        if is_windows_python_store_alias(name) {
            continue;
        }
        let output = Command::new(name)
            .args(["-c", "import sys; sys.exit(0)"])
            .output();
        if output.as_ref().is_ok_and(|output| output.status.success()) {
            return Some(Compiler::Python {
                name: name.to_string(),
            });
        }
    }
    None
}

fn is_windows_python_store_alias(name: &str) -> bool {
    if !cfg!(target_os = "windows") || !matches!(name, "python" | "python3") {
        return false;
    }
    let Ok(output) = Command::new("where.exe").arg(name).output() else {
        return false;
    };
    let first_path = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    first_path.contains("\\microsoft\\windowsapps\\")
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

fn uv_exe_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "uv.exe"
    } else {
        "uv"
    }
}

fn uv_archive_info() -> Result<(&'static str, &'static str, bool), String> {
    let (asset, is_zip) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => ("uv-x86_64-unknown-linux-musl.tar.gz", false),
        ("linux", "aarch64") => ("uv-aarch64-unknown-linux-musl.tar.gz", false),
        ("macos", "x86_64") => ("uv-x86_64-apple-darwin.tar.gz", false),
        ("macos", "aarch64") => ("uv-aarch64-apple-darwin.tar.gz", false),
        ("windows", "x86_64") => ("uv-x86_64-pc-windows-msvc.zip", true),
        _ => {
            return Err(format!(
                "no bundled uv build for {} {}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ))
        }
    };
    let url = match asset {
        "uv-x86_64-unknown-linux-musl.tar.gz" => {
            "https://github.com/astral-sh/uv/releases/latest/download/uv-x86_64-unknown-linux-musl.tar.gz"
        }
        "uv-aarch64-unknown-linux-musl.tar.gz" => {
            "https://github.com/astral-sh/uv/releases/latest/download/uv-aarch64-unknown-linux-musl.tar.gz"
        }
        "uv-x86_64-apple-darwin.tar.gz" => {
            "https://github.com/astral-sh/uv/releases/latest/download/uv-x86_64-apple-darwin.tar.gz"
        }
        "uv-aarch64-apple-darwin.tar.gz" => {
            "https://github.com/astral-sh/uv/releases/latest/download/uv-aarch64-apple-darwin.tar.gz"
        }
        "uv-x86_64-pc-windows-msvc.zip" => {
            "https://github.com/astral-sh/uv/releases/latest/download/uv-x86_64-pc-windows-msvc.zip"
        }
        _ => unreachable!(),
    };
    Ok((url, asset, is_zip))
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

    if let Some(path) = find_file_in_manual_tool_roots(uv_exe_name()) {
        return Ok(path);
    }

    let (default_url, archive_name, is_zip) = uv_archive_info()?;
    let url = std::env::var("TINYVIM_UV_URL").unwrap_or_else(|_| default_url.to_string());
    let fallback_archive_name = if is_zip { "uv.zip" } else { "uv.tar.gz" };
    let archive_names = vec![archive_name.to_string(), fallback_archive_name.to_string()];
    let archive_path = find_named_file_in_manual_tool_roots(&archive_names)
        .unwrap_or_else(|| cache.join(fallback_archive_name));
    if !archive_path.exists() {
        download_file(&url, &archive_path).map_err(|e| {
            format!(
                "download {}: {}. {} Manual fallback: download this uv archive ({}) to {} or your Downloads folder, or extract it under either folder. TinyVim will search for {} automatically. You can also set TINYVIM_UV_URL to a reachable uv archive URL, then press F5/F6 again.",
                url,
                e,
                download_failure_hint(&url, &cache),
                archive_name,
                cache.display(),
                uv_exe_name()
            )
        })?;
    }

    if is_zip {
        unzip_archive(&archive_path, &uv_dir, "uv")?;
    } else {
        untar_gz_archive(&archive_path, &uv_dir, "uv")?;
    }

    if let Some(path) = find_file_in_manual_tool_roots(uv_exe_name()) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).ok();
        }
        Ok(path)
    } else {
        Err(format!(
            "uv archive extracted, but {} was not found under {}. Manual fallback: put or extract uv under {} or your Downloads folder, then press F5/F6 again.",
            uv_exe_name(),
            uv_dir.display(),
            cache.display(),
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

fn downloads_dir() -> Option<PathBuf> {
    let base = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    let path = PathBuf::from(base).join("Downloads");
    path.exists().then_some(path)
}

fn manual_tool_roots() -> Vec<(PathBuf, usize)> {
    let cache = cache_dir();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let cwd = std::env::current_dir().ok();

    let mut roots = Vec::new();
    push_manual_tool_root(&mut roots, Some(cache), usize::MAX);
    push_manual_tool_root(&mut roots, downloads_dir(), 4);
    push_manual_tool_root(&mut roots, exe_dir, 4);
    push_manual_tool_root(&mut roots, cwd, 4);
    roots
}

fn push_manual_tool_root(
    roots: &mut Vec<(PathBuf, usize)>,
    path: Option<PathBuf>,
    max_depth: usize,
) {
    let Some(path) = path.filter(|path| path.exists()) else {
        return;
    };
    if !roots.iter().any(|(existing, _)| existing == &path) {
        roots.push((path, max_depth));
    }
}

fn find_file_in_manual_tool_roots(file_name: &str) -> Option<PathBuf> {
    find_named_file_in_manual_tool_roots(&[file_name.to_string()])
}

fn find_named_file_in_manual_tool_roots(file_names: &[String]) -> Option<PathBuf> {
    for (root, max_depth) in manual_tool_roots() {
        if let Some(path) = find_named_file_recursive_with_depth(&root, file_names, max_depth) {
            return Some(path);
        }
    }
    None
}

fn download_file(url: &str, dest: &Path) -> io::Result<()> {
    match download_file_with_ureq(url, dest) {
        Ok(()) => return Ok(()),
        Err(ureq_error) if cfg!(target_os = "windows") => {
            download_file_with_curl(url, dest).map_err(|curl_error| {
                io::Error::other(format!(
                    "ureq failed: {}; curl.exe failed: {}",
                    ureq_error, curl_error
                ))
            })?;
        }
        Err(e) => return Err(e),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755)).ok();
    }
    Ok(())
}

fn download_file_with_ureq(url: &str, dest: &Path) -> io::Result<()> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        .build()
        .into();
    let resp = agent
        .get(url)
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

fn download_file_with_curl(url: &str, dest: &Path) -> io::Result<()> {
    let mut command = Command::new("curl.exe");
    command.args([
        "-L",
        "--fail",
        "--max-time",
        &DOWNLOAD_TIMEOUT.as_secs().to_string(),
        "-o",
    ]);
    command.arg(dest);
    command.arg(url);
    let output = run_command_output_timeout(&mut command, DOWNLOAD_TIMEOUT, "download command")?;
    if output.status.success() {
        return Ok(());
    }
    let details = combine_output(&output.stdout, &output.stderr);
    Err(io::Error::other(format!(
        "curl.exe exited with {}: {}",
        output.status, details
    )))
}

fn is_windows_access_denied(error: &io::Error) -> bool {
    cfg!(target_os = "windows")
        && (error.kind() == io::ErrorKind::PermissionDenied || error.raw_os_error() == Some(5))
}

fn output_mentions_access_denied(output: &str) -> bool {
    if !cfg!(target_os = "windows") {
        return false;
    }
    let lower = output.to_lowercase();
    lower.contains("access is denied")
        || lower.contains("permission denied")
        || lower.contains("拒绝访问")
}

fn windows_access_denied_hint(program: &Path, label: &str, error: &io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!(
            "Windows denied access while starting {} ({}) after retrying. Original error: {}. This is usually Windows Defender/antivirus scanning a freshly downloaded tool or the previous run still locking the exe. Wait a moment, close any still-running program window, or allow TinyVim/cache/project folders in Windows Security, then try again.",
            label,
            program.display(),
            error
        ),
    )
}

fn spawn_with_windows_access_retry(command: &mut Command, label: &str) -> io::Result<Child> {
    let program = Path::new(command.get_program()).to_path_buf();
    let mut last_error = None;

    for attempt in 1..=WINDOWS_ACCESS_RETRY_ATTEMPTS {
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(e) if is_windows_access_denied(&e) && attempt < WINDOWS_ACCESS_RETRY_ATTEMPTS => {
                last_error = Some(e);
                thread::sleep(WINDOWS_ACCESS_RETRY_DELAY);
            }
            Err(e) if is_windows_access_denied(&e) => {
                return Err(windows_access_denied_hint(&program, label, &e));
            }
            Err(e) => return Err(e),
        }
    }

    Err(windows_access_denied_hint(
        &program,
        label,
        &last_error.unwrap_or_else(|| io::Error::from(io::ErrorKind::PermissionDenied)),
    ))
}

fn run_command_output_timeout(
    command: &mut Command,
    timeout: Duration,
    label: &str,
) -> io::Result<Output> {
    command
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8");
    let mut child = spawn_with_windows_access_retry(
        command.stdout(Stdio::piped()).stderr(Stdio::piped()),
        label,
    )?;
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output();
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "{} timed out after {} seconds. Check whether the compiler/Python process is stuck, then try again. If this is first-time setup on a clean machine, install the tool manually or put the downloaded tool in TinyVim's cache path shown in Output.",
                    label,
                    timeout.as_secs()
                ),
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
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

fn untar_xz_archive(archive_path: &Path, out_dir: &Path, label: &str) -> Result<(), String> {
    let file = std::fs::File::open(archive_path)
        .map_err(|e| format!("open downloaded archive {}: {}", archive_path.display(), e))?;
    let mut reader = BufReader::new(file);
    let mut tar_bytes = Vec::new();
    lzma_rs::xz_decompress(&mut reader, &mut tar_bytes)
        .map_err(|e| format!("decompress {} xz archive: {}", label, e))?;
    let mut archive = tar::Archive::new(Cursor::new(tar_bytes));
    archive
        .unpack(out_dir)
        .map_err(|e| format!("extract {} archive to {}: {}", label, out_dir.display(), e))
}

fn untar_gz_archive(archive_path: &Path, out_dir: &Path, label: &str) -> Result<(), String> {
    let file = std::fs::File::open(archive_path)
        .map_err(|e| format!("open downloaded archive {}: {}", archive_path.display(), e))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(out_dir)
        .map_err(|e| format!("extract {} archive to {}: {}", label, out_dir.display(), e))
}

fn download_tcc() -> Result<PathBuf, String> {
    let cache = cache_dir();
    let (default_url, exe_name, is_zip) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => (TCC_WIN64_URL, "tcc.exe", true),
        ("linux" | "macos", _) if std::env::var("TINYVIM_TCC_URL").is_ok() => ("", "tcc", false),
        ("linux" | "macos", _) => {
            return Err(
                "no built-in TCC binary URL for this platform; TinyVim will try Zig cc instead"
                    .to_string(),
            )
        }
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
    if let Some(path) = find_file_in_manual_tool_roots(exe_name) {
        return Ok(path);
    }

    if is_zip {
        let fallback_archive_name = "tcc.zip";
        let archive_names = vec![
            url_file_name(&url).unwrap_or_else(|| "tcc-0.9.27-win64-bin.zip".to_string()),
            fallback_archive_name.to_string(),
        ];
        let zip_path = find_named_file_in_manual_tool_roots(&archive_names)
            .unwrap_or_else(|| cache.join(fallback_archive_name));
        if !zip_path.exists() {
            download_file(&url, &zip_path).map_err(|e| {
                format!(
                    "download {}: {}. {} Manual fallback: download this TCC package to {} or your Downloads folder, or extract it under either folder. TinyVim will search for {} automatically. You can also set TINYVIM_TCC_URL to a reachable mirror URL, then press F5/F6 again.",
                    url,
                    e,
                    download_failure_hint(&url, &cache),
                    cache.display(),
                    exe_name
                )
            })?;
        }
        unzip_archive(&zip_path, &cache, "TCC")?;
        if zip_path.starts_with(&cache) {
            std::fs::remove_file(&zip_path).ok();
        }
        find_file_in_manual_tool_roots(exe_name).ok_or_else(|| {
            format!(
                "TCC extracted, but {} was not found under {} or your Downloads folder",
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
    let exe_name = if cfg!(target_os = "windows") {
        "tcc.exe"
    } else {
        "tcc"
    };
    find_file_in_manual_tool_roots(exe_name)
}

fn find_named_file_recursive_with_depth(
    root: &Path,
    file_names: &[String],
    max_depth: usize,
) -> Option<PathBuf> {
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    file_names
                        .iter()
                        .any(|candidate| file_name_matches_candidate(name, candidate))
                })
        {
            return Some(path);
        }
        if max_depth > 0 && path.is_dir() {
            if let Some(found) =
                find_named_file_recursive_with_depth(&path, file_names, max_depth - 1)
            {
                return Some(found);
            }
        }
    }
    None
}

fn file_name_matches_candidate(name: &str, candidate: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let candidate = candidate.to_ascii_lowercase();
    if name == candidate {
        return true;
    }

    let Some(suffix) = archive_or_file_suffix(&candidate) else {
        return false;
    };
    let Some(stem) = candidate.strip_suffix(&suffix) else {
        return false;
    };
    let Some(middle) = name
        .strip_prefix(stem)
        .and_then(|rest| rest.strip_suffix(&suffix))
    else {
        return false;
    };
    let middle = middle.trim();
    middle.is_empty()
        || (middle.starts_with('(') && middle.ends_with(')'))
        || (middle.starts_with("-copy") || middle.starts_with("_copy"))
}

fn archive_or_file_suffix(name: &str) -> Option<String> {
    [".tar.gz", ".tar.xz", ".zip", ".exe"]
        .into_iter()
        .find(|suffix| name.ends_with(suffix))
        .map(ToString::to_string)
        .or_else(|| {
            Path::new(name)
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| format!(".{}", ext.to_ascii_lowercase()))
        })
}

fn url_file_name(url: &str) -> Option<String> {
    url.rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
}

fn cached_mingw_gpp(cache: &Path) -> Option<PathBuf> {
    let direct = cache.join("w64devkit").join("bin").join("g++.exe");
    if direct.exists() {
        return Some(direct);
    }
    for (root, max_depth) in manual_tool_roots() {
        if let Some(path) =
            find_named_file_recursive_with_depth(&root, &["g++.exe".to_string()], max_depth).filter(
                |path| {
                    path.components().any(|component| {
                        component
                            .as_os_str()
                            .to_string_lossy()
                            .to_ascii_lowercase()
                            .contains("w64devkit")
                    })
                },
            )
        {
            return Some(path);
        }
    }
    None
}

fn find_mingw_archive() -> Option<PathBuf> {
    let mut archive_names = vec!["w64devkit.zip".to_string()];
    if let Some(default_name) = url_file_name(W64DEVKIT_URL) {
        archive_names.push(default_name);
    }
    find_named_file_in_manual_tool_roots(&archive_names)
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
    let zip_path = find_mingw_archive().unwrap_or_else(|| cache.join("w64devkit.zip"));

    if !zip_path.exists() {
        download_file(&zip_url, &zip_path).map_err(|e| {
            format!(
                "download {}: {}. {} Manual fallback: download this w64devkit zip to {} or your Downloads folder, or extract it under either folder. TinyVim will search for g++.exe automatically. You can also set TINYVIM_W64DEVKIT_URL to a reachable mirror URL, then press F5/F6 again.",
                zip_url,
                e,
                download_failure_hint(&zip_url, &cache),
                cache.display()
            )
        })?;
    }

    unzip_archive(&zip_path, &cache, "w64devkit")?;
    if zip_path.starts_with(&cache) {
        std::fs::remove_file(&zip_path).ok();
    }
    cached_mingw_gpp(&cache).ok_or_else(|| {
        format!(
            "w64devkit extracted, but g++.exe was not found under {} or your Downloads folder",
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
    for (root, max_depth) in manual_tool_roots() {
        if let Some(path) =
            find_named_file_recursive_with_depth(&root, &["zig".to_string()], max_depth)
        {
            return Some(path);
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
    let archive_names = vec![
        url_file_name(&url).unwrap_or_else(|| format!("{}.tar.xz", archive_stem)),
        "zig.tar.xz".to_string(),
    ];
    let archive_path = find_named_file_in_manual_tool_roots(&archive_names)
        .unwrap_or_else(|| cache.join("zig.tar.xz"));
    if !archive_path.exists() {
        download_file(&url, &archive_path).map_err(|e| {
            format!(
                "download {}: {}. {} Manual fallback: download this Zig archive to {} or your Downloads folder, or extract it under either folder. TinyVim will search for zig automatically. You can also set TINYVIM_ZIG_URL to a reachable mirror URL, then press F5/F6 again.",
                url,
                e,
                download_failure_hint(&url, &cache),
                cache.display()
            )
        })?;
    }

    untar_xz_archive(&archive_path, &cache, "Zig")?;

    let zig = find_cached_zig().ok_or_else(|| {
        format!(
            "Zig extracted, but zig executable was not found under {} or your Downloads folder",
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
        "c" | "h" => info.cc.clone(),
        "cpp" | "cc" | "cxx" | "c++" | "hh" | "hpp" | "hxx" => info.cxx.clone(),
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
        "c" | "h" => {
            if let Some(c) = probe_system_compiler(c_cands) {
                return (Some(c), None);
            }
            if let Some(path) = cached_tcc() {
                return (Some(Compiler::Tcc { path }), None);
            }
        }
        "cpp" | "cc" | "cxx" | "c++" | "hh" | "hpp" | "hxx" => {
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
    if allow_download && matches!(ext, "c" | "h") {
        if cfg!(target_os = "windows") {
            match download_tcc() {
                Ok(path) => return (Some(Compiler::Tcc { path }), None),
                Err(e) => download_error = Some(e),
            }
        } else {
            let tcc_error = match download_tcc() {
                Ok(path) => return (Some(Compiler::Tcc { path }), None),
                Err(e) => e,
            };
            match download_zig() {
                Ok(path) => return (Some(Compiler::ZigCc { path }), None),
                Err(e) => {
                    download_error = Some(format!(
                        "TCC fallback failed: {}; Zig cc fallback failed: {}",
                        tcc_error, e
                    ));
                }
            }
        }
    }

    if allow_download
        && cfg!(target_os = "windows")
        && matches!(ext, "cpp" | "cc" | "cxx" | "c++" | "hh" | "hpp" | "hxx")
    {
        match download_mingw() {
            Ok(path) => return (Some(Compiler::W64DevkitCxx { path }), None),
            Err(e) => download_error = Some(e),
        }
    }

    if allow_download
        && !cfg!(target_os = "windows")
        && matches!(ext, "cpp" | "cc" | "cxx" | "c++" | "hh" | "hpp" | "hxx")
    {
        match download_zig() {
            Ok(path) => return (Some(Compiler::ZigCxx { path }), None),
            Err(e) => download_error = Some(e),
        }
    }

    let hint = match (ext, std::env::consts::OS) {
        ("py" | "pyw", os) => {
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
        ("c" | "h", os) => {
            if let Some(error) = download_error {
                let message = if cfg!(target_os = "windows") {
                    format!(
                        "No C compiler, and automatic TCC download failed: {}. Install gcc/clang/MSVC, put the TCC package/executable in the shown cache path, or set TINYVIM_TCC_URL to a reachable TCC package. Cache: {}",
                        error,
                        cache_dir().display()
                    )
                } else {
                    format!(
                        "No C compiler, and automatic Zig cc fallback failed: {}. Install gcc/clang, put zig.tar.xz in the shown cache path, or set TINYVIM_ZIG_URL to a reachable Zig archive. Cache: {}",
                        error,
                        zig_dir().display()
                    )
                };
                return (None, Some(message));
            }
            match os {
                "linux" => "No C compiler. Install: sudo apt install gcc | sudo dnf install gcc | sudo pacman -S gcc",
                "macos" => "No C compiler. Install: xcode-select --install | brew install gcc",
                "windows" => "No C compiler found.",
                _ => "No C compiler found.",
            }
        }
        ("cpp" | "cc" | "cxx" | "c++" | "hh" | "hpp" | "hxx", "linux") => {
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
        ("cpp" | "cc" | "cxx" | "c++" | "hh" | "hpp" | "hxx", "macos") => {
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
        ("cpp" | "cc" | "cxx" | "c++" | "hh" | "hpp" | "hxx", "windows") => {
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
        Compiler::ZigCc { path } => path.as_path(),
        Compiler::ZigCxx { path } => path.as_path(),
        Compiler::Tcc { path } => path.as_path(),
        Compiler::W64DevkitCxx { path } => path.as_path(),
        Compiler::Python { name } => Path::new(name),
        Compiler::UvPython { path } => path.as_path(),
    }
}

fn configure_compiler_environment(command: &mut Command, compiler: &Compiler) {
    if let Compiler::W64DevkitCxx { path } = compiler {
        if let Some(bin_dir) = path.parent() {
            prepend_path(command, bin_dir);
        }
    }
}

fn prepend_path(command: &mut Command, dir: &Path) {
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![dir.to_path_buf()];
    paths.extend(std::env::split_paths(&old_path));
    if let Ok(path) = std::env::join_paths(paths) {
        command.env("PATH", path);
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

fn source_extension(source_path: &str) -> Option<&str> {
    Path::new(source_path)
        .extension()
        .and_then(|ext| ext.to_str())
}

fn is_cpp_source(source_path: &str) -> bool {
    matches!(
        source_extension(source_path),
        Some("cpp" | "cc" | "cxx" | "c++")
    )
}

fn is_cpp_header(source_path: &str) -> bool {
    matches!(source_extension(source_path), Some("hh" | "hpp" | "hxx"))
}

fn is_c_header(source_path: &str) -> bool {
    matches!(source_extension(source_path), Some("h"))
}

fn is_header(source_path: &str) -> bool {
    is_c_header(source_path) || is_cpp_header(source_path)
}

fn is_c_source(source_path: &str) -> bool {
    matches!(source_extension(source_path), Some("c"))
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
    } else if is_header(source_path) && matches!(compiler, Compiler::Msvc { .. }) {
        let mut args = vec!["/nologo".into(), "/Zs".into()];
        args.push(if is_cpp_header(source_path) {
            "/TP".into()
        } else {
            "/TC".into()
        });
        args.push(source_path.into());
        args
    } else if is_header(source_path) && matches!(compiler, Compiler::ZigCc { .. }) {
        vec![
            "cc".into(),
            "-Wall".into(),
            "-fsyntax-only".into(),
            "-x".into(),
            "c-header".into(),
            source_path.into(),
        ]
    } else if is_header(source_path) && matches!(compiler, Compiler::ZigCxx { .. }) {
        vec![
            "c++".into(),
            "-Wall".into(),
            "-fsyntax-only".into(),
            "-x".into(),
            "c++-header".into(),
            source_path.into(),
        ]
    } else if is_header(source_path) && !is_tcc(compiler) {
        vec![
            "-Wall".into(),
            "-fsyntax-only".into(),
            "-x".into(),
            if is_cpp_header(source_path) {
                "c++-header".into()
            } else {
                "c-header".into()
            },
            source_path.into(),
        ]
    } else if is_header(source_path) {
        vec!["-c".into(), source_path.into()]
    } else if matches!(compiler, Compiler::ZigCc { .. }) {
        vec![
            "cc".into(),
            "-Wall".into(),
            "-o".into(),
            out,
            source_path.into(),
            "-lm".into(),
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
    let attempts = if cfg!(target_os = "windows") {
        WINDOWS_ACCESS_RETRY_ATTEMPTS
    } else {
        1
    };

    for attempt in 1..=attempts {
        let mut command = Command::new(compiler_exe(compiler));
        command.args(&all_flags);
        configure_compiler_environment(&mut command, compiler);
        let result =
            run_command_output_timeout(&mut command, BUILD_TIMEOUT, "build/check command")?;
        let mut output = combine_output(&result.stdout, &result.stderr);
        let access_denied = output_mentions_access_denied(&output);

        if !result.status.success() && access_denied && attempt < attempts {
            thread::sleep(WINDOWS_ACCESS_RETRY_DELAY);
            continue;
        }

        if !result.status.success() && access_denied {
            output.push_str("\n\nTinyVim hint: Windows refused access while compiling. The previous program may still be running or antivirus may still be scanning the freshly built/downloaded executable. TinyVim retried automatically; close the running program window, wait a moment, or allow the project/cache folder in Windows Security, then press F5/F6 again.");
        }

        return Ok(BuildResult {
            success: result.status.success(),
            output,
            command_line: cmd_line,
        });
    }

    unreachable!("compile retry loop always returns")
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
    fn manual_archive_match_accepts_browser_duplicate_names() {
        assert!(file_name_matches_candidate(
            "uv-x86_64-pc-windows-msvc (1).zip",
            "uv-x86_64-pc-windows-msvc.zip"
        ));
        assert!(file_name_matches_candidate(
            "zig-x86_64-linux-0.15.2 (2).tar.xz",
            "zig-x86_64-linux-0.15.2.tar.xz"
        ));
        assert!(file_name_matches_candidate(
            "uv-x86_64-unknown-linux-musl (1).tar.gz",
            "uv-x86_64-unknown-linux-musl.tar.gz"
        ));
    }

    #[test]
    fn manual_archive_match_rejects_unrelated_files() {
        assert!(!file_name_matches_candidate(
            "not-uv-x86_64-pc-windows-msvc.zip",
            "uv-x86_64-pc-windows-msvc.zip"
        ));
        assert!(!file_name_matches_candidate(
            "w64devkit-1.21.0.zip",
            "w64devkit-1.22.0.zip"
        ));
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
