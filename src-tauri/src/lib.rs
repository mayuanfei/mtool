mod file_search;
mod jar_viewer;
mod file_transfer;
use base64::prelude::*;
use std::fs;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use arboard::{Clipboard, ImageData};
use image::Rgb;
use notify::event::{ModifyKind, RenameMode};
use notify::{EventKind, RecursiveMode, Watcher};
use qrcode::{EcLevel, QrCode};

use tauri::{Emitter, Manager};
use rayon::prelude::*;

use file_search::{
    build_index_streaming, content_matches,
    count_entries, delete_entry_in_db, entry_from_path,
    get_db_path, init_db, is_index_disabled, parse_query,
    rebuild_fts5_background, search_in_db, set_last_built_at,
    should_skip_path, upsert_entry_in_db, FileEntry, IndexEngine,
    IndexStatus, get_watch_roots_pub,
};

fn maybe_start_watcher(engine: &IndexEngine, has_data: bool, index_disabled: bool) {
    if index_disabled || !has_data {
        return;
    }
    if !engine.watcher_stopped.load(Ordering::Relaxed) {
        return;
    }
    let disabled = engine.disabled.clone();
    let shutdown = engine.shutdown.clone();
    let watcher_stopped = engine.watcher_stopped.clone();
    start_fs_watcher(engine.db_path.clone(), disabled, shutdown, watcher_stopped);
}

// ---------------------------------------------------------------------------
// 现有命令
// ---------------------------------------------------------------------------

#[tauri::command]
fn format_json(input: &str) -> Result<String, String> {
    if input.len() > 5 * 1024 * 1024 {
        return Err("JSON too large (max 5 MB)".to_string());
    }
    let value: serde_json::Value =
        serde_json::from_str(input).map_err(|e| format!("Invalid JSON: {}", e))?;
    serde_json::to_string_pretty(&value).map_err(|e| format!("Format error: {}", e))
}

#[tauri::command]
fn minify_json(input: &str) -> Result<String, String> {
    if input.len() > 5 * 1024 * 1024 {
        return Err("JSON too large (max 5 MB)".to_string());
    }
    let value: serde_json::Value =
        serde_json::from_str(input).map_err(|e| format!("Invalid JSON: {}", e))?;
    serde_json::to_string(&value).map_err(|e| format!("Minify error: {}", e))
}

#[tauri::command]
fn generate_qr(payload: &str, redundancy: &str, resolution: u32, color: &str, bg_color: &str) -> Result<String, String> {
    if payload.is_empty() {
        return Err("Payload is empty".to_string());
    }
    let resolution = resolution.clamp(64, 2048);
    let ec_level = match redundancy {
        "L" => EcLevel::L, "M" => EcLevel::M, "Q" => EcLevel::Q, "H" => EcLevel::H,
        _ => EcLevel::M,
    };
    let code = QrCode::with_error_correction_level(payload, ec_level)
        .map_err(|e| format!("QR Code generation failed: {}", e))?;
    let hex = color.trim_start_matches('#');
    let r = u8::from_str_radix(hex.get(0..2).unwrap_or("00"), 16).unwrap_or(0);
    let g = u8::from_str_radix(hex.get(2..4).unwrap_or("00"), 16).unwrap_or(0);
    let b = u8::from_str_radix(hex.get(4..6).unwrap_or("00"), 16).unwrap_or(0);
    let bghex = bg_color.trim_start_matches('#');
    let bg_r = u8::from_str_radix(bghex.get(0..2).unwrap_or("ff"), 16).unwrap_or(255);
    let bg_g = u8::from_str_radix(bghex.get(2..4).unwrap_or("ff"), 16).unwrap_or(255);
    let bg_b = u8::from_str_radix(bghex.get(4..6).unwrap_or("ff"), 16).unwrap_or(255);
    let image = code
        .render::<Rgb<u8>>()
        .min_dimensions(resolution, resolution)
        .dark_color(Rgb([r, g, b]))
        .light_color(Rgb([bg_r, bg_g, bg_b]))
        .build();
    let mut buf = Vec::new();
    let mut cursor = Cursor::new(&mut buf);
    image
        .write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| format!("Failed to write PNG: {}", e))?;
    Ok(BASE64_STANDARD.encode(&buf))
}

#[tauri::command]
fn read_text_from_clipboard() -> Result<String, String> {
    let mut clipboard =
        Clipboard::new().map_err(|e| format!("Failed to init clipboard: {}", e))?;
    clipboard
        .get_text()
        .map_err(|e| format!("Clipboard read error: {}", e))
}

#[tauri::command]
fn copy_qr_to_clipboard(base64_str: &str) -> Result<(), String> {
    let mut clipboard =
        Clipboard::new().map_err(|e| format!("Failed to init clipboard: {}", e))?;
    let img_bytes = BASE64_STANDARD
        .decode(base64_str)
        .map_err(|e| format!("Base64 decode error: {}", e))?;
    let img = image::load_from_memory(&img_bytes)
        .map_err(|e| format!("Image load error: {}", e))?;
    let img_rgba = img.to_rgba8();
    let img_data = ImageData {
        width: img_rgba.width() as usize,
        height: img_rgba.height() as usize,
        bytes: img_rgba.into_raw().into(),
    };
    clipboard
        .set_image(img_data)
        .map_err(|e| format!("Clipboard write error: {}", e))?;
    Ok(())
}

#[tauri::command]
fn download_qr(base64_str: &str) -> Result<(), String> {
    let img_bytes = BASE64_STANDARD
        .decode(base64_str)
        .map_err(|e| format!("Base64 decode error: {}", e))?;
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("PNG Image", &["png"])
        .set_file_name("qrcode.png")
        .save_file()
    {
        fs::write(path, img_bytes).map_err(|e| format!("Failed to save file: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
fn open_md_file() -> Result<(String, String), String> {
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("Markdown", &["md", "markdown", "txt"])
        .pick_file()
    {
        let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        if metadata.len() > 10 * 1024 * 1024 {
            return Err(format!(
                "File too large ({} MB). Max 10 MB.",
                metadata.len() / 1024 / 1024
            ));
        }
        let content = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read file: {}", e))?
            .replace("\r\n", "\n");
        let path_str = path.to_string_lossy().to_string();
        Ok((path_str, content))
    } else {
        Err("No file selected".to_string())
    }
}

#[tauri::command]
fn save_md_file(path: &str, content: &str) -> Result<String, String> {
    if content.len() > 10 * 1024 * 1024 {
        return Err("Content too large (max 10 MB)".to_string());
    }
    if path.is_empty() {
        if let Some(save_path) = rfd::FileDialog::new()
            .add_filter("Markdown", &["md", "markdown"])
            .set_file_name("untitled.md")
            .save_file()
        {
            fs::write(&save_path, content)
                .map_err(|e| format!("Failed to save file: {}", e))?;
            Ok(save_path.to_string_lossy().to_string())
        } else {
            Err("Save cancelled".to_string())
        }
    } else {
        fs::write(path, content).map_err(|e| format!("Failed to save file: {}", e))?;
        Ok(path.to_string())
    }
}

#[tauri::command]
fn save_md_file_as(content: &str) -> Result<String, String> {
    if content.len() > 10 * 1024 * 1024 {
        return Err("Content too large (max 10 MB)".to_string());
    }
    if let Some(save_path) = rfd::FileDialog::new()
        .add_filter("Markdown", &["md", "markdown"])
        .set_file_name("untitled.md")
        .save_file()
    {
        fs::write(&save_path, content)
            .map_err(|e| format!("Failed to save file: {}", e))?;
        Ok(save_path.to_string_lossy().to_string())
    } else {
        Err("Save cancelled".to_string())
    }
}

#[tauri::command]
fn open_md_file_by_path(path: &str) -> Result<(String, String), String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if metadata.len() > 10 * 1024 * 1024 {
        return Err(format!(
            "File too large ({} MB). Max 10 MB.",
            metadata.len() / 1024 / 1024
        ));
    }
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read file: {}", e))?
        .replace("\r\n", "\n");
    Ok((path.to_string(), content))
}

#[tauri::command]
fn open_text_file() -> Result<(String, String), String> {
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("Text Files", &[
            "txt", "md", "markdown", "yaml", "yml", "json", "jsonc", "json5",
            "xml", "html", "htm", "css", "scss", "less", "js", "jsx", "ts",
            "tsx", "csv", "tsv", "log", "ini", "cfg", "conf", "toml", "env",
            "sh", "bash", "zsh", "bat", "cmd", "ps1", "py", "rb", "java",
            "c", "cpp", "h", "hpp", "go", "rs", "swift", "kt", "sql", "graphql",
            "properties", "gitignore", "dockerignore", "editorconfig", "eslintrc",
            "prettierrc", "babelrc", "npmrc", "lock", "vue", "svelte",
        ])
        .pick_file()
    {
        let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        if metadata.len() > 10 * 1024 * 1024 {
            return Err(format!(
                "File too large ({} MB). Max 10 MB.",
                metadata.len() / 1024 / 1024
            ));
        }
        let content = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read file: {}", e))?
            .replace("\r\n", "\n");
        let path_str = path.to_string_lossy().to_string();
        Ok((path_str, content))
    } else {
        Err("No file selected".to_string())
    }
}

#[tauri::command]
fn read_text_file_by_path(path: &str) -> Result<(String, String), String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if metadata.len() > 10 * 1024 * 1024 {
        return Err(format!(
            "File too large ({} MB). Max 10 MB.",
            metadata.len() / 1024 / 1024
        ));
    }
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read file: {}", e))?
        .replace("\r\n", "\n");
    Ok((path.to_string(), content))
}

// ---------------------------------------------------------------------------
// 文件搜索命令
// ---------------------------------------------------------------------------

#[tauri::command]
async fn build_index(
    app: tauri::AppHandle,
    state: tauri::State<'_, IndexEngine>,
) -> Result<usize, String> {
    if state.is_building.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        return Err("Index build already in progress".to_string());
    }

    let status_ref = state.status.clone();
    let db_path = state.db_path.clone();

    // 重置禁用标志
    state.disabled.store(false, Ordering::Relaxed);
    state.fts5_ready.store(false, Ordering::Relaxed);

    // 先停掉 watcher，等它真正退出
    state.shutdown.store(true, Ordering::Relaxed);
    let watcher_stopped_clone = state.watcher_stopped.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let mut attempts = 0;
        while !watcher_stopped_clone.load(Ordering::Relaxed) && attempts < 500 {
            std::thread::sleep(Duration::from_millis(10));
            attempts += 1;
        }
        if attempts >= 500 {
            eprintln!("[mtool] build_index: timeout waiting for watcher to stop");
        }
    }).await;

    {
        let mut s = status_ref.write().unwrap_or_else(|e| e.into_inner());
        s.is_indexing = true;
        s.total = 0;
    }

    let app_clone = app.clone();
    let status_ref_clone = status_ref.clone();
    let db_fts = db_path.clone();
    let total = match tauri::async_runtime::spawn_blocking(move || -> Result<usize, String> {
        // 物理删除旧库文件，避免 SQLite DROP TABLE 耗时阻塞
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(format!("{}-wal", db_path));
        let _ = std::fs::remove_file(format!("{}-shm", db_path));
        // 重新初始化空库结构
        file_search::init_db(&db_path)?;

        let total = build_index_streaming(&db_path, move |count, _path| {
            if let Ok(mut s) = status_ref_clone.write() {
                s.total = count;
            }
            app_clone.emit("index_progress", count).ok();
        });
        let now_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        set_last_built_at(&db_path, now_ts);
        Ok(total)
    })
    .await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => {
            let mut s = status_ref.write().unwrap_or_else(|e| e.into_inner());
            s.is_indexing = false;
            state.is_building.store(false, Ordering::Release);
            return Err(e);
        }
        Err(e) => {
            let mut s = status_ref.write().unwrap_or_else(|e| e.into_inner());
            s.is_indexing = false;
            state.is_building.store(false, Ordering::Release);
            return Err(e.to_string());
        }
    };

    {
        let mut s = status_ref.write().unwrap_or_else(|e| e.into_inner());
        s.is_indexing = false;
        s.total = total;
        s.last_built_at = file_search::get_last_built_at(&state.db_path);
    }

    state.load_from_db();

    app.emit("index_progress", total).ok();
    app.emit("index_complete", total).ok();

    // FTS5 后台重建 + watcher 启动（顺序执行，避免写写竞争）
    let fts5_ready_clone = state.fts5_ready.clone();
    state.shutdown.store(false, Ordering::Relaxed);
    let disabled = state.disabled.clone();
    let shutdown = state.shutdown.clone();
    let db_path_w = state.db_path.clone();
    let watcher_stopped = state.watcher_stopped.clone();
    let is_building_clone = state.is_building.clone();
    std::thread::spawn(move || {
        // 1. 先完成 FTS5 重建
        if rebuild_fts5_background(&db_fts) {
            fts5_ready_clone.store(true, Ordering::Relaxed);
        }
        // 2. 再启动 watcher（避免与 FTS5 重建产生写写竞争）
        start_fs_watcher(db_path_w, disabled, shutdown, watcher_stopped);
        is_building_clone.store(false, Ordering::Release);
    });

    Ok(total)
}

#[tauri::command]
fn get_index_status(state: tauri::State<'_, IndexEngine>) -> IndexStatus {
    state.status.read().unwrap_or_else(|e| e.into_inner()).clone()
}

#[tauri::command]
async fn search_files(
    query: String,
    mut limit: usize,
    state: tauri::State<'_, IndexEngine>,
) -> Result<Vec<FileEntry>, String> {
    if query.len() > 1024 {
        return Err("Query too long (max 1024 chars)".to_string());
    }
    limit = limit.min(2_000);
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let parsed = parse_query(&query);
    let db_path = state.db_path.clone();
    let fts5_ready = state.fts5_ready.load(Ordering::Relaxed);

    // ── 非 content 查询：FTS5 trigram 或 SQLite LIKE ────────────────────────────
    if parsed.content_filter.is_none() {
        return tauri::async_runtime::spawn_blocking(move || {
            search_in_db(&db_path, &parsed, limit, fts5_ready)
        })
        .await
        .map_err(|e| e.to_string());
    }

    // ── 纯 content: 搜索（无文件名/glob 限定）→ 直接拒绝，避免只扫前 10 万行 ──
    if parsed.name_terms.is_empty() && parsed.glob_pattern.is_none() {
        return Err("ERR_CONTENT_REQUIRES_FILENAME".to_string());
    }

    // ── content 过滤 → SQLite 取候选，rayon 并行扫文件内容 ─────────────────────
    let content_needle = parsed.content_filter.as_ref()
        .expect("content_filter checked above")
        .as_bytes().to_vec();
    let results = tauri::async_runtime::spawn_blocking(move || {
        let candidates = search_in_db(&db_path, &parsed, 100_000, fts5_ready);
        let mut matched: Vec<FileEntry> = candidates
            .into_par_iter()
            .filter(|e| {
                !e.is_dir 
                && !should_skip_path(std::path::Path::new(&e.path)) 
                && content_matches(&e.path, &content_needle)
            })
            .collect();
        matched.sort_unstable_by(|a, b| b.modified.cmp(&a.modified));
        matched.truncate(limit);
        matched
    })
    .await
    .map_err(|e| e.to_string())?;

    Ok(results)
}

#[tauri::command]
async fn disable_file_search(
    state: tauri::State<'_, IndexEngine>,
) -> Result<(), String> {
    {
        let mut s = state.status.write().unwrap_or_else(|e| e.into_inner());
        s.is_indexing = false;
        s.total = 0;
        s.last_built_at = None;
    }
    state.disabled.store(true, Ordering::Relaxed);
    state.shutdown.store(true, Ordering::Relaxed);
    state.fts5_ready.store(false, Ordering::Relaxed);
    state.is_building.store(false, Ordering::Release);

    let db_path = state.db_path.clone();
    let watcher_stopped = state.watcher_stopped.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 等 watcher 线程退出，避免 watcher 访问正在被清空的表
        let mut attempts = 0;
        while !watcher_stopped.load(Ordering::Relaxed) && attempts < 500 {
            std::thread::sleep(Duration::from_millis(10));
            attempts += 1;
        }
        if attempts >= 500 {
            eprintln!("[mtool] disable_file_search: timeout waiting for watcher to stop");
        }
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(format!("{}-wal", db_path));
        let _ = std::fs::remove_file(format!("{}-shm", db_path));
        if let Err(e) = file_search::init_db(&db_path) {
            eprintln!("[mtool] failed to init db in disable_file_search: {}", e);
        }
        // 持久化禁用状态，下次启动时跳过自动建索引和 watcher
        if let Ok(conn) = rusqlite::Connection::open(&db_path) {
            conn.execute(
                "INSERT OR REPLACE INTO index_meta(key,value) VALUES('disabled','1')",
                [],
            )
            .ok();
        }
    })
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
fn reveal_in_explorer(path: String) -> Result<(), String> {
    if !std::path::Path::new(&path).exists() {
        return Err("File or directory does not exist".to_string());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let clean_path = path.replace("/", "\\");
        std::process::Command::new("explorer")
            .raw_arg(format!(r#"/select,"{}""#, clean_path))
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        let parent = std::path::Path::new(&path)
            .parent()
            .unwrap_or(std::path::Path::new("/"));
        std::process::Command::new("xdg-open")
            .arg(parent)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn open_file(path: String) -> Result<(), String> {
    if !std::path::Path::new(&path).exists() {
        return Err("File or directory does not exist".to_string());
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        std::process::Command::new("cmd")
            .args(["/c", "start", "", &path])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// Helper to retrieve the local path to Pandoc in mtool's local application directory.
fn get_internal_pandoc_path(app_handle: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let app_dir = app_handle.path().app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    let bin_dir = app_dir.join("bin");
    let exe_name = if cfg!(windows) { "pandoc.exe" } else { "pandoc" };
    Ok(bin_dir.join(exe_name))
}

#[tauri::command]
async fn check_pandoc(app_handle: tauri::AppHandle) -> Result<String, String> {
    // 1. Check internal storage path
    if let Ok(internal_path) = get_internal_pandoc_path(&app_handle) {
        if internal_path.exists() {
            let output = std::process::Command::new(&internal_path)
                .arg("--version")
                .output();
            if let Ok(out) = output {
                if out.status.success() {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    if let Some(first_line) = stdout.lines().next() {
                        return Ok(format!("internal:{}", first_line));
                    }
                }
            }
        }
    }

    // 2. Check system PATH
    let output = std::process::Command::new("pandoc")
        .arg("--version")
        .output();
    
    if let Ok(out) = output {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if let Some(first_line) = stdout.lines().next() {
                return Ok(format!("system:{}", first_line));
            }
        }
    }

    Ok("not_installed".to_string())
}

#[derive(serde::Serialize, Clone)]
struct InstallProgress {
    stage: String,   // "not_started" | "downloading" | "extracting" | "success" | "failed"
    progress: u32,   // 0 - 100
    message: String,
}

use std::sync::Arc;
use std::sync::Mutex;

#[derive(Clone)]
struct PandocInstallState(Arc<PandocInstallStateInner>);

struct PandocInstallStateInner {
    is_installing: Mutex<bool>,
    current_progress: Mutex<InstallProgress>,
}

impl PandocInstallState {
    fn new() -> Self {
        Self(Arc::new(PandocInstallStateInner {
            is_installing: Mutex::new(false),
            current_progress: Mutex::new(InstallProgress {
                stage: "not_started".to_string(),
                progress: 0,
                message: "".to_string(),
            }),
        }))
    }
}

#[tauri::command]
fn get_pandoc_install_status(state: tauri::State<'_, PandocInstallState>) -> InstallProgress {
    state.0.current_progress.lock().unwrap().clone()
}

#[tauri::command]
async fn install_pandoc(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, PandocInstallState>
) -> Result<(), String> {
    {
        let mut is_installing = state.0.is_installing.lock().unwrap();
        if *is_installing {
            return Err("Installation is already in progress".to_string());
        }
        *is_installing = true;
    }

    let app_handle_clone = app_handle.clone();
    let state_clone = state.inner().clone();

    tauri::async_runtime::spawn_blocking(move || {
        use std::io::{Read, Write};
        let emit_progress = |stage: &str, progress: u32, message: &str| {
            let prog = InstallProgress {
                stage: stage.to_string(),
                progress,
                message: message.to_string(),
            };
            {
                let mut current = state_clone.0.current_progress.lock().unwrap();
                *current = prog.clone();
            }
            let _ = app_handle_clone.emit("pandoc_install_progress", prog);
        };

        let result = (move || -> Result<(), String> {
            emit_progress("downloading", 0, "Initializing download...");

            // Determine the appropriate URL based on the OS and CPU Architecture
            let (url, filename, is_zip) = if cfg!(target_os = "macos") {
                if cfg!(target_arch = "aarch64") {
                    ("https://github.com/jgm/pandoc/releases/download/3.2.1/pandoc-3.2.1-arm64-macOS.zip", "pandoc-macOS.zip", true)
                } else {
                    ("https://github.com/jgm/pandoc/releases/download/3.2.1/pandoc-3.2.1-x86_64-macOS.zip", "pandoc-macOS.zip", true)
                }
            } else if cfg!(target_os = "windows") {
                ("https://github.com/jgm/pandoc/releases/download/3.2.1/pandoc-3.2.1-windows-x86_64.zip", "pandoc-windows.zip", true)
            } else {
                ("https://github.com/jgm/pandoc/releases/download/3.2.1/pandoc-3.2.1-linux-amd64.tar.gz", "pandoc-linux.tar.gz", false)
            };

            let backup_url = format!("https://ghfast.top/{}", url);

            let app_dir = app_handle_clone.path().app_data_dir()
                .map_err(|e| format!("Failed to get app data dir: {}", e))?;
            let bin_dir = app_dir.join("bin");
            if !bin_dir.exists() {
                std::fs::create_dir_all(&bin_dir).map_err(|e| format!("Failed to create bin dir: {}", e))?;
            }

            let temp_file_path = app_dir.join(filename);

            // HTTP Download Client
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

            let response = match client.get(url).send() {
                Ok(resp) if resp.status().is_success() => Ok(resp),
                _ => {
                    emit_progress("downloading", 10, "Main source slow, switching to mirror source...");
                    client.get(&backup_url).send().map_err(|e| format!("Failed to download from mirror: {}", e))
                }
            }?;

            let total_size = response.content_length().unwrap_or(0);
            let mut file = std::fs::File::create(&temp_file_path).map_err(|e| format!("Failed to create temp file: {}", e))?;
            let mut downloaded: u64 = 0;
            let mut buffer = [0; 8192];
            
            let mut reader = response;

            loop {
                let bytes_read = reader.read(&mut buffer).map_err(|e| format!("Failed to read stream: {}", e))?;
                if bytes_read == 0 {
                    break;
                }
                file.write_all(&buffer[..bytes_read]).map_err(|e| format!("Failed to write to file: {}", e))?;
                downloaded += bytes_read as u64;
                
                if total_size > 0 {
                    let percent = (downloaded as f64 / total_size as f64 * 80.0) as u32; // Reserve 80-100% for extraction
                    emit_progress("downloading", percent, &format!("Downloading: {}%", percent));
                }
            }
            drop(file);

            emit_progress("extracting", 80, "Extracting binary...");
            let exe_name = if cfg!(windows) { "pandoc.exe" } else { "pandoc" };
            let final_exe_path = bin_dir.join(exe_name);

            if is_zip {
                // Extract zip
                let file = std::fs::File::open(&temp_file_path).map_err(|e| format!("Failed to open ZIP: {}", e))?;
                let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("Invalid zip archive: {}", e))?;
                let mut found = false;
                for i in 0..archive.len() {
                    let mut file = archive.by_index(i).map_err(|e| format!("Failed to read zip index: {}", e))?;
                    let name = file.name().to_string();
                    if name.ends_with("pandoc.exe") || name.ends_with("bin/pandoc") || name.ends_with("/pandoc") {
                        let mut out = std::fs::File::create(&final_exe_path).map_err(|e| format!("Failed to create final exe: {}", e))?;
                        std::io::copy(&mut file, &mut out).map_err(|e| format!("Failed to extract pandoc binary: {}", e))?;
                        found = true;

                        // Apply execution permissions on macOS
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let mut perms = std::fs::metadata(&final_exe_path).map_err(|e| e.to_string())?.permissions();
                            perms.set_mode(0o755);
                            std::fs::set_permissions(&final_exe_path, perms).map_err(|e| e.to_string())?;
                        }
                        break;
                    }
                }
                if !found {
                    return Err("pandoc executable not found in zip archive".to_string());
                }
            } else {
                // Extract tar.gz (Linux)
                let file = std::fs::File::open(&temp_file_path).map_err(|e| format!("Failed to open downloaded tar.gz: {}", e))?;
                let tar_gz = flate2::read::GzDecoder::new(file);
                let mut archive = tar::Archive::new(tar_gz);
                let entries = archive.entries().map_err(|e| format!("Failed to read tar entries: {}", e))?;

                let mut found = false;
                for entry in entries {
                    let mut entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
                    let path = entry.path().map_err(|e| format!("Failed to read entry path: {}", e))?;
                    let path_str = path.to_string_lossy();
                    if path_str.ends_with("bin/pandoc") || path_str.ends_with("/pandoc") {
                        let mut out = std::fs::File::create(&final_exe_path).map_err(|e| format!("Failed to create final bin: {}", e))?;
                        std::io::copy(&mut entry, &mut out).map_err(|e| format!("Failed to extract pandoc: {}", e))?;
                        found = true;
                        
                        // Apply execution permissions on Linux
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let mut perms = std::fs::metadata(&final_exe_path).map_err(|e| e.to_string())?.permissions();
                            perms.set_mode(0o755);
                            std::fs::set_permissions(&final_exe_path, perms).map_err(|e| e.to_string())?;
                        }
                        break;
                    }
                }
                if !found {
                    return Err("pandoc executable not found in tar.gz archive".to_string());
                }
            }

            // Clean up the temp zip/tar file
            let _ = std::fs::remove_file(&temp_file_path);
            Ok(())
		})();

        {
            let mut is_installing = state_clone.0.is_installing.lock().unwrap();
            *is_installing = false;
        }

        match result {
            Ok(_) => {
                emit_progress("success", 100, "Pandoc installed successfully!");
                Ok(())
            }
            Err(e) => {
                let prog = InstallProgress {
                    stage: "failed".to_string(),
                    progress: 0,
                    message: e.clone(),
                };
                {
                    let mut current = state_clone.0.current_progress.lock().unwrap();
                    *current = prog.clone();
                }
                let _ = app_handle_clone.emit("pandoc_install_progress", prog);
                Err(e)
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn run_pandoc_convert(
    app_handle: tauri::AppHandle,
    input_path: String,
    output_path: String,
    from_format: Option<String>,
    to_format: Option<String>,
    extra_args: Option<Vec<String>>
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        // Decide path: check app bin folder first, fallback to system PATH "pandoc"
        let pandoc_bin = if let Ok(internal_path) = get_internal_pandoc_path(&app_handle) {
            if internal_path.exists() {
                internal_path.to_string_lossy().to_string()
            } else {
                "pandoc".to_string()
            }
        } else {
            "pandoc".to_string()
        };

        let mut cmd = std::process::Command::new(pandoc_bin);
        cmd.arg(&input_path);
        cmd.arg("-o").arg(&output_path);
        
        if let Some(from) = from_format {
            if !from.is_empty() && from != "auto" {
                let clean_from = match from.as_str() {
                    "md" | "markdown" => "markdown",
                    "tex" => "latex",
                    other => other,
                };
                cmd.arg("-f").arg(clean_from);
            }
        }
        if let Some(to) = to_format {
            if !to.is_empty() {
                let clean_to = match to.as_str() {
                    "md" | "markdown" => Some("markdown"),
                    "tex" => Some("latex"),
                    "pdf" => None, // Pandoc routes PDF automatically via output file extension; no -t flag should be used.
                    other => Some(other),
                };
                if let Some(t) = clean_to {
                    cmd.arg("-t").arg(t);
                }
            }
        }
        if let Some(args) = extra_args {
            for arg in args {
                if !arg.trim().is_empty() {
                    cmd.arg(arg);
                }
            }
        }

        let output = cmd.output().map_err(|e| format!("Failed to run pandoc: {}", e))?;
        if output.status.success() {
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr);
            Err(err_msg.trim().to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn select_source_file() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Select Document to Convert")
            .add_filter("All Documents", &["md", "markdown", "docx", "pdf", "html", "epub", "tex", "pptx", "txt"])
            .pick_file()
        {
            Ok(path.to_string_lossy().to_string())
        } else {
            Err("No file selected".to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn select_target_file(default_name: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Save Converted File As")
            .set_file_name(&default_name)
            .save_file()
        {
            Ok(path.to_string_lossy().to_string())
        } else {
            Err("Save cancelled".to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// 文件系统监听器（增量更新索引）
// ---------------------------------------------------------------------------

fn start_fs_watcher(db_path: String, disabled: Arc<AtomicBool>, shutdown: Arc<AtomicBool>, watcher_stopped: Arc<AtomicBool>) {
    watcher_stopped.store(false, Ordering::Relaxed);
    std::thread::spawn(move || {
        let (tx, rx) = mpsc::sync_channel::<notify::Event>(1024);

        let mut watcher = match notify::recommended_watcher(move |res| {
            if let Ok(event) = res {
                tx.try_send(event).ok();
            }
        }) {
            Ok(w) => w,
            Err(_) => {
                watcher_stopped.store(true, Ordering::Relaxed);
                return;
            }
        };

        let roots = get_watch_roots_pub();
        for root in &roots {
            if let Err(e) = watcher.watch(root, RecursiveMode::Recursive) {
                eprintln!("[mtool watcher] failed to watch {:?}: {}", root, e);
            } else {
                eprintln!("[mtool watcher] watching {:?}", root);
            }
        }

        let mut db_conn = rusqlite::Connection::open(&db_path).ok();

        loop {
            let mut events = Vec::new();
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(ev) => {
                    events.push(ev);
                    while let Ok(e) = rx.try_recv() {
                        events.push(e);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }

            if shutdown.load(Ordering::Relaxed) {
                drop(watcher);
                break;
            }

            if disabled.load(Ordering::Relaxed) {
                continue;
            }

            for event in events {
                handle_fs_event(&event, &db_path, &mut db_conn);
            }
        }
        watcher_stopped.store(true, Ordering::Relaxed);
    });
}

fn handle_fs_event(event: &notify::Event, db_path: &str, conn: &mut Option<rusqlite::Connection>) {
    if matches!(event.kind, EventKind::Modify(ModifyKind::Name(RenameMode::Both))) {
        if conn.is_none() {
            *conn = rusqlite::Connection::open(db_path).ok();
        }
        if let Some(c) = conn.as_mut() {
            if let Some(from) = event.paths.first() {
                if !should_skip_path(from) {
                    delete_entry_in_db(c, &from.to_string_lossy());
                }
            }
            if let Some(to) = event.paths.get(1) {
                if !should_skip_path(to) {
                    if let Some(entry) = entry_from_path(to) {
                        upsert_entry_in_db(c, &entry);
                    }
                }
            }
        }
        return;
    }

    for path in &event.paths {
        if should_skip_path(path) {
            continue;
        }
        let path_str = path.to_string_lossy().to_string();
        match &event.kind {
            EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                if conn.is_none() {
                    *conn = rusqlite::Connection::open(db_path).ok();
                }
                if let Some(c) = conn.as_mut() {
                    delete_entry_in_db(c, &path_str);
                }
            }
            EventKind::Modify(ModifyKind::Name(_)) |
            EventKind::Create(_) | EventKind::Modify(_) => {
                if let Some(entry) = entry_from_path(path) {
                    if conn.is_none() {
                        *conn = rusqlite::Connection::open(db_path).ok();
                    }
                    if let Some(c) = conn.as_mut() {
                        upsert_entry_in_db(c, &entry);
                    }
                }
            }
            EventKind::Remove(_) => {
                if conn.is_none() {
                    *conn = rusqlite::Connection::open(db_path).ok();
                }
                if let Some(c) = conn.as_mut() {
                    delete_entry_in_db(c, &path_str);
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// 总公司专有加解密 DLL 调用
// ---------------------------------------------------------------------------
#[tauri::command]
async fn hq_crypto(
    action: String, 
    payload: String,
    jar_path: String,
    biz_type: String,
    jdk_path: Option<String>
) -> Result<String, String> {
    if action != "enc" && action != "dec" {
        return Err("Invalid action. Must be 'enc' or 'dec'.".to_string());
    }

    if biz_type.is_empty() 
        || biz_type.len() > 64 
        || !biz_type.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') 
    {
        return Err("Invalid bizType format. Only alphanumeric characters, underscores, and hyphens are allowed (max 64 chars).".to_string());
    }

    if payload.len() > 1 * 1024 * 1024 {
        return Err("Payload too large (max 1 MB)".to_string());
    }

    let path = std::path::Path::new(&jar_path);
    if !path.exists() || path.extension().map_or(true, |ext| ext != "jar") {
        return Err("Invalid JAR path: file must exist and have a .jar extension.".to_string());
    }

    tauri::async_runtime::spawn_blocking(move || {
        // Resolve java executable: prefer user-specified JDK directory, fall back to system PATH
        let java_bin = if let Some(ref jdk) = jdk_path {
            let jdk_dir = std::path::Path::new(jdk);
            let bin = jdk_dir.join("bin").join(if cfg!(windows) { "java.exe" } else { "java" });
            if bin.exists() {
                bin.to_string_lossy().to_string()
            } else {
                return Err(format!(
                    "Java executable not found at '{}'. Please verify the JDK directory.",
                    bin.display()
                ));
            }
        } else {
            "java".to_string()
        };
        let mut command = std::process::Command::new(&java_bin);
        command
            .arg("-Dfile.encoding=UTF-8")
            .arg("-jar")
            .arg(&jar_path)
            .arg(&biz_type)
            .arg(&action)
            .arg(&payload)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let child = command
            .spawn()
            .map_err(|e| format!("Failed to execute 'java': {}", e))?;

        let pid = child.id();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(child.wait_with_output());
        });

        match rx.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(Ok(output)) => {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    Ok(stdout)
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    Err(format!("HQ library execution error: {}\n{}", stderr, stdout))
                }
            }
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => {
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
                Err("Execution timeout (30s), background process has been force terminated.".to_string())
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn select_hq_jar() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Jar Archive", &["jar"])
            .pick_file()
        {
            Ok(path.to_string_lossy().to_string())
        } else {
            Err("No file selected".to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn select_jdk_dir() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Select JDK / JRE Directory")
            .pick_folder()
        {
            // Verify the directory contains bin/java or bin/java.exe
            let java_bin = path.join("bin").join(if cfg!(windows) { "java.exe" } else { "java" });
            if java_bin.exists() {
                Ok(path.to_string_lossy().to_string())
            } else {
                Err(format!(
                    "Selected directory does not contain bin/java. Please select a valid JDK/JRE root directory."
                ))
            }
        } else {
            Err("No directory selected".to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// 应用入口
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
   tauri::Builder::default()
       .plugin(tauri_plugin_window_state::Builder::default().build())
       .plugin(tauri_plugin_opener::init())
       .plugin(tauri_plugin_updater::Builder::new().build())
       .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let db_path = get_db_path();
            if let Err(e) = init_db(&db_path) {
                eprintln!("[mtool] failed to init db in setup: {}", e);
            }

            let engine = IndexEngine::new_with_db(db_path.clone());
            engine.load_from_db();

            let index_disabled = is_index_disabled(&db_path);
            let has_data = count_entries(&db_path) > 0;
            if index_disabled {
                engine.disabled.store(true, Ordering::Relaxed);
                engine.shutdown.store(true, Ordering::Relaxed);
            }
            maybe_start_watcher(&engine, has_data, index_disabled);

            app.manage(engine);

            let transfer_state = file_transfer::TransferState::new();
            app.manage(transfer_state.clone());
            app.manage(PandocInstallState::new());
            let app_handle = app.handle().clone();
            let transfer_state_clone = transfer_state.clone();
            tauri::async_runtime::spawn(async move {
                file_transfer::start_file_transfer_server(app_handle, transfer_state_clone).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            format_json,
            minify_json,
            generate_qr,
            read_text_from_clipboard,
            copy_qr_to_clipboard,
            download_qr,
            open_md_file,
            open_md_file_by_path,
            save_md_file,
            save_md_file_as,
            build_index,
            get_index_status,
            search_files,
            disable_file_search,
            reveal_in_explorer,
            open_file,
            open_text_file,
            read_text_file_by_path,
            jar_viewer::open_jar_or_class,
            jar_viewer::list_jar_entries,
            jar_viewer::read_jar_entry,
            jar_viewer::read_local_class,
            hq_crypto,
            select_hq_jar,
            select_jdk_dir,
            file_transfer::get_local_transfer_info,
            file_transfer::get_transfer_config,
            file_transfer::update_save_dir,
            file_transfer::select_save_dir,
            file_transfer::remove_trusted_peer,
            file_transfer::update_peer_alias,
            file_transfer::send_friend_request,
            file_transfer::respond_friend_request,
            file_transfer::send_file,
            file_transfer::select_file_to_send,
            file_transfer::get_file_info,
            file_transfer::cancel_transfer,
            file_transfer::delete_local_file,
            file_transfer::get_history_records,
            file_transfer::delete_history_record,
            file_transfer::clear_history_records,
            check_pandoc,
            install_pandoc,
            get_pandoc_install_status,
            run_pandoc_convert,
            select_source_file,
            select_target_file,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, _event| {});
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn test_maybe_start_watcher_conditions() {
        let engine = file_search::IndexEngine::new_with_db(":memory:".to_string());
        
        // Ensure watcher_stopped is initially true
        engine.watcher_stopped.store(true, Ordering::Relaxed);
        
        // Case 1: index_disabled = true, has_data = true
        // Watcher should NOT start
        maybe_start_watcher(&engine, true, true);
        assert!(engine.watcher_stopped.load(Ordering::Relaxed), "Watcher should not start if disabled");
        
        // Case 2: index_disabled = false, has_data = false
        // Watcher should NOT start
        maybe_start_watcher(&engine, false, false);
        assert!(engine.watcher_stopped.load(Ordering::Relaxed), "Watcher should not start if no data");
        
        // Case 3: index_disabled = false, has_data = true
        // Watcher SHOULD start. We set shutdown=true beforehand so the thread exits quickly.
        engine.shutdown.store(true, Ordering::Relaxed);
        maybe_start_watcher(&engine, true, false);
        
        // start_fs_watcher immediately sets watcher_stopped to false before spawning thread
        assert!(!engine.watcher_stopped.load(Ordering::Relaxed), "Watcher should start");
        
        // Wait for the watcher thread to gracefully exit to avoid panics on test end
        let mut attempts = 0;
        while !engine.watcher_stopped.load(Ordering::Relaxed) && attempts < 500 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            attempts += 1;
        }
    }
}
