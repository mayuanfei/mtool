use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;
use tauri::Manager;
use zip::ZipArchive;

static CFR_PATH: OnceLock<PathBuf> = OnceLock::new();

const EXPECTED_SHA256: &str = "f686e8f3ded377d7bc87d216a90e9e9512df4156e75b06c655a16648ae8765b2";

pub fn ensure_cfr(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    // 1. Check OnceLock for cached path
    if let Some(path) = CFR_PATH.get() {
        return Ok(path.clone());
    }

    // 2. Resolve bundled CFR (with download fallback if not bundled)
    let bundled = resolve_bundled_cfr(app_handle)?;

    // 3. Handle UNC paths for Java compatibility (VM scenarios)
    let cfr_path = if is_unc_path(&bundled) {
        copy_to_temp_if_needed(&bundled)?
    } else {
        bundled
    };

    let _ = CFR_PATH.set(cfr_path.clone());
    Ok(cfr_path)
}

fn is_unc_path(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        match path.components().next() {
            Some(Component::Prefix(p)) => matches!(p.kind(), Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _)),
            _ => false,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

fn verify_sha256(path: &Path) -> bool {
    if !path.exists() { return false; }
    let Ok(mut file) = File::open(path) else { return false; };
    let mut hasher = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buffer[..n]),
            Err(_) => return false,
        }
    }
    let hash = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>();
    hash == EXPECTED_SHA256
}

fn resolve_bundled_cfr(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    // "resources/cfr-0.152.jar" — Tauri 标准打包布局：资源文件位于应用程序包的 resources/ 子目录
    // "cfr-0.152.jar"            — Tauri resource root 备用：部分打包配置会将资源直接放到根目录
    let possible_rel_paths = ["resources/cfr-0.152.jar", "cfr-0.152.jar"];
    for rel_path in possible_rel_paths {
        if let Ok(path) = app_handle.path().resolve(rel_path, tauri::path::BaseDirectory::Resource) {
            if path.exists() && verify_sha256(&path) {
                return Ok(path);
            }
        }
    }
    
    // Fallback: Check local temp (maybe downloaded previously)
    let temp_path = std::env::temp_dir().join("mtool_cfr_0.152.jar");
    if verify_sha256(&temp_path) {
        return Ok(temp_path);
    }
    
    // Final fallback: Download to local temp
    download_cfr(&temp_path)
}

fn copy_to_temp_if_needed(src: &Path) -> Result<PathBuf, String> {
    let dest = std::env::temp_dir().join("mtool_cfr_0.152.jar");
    if !verify_sha256(&dest) {
        fs::copy(src, &dest).map_err(|e| format!("Failed to copy CFR to temp for UNC support: {}", e))?;
    }
    Ok(dest)
}

fn download_cfr(dest: &Path) -> Result<PathBuf, String> {
    let url = "https://github.com/leibnitz27/cfr/releases/download/0.152/cfr-0.152.jar";
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build download client: {}", e))?;
        
    let response = client.get(url).send().map_err(|e| format!("Failed to download CFR from GitHub: {}. Please check your internet connection.", e))?;
    let bytes = response.bytes().map_err(|e| format!("Failed to read CFR bytes: {}", e))?;
    
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let hash = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>();
    
    if hash != EXPECTED_SHA256 {
        return Err(format!("CFR jar integrity check failed. Expected {}, got {}", EXPECTED_SHA256, hash));
    }
    
    fs::write(dest, &bytes).map_err(|e| format!("Failed to write CFR to local temp: {}", e))?;
    Ok(dest.to_path_buf())
}


pub fn decompile_class(app_handle: &tauri::AppHandle, class_file_path: &Path) -> Result<String, String> {
    let cfr_path = ensure_cfr(app_handle)?;

    let mut command = Command::new("java");
    command
        .arg("-jar")
        .arg(&cfr_path)
        .arg(class_file_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let child = command
        .spawn()
        .map_err(|e| format!("Failed to execute 'java'. Please ensure Java (JRE/JDK) is installed and available in your PATH. Error: {}", e))?;


    // Save PID before moving child into the thread.
    // On timeout we kill via OS signal using the PID — no shared state, no locks.
    let pid = child.id();

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(Ok(output)) => {
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            } else {
                Err(format!("Decompile failed: {}", String::from_utf8_lossy(&output.stderr)))
            }
        }
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => {
            // Timeout: kill the java process via PID (lock-free, no deadlock risk).
            #[cfg(unix)]
            if let Ok(pid_i32) = pid.try_into() {
                unsafe { libc::kill(pid_i32, libc::SIGKILL); }
            }
            #[cfg(windows)]
            unsafe {
                use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
                use windows_sys::Win32::Foundation::CloseHandle;
                let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
                if handle != 0 as _ {
                    TerminateProcess(handle, 1);
                    CloseHandle(handle);
                }
            }
            Err("Decompilation timed out (30s)".to_string())
        }
    }
}

#[tauri::command]
pub async fn list_jar_entries(path: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let file = File::open(&path).map_err(|e| e.to_string())?;
        let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        for i in 0..archive.len() {
            if let Ok(file) = archive.by_index(i) {
                entries.push(file.name().to_string());
            }
        }
        Ok(entries)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn read_jar_entry(app_handle: tauri::AppHandle, jar_path: String, entry_name: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let file = File::open(&jar_path).map_err(|e| e.to_string())?;
        let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
        let mut entry = archive.by_name(&entry_name).map_err(|e| e.to_string())?;
        
        if entry.size() > 10 * 1024 * 1024 {
            return Err(format!("Entry too large ({} MB). Max 10 MB.", entry.size() / 1024 / 1024));
        }

        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        
        if entry_name.ends_with(".class") {
            let temp_dir = std::env::temp_dir();
            let temp_class = temp_dir.join(format!("mtool_temp_{:x}.class", rand::random::<u64>()));
            fs::write(&temp_class, &buf).map_err(|e| e.to_string())?;
            
            let result = decompile_class(&app_handle, &temp_class);
            let _ = fs::remove_file(temp_class);
            result
        } else {
            Ok(String::from_utf8_lossy(&buf).to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn read_local_class(app_handle: tauri::AppHandle, path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path_obj = Path::new(&path);
        if path.ends_with(".class") {
            // 将 .class 文件复制到本地 temp 目录再传给 java，
            // 避免 VM 共享目录 UNC 路径（如 \\Mac\Home\...）导致 java 无法读取输入文件
            let bytes = fs::read(path_obj).map_err(|e| e.to_string())?;
            let temp_class = std::env::temp_dir()
                .join(format!("mtool_temp_{:x}.class", rand::random::<u64>()));
            fs::write(&temp_class, &bytes).map_err(|e| e.to_string())?;
            let result = decompile_class(&app_handle, &temp_class);
            let _ = fs::remove_file(&temp_class);
            result
        } else {
            let metadata = fs::metadata(path_obj).map_err(|e| e.to_string())?;
            if metadata.len() > 10 * 1024 * 1024 {
                return Err(format!(
                    "File too large ({} MB). Max 10 MB.",
                    metadata.len() / 1024 / 1024
                ));
            }
            fs::read_to_string(path_obj).map_err(|e| e.to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn open_jar_or_class() -> Result<(String, String), String> {
    tauri::async_runtime::spawn_blocking(|| {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Archive/Class/Text Files", &[
                "jar", "zip", "class", "txt", "md", "markdown", "yaml", "yml", "json", "jsonc", "json5",
                "xml", "html", "htm", "css", "scss", "less", "js", "jsx", "ts",
                "tsx", "csv", "tsv", "log", "ini", "cfg", "conf", "toml", "env",
                "sh", "bash", "zsh", "bat", "cmd", "ps1", "py", "rb", "java",
                "c", "cpp", "h", "hpp", "go", "rs", "swift", "kt", "sql", "graphql",
                "properties", "vue", "svelte",
            ])
            .pick_file()
        {
            let ext = path.extension().unwrap_or_default().to_string_lossy().to_string();
            Ok((path.to_string_lossy().to_string(), ext))
        } else {
            Err("No file selected".to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
