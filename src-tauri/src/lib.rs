mod file_search;

use base64::prelude::*;
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use arboard::{Clipboard, ImageData};
use image::Rgb;
use notify::{EventKind, RecursiveMode, Watcher};
use qrcode::{EcLevel, QrCode};

use tauri::{Emitter, Manager};
use rayon::prelude::*;

use file_search::{
    build_index_full_system, content_matches, delete_entry_in_db, entry_from_path,
    get_db_path, init_db, parse_query, save_entries_to_db, search_in_db, set_last_built_at,
    should_skip_path, upsert_entry_in_db, FileEntry, IndexEngine, IndexStatus,
    get_watch_roots_pub, SizeOp,
};

// ---------------------------------------------------------------------------
// 现有命令
// ---------------------------------------------------------------------------

#[tauri::command]
fn format_json(input: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(input).map_err(|e| format!("Invalid JSON: {}", e))?;
    serde_json::to_string_pretty(&value).map_err(|e| format!("Format error: {}", e))
}

#[tauri::command]
fn minify_json(input: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(input).map_err(|e| format!("Invalid JSON: {}", e))?;
    serde_json::to_string(&value).map_err(|e| format!("Minify error: {}", e))
}

#[tauri::command]
fn generate_qr(payload: &str, redundancy: &str, resolution: u32, color: &str) -> Result<String, String> {
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
    let image = code
        .render::<Rgb<u8>>()
        .min_dimensions(resolution, resolution)
        .dark_color(Rgb([r, g, b]))
        .light_color(Rgb([255, 255, 255]))
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

// ---------------------------------------------------------------------------
// 文件搜索命令
// ---------------------------------------------------------------------------

#[tauri::command]
async fn build_index(
    app: tauri::AppHandle,
    state: tauri::State<'_, IndexEngine>,
) -> Result<usize, String> {
    let entries_ref = state.entries.clone();
    let status_ref = state.status.clone();
    let db_path = state.db_path.clone();

    {
        let mut s = status_ref.write().unwrap();
        s.is_indexing = true;
        s.total = 0;
    }

    let app_clone = app.clone();
    let (total, _elapsed) = tauri::async_runtime::spawn_blocking(move || {
        let t0 = Instant::now();
        let new_entries = build_index_full_system(|count| {
            app_clone.emit("index_progress", count).ok();
        });
        let total = new_entries.len();

        save_entries_to_db(&db_path, &new_entries);

        let now_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        set_last_built_at(&db_path, now_ts);

        *entries_ref.write().unwrap() = new_entries;
        (total, t0.elapsed().as_millis())
    })
    .await
    .map_err(|e| e.to_string())?;

    {
        let mut s = status_ref.write().unwrap();
        s.is_indexing = false;
        s.total = total;
        s.last_built_at = file_search::get_last_built_at(&state.db_path);
    }

    app.emit("index_progress", total).ok();
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

    // ── 路径 1：纯名称 / 名称+大小，无 glob 无 content ──────────────────────
    // SQLite LIKE '%term%' 前置通配符无法走索引，460 万行全表扫 → 15s
    // 改走内存 Vec + rayon 并行过滤，预计 < 200ms
    let is_mem_path = parsed.content_filter.is_none()
        && parsed.glob_pattern.is_none()
        && (!parsed.name_terms.is_empty() || parsed.size_filter.is_some());

    if is_mem_path {
        let entries_ref = state.entries.clone();
        let terms = parsed.name_terms.clone();
        let size_filter = parsed.size_filter.clone();
        return tauri::async_runtime::spawn_blocking(move || {
            let entries = entries_ref.read().unwrap();
            let mut results: Vec<FileEntry> = entries
                .par_iter()
                .filter(|e| {
                    // 名称关键词：每个词都必须在 name_lower 中出现
                    if !terms.iter().all(|t| e.name_lower.contains(t.as_str())) {
                        return false;
                    }
                    // 大小过滤
                    if let Some(ref sf) = size_filter {
                        if e.is_dir {
                            return false;
                        }
                        let ok = match sf.op {
                            SizeOp::Gt  => e.size >  sf.bytes,
                            SizeOp::Gte => e.size >= sf.bytes,
                            SizeOp::Lt  => e.size <  sf.bytes,
                            SizeOp::Lte => e.size <= sf.bytes,
                            SizeOp::Eq  => e.size == sf.bytes,
                        };
                        if !ok {
                            return false;
                        }
                    }
                    true
                })
                .take_any(limit)
                .cloned()
                .collect();
            results.truncate(limit);
            results
        })
        .await
        .map_err(|e| e.to_string());
    }

    // ── 路径 2：glob 模式（*.ext → idx_ext 索引，极快）────────────────────────
    if parsed.content_filter.is_none() {
        return tauri::async_runtime::spawn_blocking(move || {
            search_in_db(&db_path, &parsed, limit)
        })
        .await
        .map_err(|e| e.to_string());
    }

    // ── 路径 3：content 过滤 → SQLite 取候选，rayon 并行扫文件内容 ─────────────
    let content_needle = parsed.content_filter.as_ref().unwrap().as_bytes().to_vec();
    let results = tauri::async_runtime::spawn_blocking(move || {
        let candidates = search_in_db(&db_path, &parsed, 100_000);
        let mut matched: Vec<FileEntry> = candidates
            .into_par_iter()
            .filter(|e| !e.is_dir && content_matches(&e.path, &content_needle))
            .collect();
        matched.truncate(limit);
        matched
    })
    .await
    .map_err(|e| e.to_string())?;

    Ok(results)
}

#[tauri::command]
fn reveal_in_explorer(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg("/select,")
            .arg(&path)
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
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &path])
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
// ---------------------------------------------------------------------------
// 文件系统监听器（增量更新索引）
// ---------------------------------------------------------------------------

fn start_fs_watcher(
    entries_ref: std::sync::Arc<std::sync::RwLock<Vec<FileEntry>>>,
    db_path: String,
) {
    std::thread::spawn(move || {
        let (tx, rx) = mpsc::channel();

        let mut watcher = match notify::recommended_watcher(move |res| {
            if let Ok(event) = res {
                tx.send(event).ok();
            }
        }) {
            Ok(w) => w,
            Err(_) => return,
        };

        let roots = get_watch_roots_pub();
        for root in &roots {
            if let Err(e) = watcher.watch(root, RecursiveMode::Recursive) {
                eprintln!("[mtool watcher] failed to watch {:?}: {}", root, e);
            } else {
                eprintln!("[mtool watcher] watching {:?}", root);
            }
        }

        // 批量处理，每 500ms 聚合一次避免频繁写库
        loop {
            let mut events = Vec::new();
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(ev) => {
                    events.push(ev);
                    // 尽量排空 channel 内剩余事件
                    while let Ok(e) = rx.try_recv() {
                        events.push(e);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
            }

            for event in events {
                handle_fs_event(&event, &entries_ref, &db_path);
            }
        }
    });
}

fn handle_fs_event(
    event: &notify::Event,
    entries_ref: &std::sync::Arc<std::sync::RwLock<Vec<FileEntry>>>,
    db_path: &str,
) {
    for path in &event.paths {
        if should_skip_path(path) {
            continue;
        }
        let path_str = path.to_string_lossy().to_string();

        match &event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                upsert_in_memory_and_db(path, &path_str, entries_ref, db_path);
            }
            EventKind::Remove(_) => {
                remove_from_memory_and_db(&path_str, entries_ref, db_path);
            }
            _ => {}
        }
    }
}

fn upsert_in_memory_and_db(
    path: &Path,
    path_str: &str,
    entries_ref: &std::sync::Arc<std::sync::RwLock<Vec<FileEntry>>>,
    db_path: &str,
) {
    let Some(new_entry) = entry_from_path(path) else { return };
    upsert_entry_in_db(db_path, &new_entry);
    let mut entries = entries_ref.write().unwrap();
    if let Some(pos) = entries.iter().position(|e| e.path == path_str) {
        entries[pos] = new_entry;
    } else {
        entries.push(new_entry);
    }
}

fn remove_from_memory_and_db(
    path_str: &str,
    entries_ref: &std::sync::Arc<std::sync::RwLock<Vec<FileEntry>>>,
    db_path: &str,
) {
    delete_entry_in_db(db_path, path_str);
    let mut entries = entries_ref.write().unwrap();
    entries.retain(|e| e.path != path_str);
}

// ---------------------------------------------------------------------------
// 应用入口
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let db_path = get_db_path();
            init_db(&db_path);

            let engine = IndexEngine::new_with_db(db_path.clone());
            engine.load_from_db();

            let has_data = {
                let entries = engine.entries.read().unwrap();
                !entries.is_empty()
            };

            // 启动文件系统监听器（增量更新）
            start_fs_watcher(engine.entries.clone(), db_path.clone());

            app.manage(engine);

            if !has_data {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let state: tauri::State<IndexEngine> = app_handle.state();
                    let entries_ref = state.entries.clone();
                    let status_ref = state.status.clone();
                    let db = state.db_path.clone();

                    {
                        let mut s = status_ref.write().unwrap();
                        s.is_indexing = true;
                    }

                    let app_clone = app_handle.clone();
                    let result = tauri::async_runtime::spawn_blocking(move || {
                        let new_entries = build_index_full_system(|count| {
                            app_clone.emit("index_progress", count).ok();
                        });
                        let total = new_entries.len();
                        save_entries_to_db(&db, &new_entries);
                        let now_ts = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        set_last_built_at(&db, now_ts);
                        *entries_ref.write().unwrap() = new_entries;
                        (total, now_ts)
                    })
                    .await;

                    if let Ok((total, ts)) = result {
                        let mut s = status_ref.write().unwrap();
                        s.is_indexing = false;
                        s.total = total;
                        s.last_built_at = Some(ts);
                        app_handle.emit("index_progress", total).ok();
                        app_handle.emit("index_complete", total).ok();
                    }
                });
            }

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
            save_md_file,
            save_md_file_as,
            build_index,
            get_index_status,
            search_files,
            reveal_in_explorer,
            open_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
