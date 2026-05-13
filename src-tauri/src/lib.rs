mod file_search;
mod jar_viewer;
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
            .map_err(|e| format!("Failed to read file: {}", e))?;
        let path_str = path.to_string_lossy().to_string();
        Ok((path_str, content))
    } else {
        Err("No file selected".to_string())
    }
}

#[tauri::command]
fn save_md_file(path: &str, content: &str) -> Result<String, String> {
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
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read file: {}", e))?;
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
            "properties", "vue", "svelte",
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
            .map_err(|e| format!("Failed to read file: {}", e))?;
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
        .map_err(|e| format!("Failed to read file: {}", e))?;
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
        let mut s = status_ref.write().unwrap();
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
            let mut s = status_ref.write().unwrap();
            s.is_indexing = false;
            return Err(e);
        }
        Err(e) => {
            let mut s = status_ref.write().unwrap();
            s.is_indexing = false;
            return Err(e.to_string());
        }
    };

    {
        let mut s = status_ref.write().unwrap();
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
    std::thread::spawn(move || {
        // 1. 先完成 FTS5 重建
        if rebuild_fts5_background(&db_fts) {
            fts5_ready_clone.store(true, Ordering::Relaxed);
        }
        // 2. 再启动 watcher（避免与 FTS5 重建产生写写竞争）
        start_fs_watcher(db_path_w, disabled, shutdown, watcher_stopped);
    });

    Ok(total)
}

#[tauri::command]
fn get_index_status(state: tauri::State<'_, IndexEngine>) -> IndexStatus {
    state.status.read().unwrap().clone()
}

#[tauri::command]
async fn search_files(
    query: String,
    limit: usize,
    state: tauri::State<'_, IndexEngine>,
) -> Result<Vec<FileEntry>, String> {
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
    let content_needle = parsed.content_filter.as_ref().unwrap().as_bytes().to_vec();
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
        let mut s = state.status.write().unwrap();
        s.is_indexing = false;
        s.total = 0;
        s.last_built_at = None;
    }
    state.disabled.store(true, Ordering::Relaxed);
    state.shutdown.store(true, Ordering::Relaxed);
    state.fts5_ready.store(false, Ordering::Relaxed);

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
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", path))
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
