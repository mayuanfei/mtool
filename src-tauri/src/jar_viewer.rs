use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use sha2::{Sha256, Digest};
use zip::ZipArchive;

pub fn ensure_cfr() -> Result<PathBuf, String> {
    let mut cfr_path = std::env::temp_dir();
    cfr_path.push("cfr-0.152.jar");
    
    let expected_sha256 = "f686e8f3ded377d7bc87d216a90e9e9512df4156e75b06c655a16648ae8765b2";

    if cfr_path.exists() {
        if let Ok(bytes) = fs::read(&cfr_path) {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let hash = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>();
            if hash == expected_sha256 {
                return Ok(cfr_path);
            }
        }
    }
    
    let url = "https://github.com/leibnitz27/cfr/releases/download/0.152/cfr-0.152.jar";
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build client: {}", e))?;
        
    let response = client.get(url).send().map_err(|e| format!("Failed to download CFR: {}", e))?;
    let bytes = response.bytes().map_err(|e| format!("Failed to read CFR bytes: {}", e))?;
    
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let hash = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>();
    
    if hash != expected_sha256 {
        return Err(format!("CFR jar integrity check failed. Expected {}, got {}", expected_sha256, hash));
    }
    
    let temp_download_path = cfr_path.with_file_name(format!("cfr-0.152-{:x}.tmp", rand::random::<u64>()));
    fs::write(&temp_download_path, &bytes).map_err(|e| format!("Failed to write temp CFR: {}", e))?;
    let _ = fs::rename(&temp_download_path, &cfr_path);
    
    Ok(cfr_path)
}

pub fn decompile_class(class_file_path: &Path) -> Result<String, String> {
    let cfr_path = ensure_cfr()?;
    
    let mut child = Command::new("java")
        .arg("-jar")
        .arg(&cfr_path)
        .arg(class_file_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to execute java (is it installed?): {}", e))?;

    // Extract pipe handles BEFORE wrapping child in Arc.
    // These are independent owned handles; reading them doesn't need the child lock.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    // Child stays inside the Mutex at all times so the main thread can always kill it.
    let child = Arc::new(Mutex::new(child));
    let child_for_thread = Arc::clone(&child);

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // Read pipes (no lock needed — these are separate owned handles).
        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        if let Some(mut out) = stdout_pipe {
            let _ = out.read_to_end(&mut stdout_buf);
        }
        if let Some(mut err) = stderr_pipe {
            let _ = err.read_to_end(&mut stderr_buf);
        }

        // Wait for process exit through the Mutex (child is NOT taken out).
        let status = child_for_thread
            .lock()
            .ok()
            .and_then(|mut guard| guard.wait().ok());

        let _ = tx.send((status, stdout_buf, stderr_buf));
    });

    match rx.recv_timeout(Duration::from_secs(30)) {
        Ok((Some(status), stdout_buf, stderr_buf)) => {
            if status.success() {
                Ok(String::from_utf8_lossy(&stdout_buf).to_string())
            } else {
                Err(format!("Decompile failed: {}", String::from_utf8_lossy(&stderr_buf)))
            }
        }
        Ok((None, _, _)) => Err("Failed to wait for decompilation process".to_string()),
        Err(_) => {
            // Timeout: child is still inside the Mutex, kill is guaranteed to reach it.
            if let Ok(mut guard) = child.lock() {
                let _ = guard.kill();
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
pub async fn read_jar_entry(jar_path: String, entry_name: String) -> Result<String, String> {
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
            
            let result = decompile_class(&temp_class);
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
pub async fn read_local_class(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path_obj = Path::new(&path);
        if path.ends_with(".class") {
            decompile_class(path_obj)
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
pub fn open_jar_or_class() -> Result<(String, String), String> {
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
}
