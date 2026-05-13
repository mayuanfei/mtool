use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use zip::ZipArchive;

pub fn ensure_cfr() -> Result<PathBuf, String> {
    let mut cfr_path = std::env::temp_dir();
    cfr_path.push("cfr-0.152.jar");
    
    if !cfr_path.exists() {
        let url = "https://github.com/leibnitz27/cfr/releases/download/0.152/cfr-0.152.jar";
        let response = reqwest::blocking::get(url).map_err(|e| format!("Failed to download CFR: {}", e))?;
        let bytes = response.bytes().map_err(|e| format!("Failed to read CFR bytes: {}", e))?;
        fs::write(&cfr_path, bytes).map_err(|e| format!("Failed to write CFR: {}", e))?;
    }
    
    Ok(cfr_path)
}

pub fn decompile_class(class_file_path: &Path) -> Result<String, String> {
    let cfr_path = ensure_cfr()?;
    
    let output = Command::new("java")
        .arg("-jar")
        .arg(&cfr_path)
        .arg(class_file_path)
        .output()
        .map_err(|e| format!("Failed to execute java (is it installed?): {}", e))?;
        
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(format!("Decompile failed: {}", String::from_utf8_lossy(&output.stderr)))
    }
}

#[tauri::command]
pub fn list_jar_entries(path: &str) -> Result<Vec<String>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for i in 0..archive.len() {
        if let Ok(file) = archive.by_index(i) {
            entries.push(file.name().to_string());
        }
    }
    Ok(entries)
}

#[tauri::command]
pub fn read_jar_entry(jar_path: &str, entry_name: &str) -> Result<String, String> {
    let file = File::open(jar_path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut entry = archive.by_name(entry_name).map_err(|e| e.to_string())?;
    
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    
    if entry_name.ends_with(".class") {
        let temp_dir = std::env::temp_dir();
        let temp_class = temp_dir.join(format!("mtool_temp_{}.class", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::write(&temp_class, &buf).map_err(|e| e.to_string())?;
        
        let result = decompile_class(&temp_class);
        let _ = fs::remove_file(temp_class);
        result
    } else {
        Ok(String::from_utf8_lossy(&buf).to_string())
    }
}

#[tauri::command]
pub fn read_local_class(path: &str) -> Result<String, String> {
    let path_obj = Path::new(path);
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
