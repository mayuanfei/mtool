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

pub fn ensure_cfr(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    let expected_sha256 = "f686e8f3ded377d7bc87d216a90e9e9512df4156e75b06c655a16648ae8765b2";
    
    // Always use a path in the local temp directory for execution.
    // This avoids issues with Java and UNC paths (e.g. in macOS VMs).
    let mut local_cfr_path = std::env::temp_dir();
    local_cfr_path.push("mtool_cfr_0.152.jar");

    // 1. If it already exists in temp and is valid, use it.
    if local_cfr_path.exists() {
        if let Ok(bytes) = fs::read(&local_cfr_path) {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let hash = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>();
            if hash == expected_sha256 {
                return Ok(local_cfr_path);
            }
        }
    }

    // 2. Try to find bundled CFR and copy it to temp.
    let possible_rel_paths = ["resources/cfr-0.152.jar", "cfr-0.152.jar", "_up_/resources/cfr-0.152.jar"];
    for rel_path in possible_rel_paths {
        if let Ok(bundled_path) = app_handle.path().resolve(rel_path, tauri::path::BaseDirectory::Resource) {
            if bundled_path.exists() {
                if let Ok(bytes) = fs::read(&bundled_path) {
                    let mut hasher = Sha256::new();
                    hasher.update(&bytes);
                    let hash = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>();
                    if hash == expected_sha256 {
                        // Found valid bundled jar, copy to local temp for execution
                        if fs::write(&local_cfr_path, &bytes).is_ok() {
                            return Ok(local_cfr_path);
                        }
                    }
                }
            }
        }
    }

    // 3. Fallback: Download to local temp.
    let url = "https://github.com/leibnitz27/cfr/releases/download/0.152/cfr-0.152.jar";
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build client for download: {}", e))?;
        
    let response = client.get(url).send().map_err(|e| format!("Failed to download CFR from GitHub: {}. Please check your internet connection.", e))?;
    let bytes = response.bytes().map_err(|e| format!("Failed to read CFR bytes: {}", e))?;
    
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let hash = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>();
    
    if hash != expected_sha256 {
        return Err(format!("CFR jar integrity check failed. Expected {}, got {}", expected_sha256, hash));
    }
    
    fs::write(&local_cfr_path, &bytes).map_err(|e| format!("Failed to write CFR to local temp: {}", e))?;
    
    Ok(local_cfr_path)
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
            unsafe { libc::kill(pid as i32, libc::SIGKILL); }
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
            decompile_class(&app_handle, path_obj)
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
