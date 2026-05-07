use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, RwLock};
use std::time::{Duration, UNIX_EPOCH};
use rayon::prelude::*;

const CONTENT_BUFFER_BYTES: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// 数据类型
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub created: u64,
    pub modified: u64,
    pub is_dir: bool,
    pub ext: String,
    pub name_lower: String,
}

#[derive(Clone, Serialize, Debug, Default)]
pub struct IndexStatus {
    pub total: usize,
    pub is_indexing: bool,
    pub last_built_at: Option<u64>,
}

pub struct IndexEngine {
    pub status: Arc<RwLock<IndexStatus>>,
    pub db_path: String,
    pub disabled: Arc<AtomicBool>,
    pub shutdown: Arc<AtomicBool>,
    pub fts5_ready: Arc<AtomicBool>,
    pub watcher_stopped: Arc<AtomicBool>,
}

impl IndexEngine {
    pub fn new_with_db(db_path: String) -> Self {
        let fts5_ready = match Connection::open(&db_path) {
            Ok(conn) => {
                let index_count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM file_index", [], |r| r.get(0),
                ).unwrap_or(0);
                let fts5_count: i64 = conn.query_row(
                    "SELECT CAST(value AS INTEGER) FROM index_meta WHERE key='fts5_count'",
                    [], |r| r.get(0),
                ).unwrap_or(0);
                index_count > 0
                    && fts5_count > 0
                    && (fts5_count - index_count).abs() <= index_count / 100
            }
            Err(_) => false,
        };

        Self {
            status: Arc::new(RwLock::new(IndexStatus::default())),
            db_path,
            disabled: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(AtomicBool::new(false)),
            fts5_ready: Arc::new(AtomicBool::new(fts5_ready)),
            watcher_stopped: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn load_from_db(&self) {
        let total = count_entries(&self.db_path);
        let last_built_at = get_last_built_at(&self.db_path);
        {
            let mut s = self.status.write().unwrap();
            s.total = total;
            s.last_built_at = last_built_at;
        }
    }
}


// ---------------------------------------------------------------------------
// 跨平台数据库路径
// ---------------------------------------------------------------------------

pub fn get_db_path() -> String {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."));
    let app_dir = data_dir.join("mtool");
    std::fs::create_dir_all(&app_dir).ok();
    app_dir.join("mtool_index.db").to_string_lossy().to_string()
}

// ---------------------------------------------------------------------------
// SQLite 操作
// ---------------------------------------------------------------------------

pub fn init_db(db_path: &str) {
    let conn = Connection::open(db_path).expect("open db");
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA cache_size=-8192;",
    )
    .ok();
    conn.execute_batch(SCHEMA_SQL).expect("init db schema");
}

/// 仅 file_index + file_fts 表的 Schema（不含 index_meta，避免 DROP 时误删退出时间戳）
const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS file_index (
    id       INTEGER PRIMARY KEY,
    name     TEXT NOT NULL,
    path     TEXT NOT NULL UNIQUE,
    size     INTEGER NOT NULL,
    created  INTEGER NOT NULL,
    modified INTEGER NOT NULL,
    is_dir   INTEGER NOT NULL,
    ext      TEXT NOT NULL,
    name_lower TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_name_lower ON file_index(name_lower);
CREATE INDEX IF NOT EXISTS idx_ext ON file_index(ext);
CREATE TABLE IF NOT EXISTS index_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS file_fts USING fts5(
    path UNINDEXED,
    name_lower,
    tokenize='trigram'
);
";

/// DROP + 重建 file_index / file_fts 表。
/// O(1)，比 DELETE FROM file_index（在 WAL 模式下需生成大量 WAL 页）快一到两个数量级。
/// 保留 index_meta 表，避免丢失 last_built_at。
pub fn count_entries(db_path: &str) -> usize {
    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    conn.query_row("SELECT COUNT(*) FROM file_index", [], |row| {
        row.get::<_, i64>(0)
    })
    .unwrap_or(0) as usize
}

pub fn get_last_built_at(db_path: &str) -> Option<u64> {
    let conn = Connection::open(db_path).ok()?;
    conn.query_row(
        "SELECT value FROM index_meta WHERE key='last_built_at'",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|v| v.parse::<u64>().ok())
}

pub fn set_last_built_at(db_path: &str, ts: u64) {
    let conn = Connection::open(db_path).expect("open db");
    conn.execute(
        "INSERT OR REPLACE INTO index_meta(key,value) VALUES('last_built_at',?1)",
        params![ts.to_string()],
    )
    .ok();
}

// ---------------------------------------------------------------------------
// 跨平台跳过目录逻辑
// ---------------------------------------------------------------------------

pub fn should_skip_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();

    #[cfg(target_os = "macos")]
    {
        let skip_prefixes = [
            // 伪文件系统 / 内核接口
            "/dev",
            "/proc",
            "/sys",
            // APFS 数据卷 —— 与 / 下内容完全重叠，不跳过会重复计数
            "/System/Volumes/Data",
            "/System/Volumes/VM",
            "/System/Volumes/Preboot",
            "/System/Volumes/Update",
            "/System/Volumes/xarts",
            "/System/Volumes/iSCPreboot",
            "/System/Volumes/Hardware",
            "/System/Volumes/FieldService",
            // Time Machine 本地快照
            "/Volumes/com.apple.TimeMachine",
            // 系统临时 / 虚拟内存
            "/private/var/vm",
            "/private/var/folders",
            "/private/var/db",
            "/private/tmp",
            "/private/var/tmp",
            // Spotlight / fsevents
            "/.Spotlight-V100",
            "/.fseventsd",
            // 崩溃转储
            "/cores",
            // Recovery 分区
            "/Volumes/Recovery",
            // Xcode 模拟器（巨大，通常数百万文件）
            "/Library/Developer/CoreSimulator",
        ];
        for prefix in &skip_prefixes {
            if path_str.as_bytes().starts_with(prefix.as_bytes())
                && (path_str.len() == prefix.len()
                    || path_str.as_bytes().get(prefix.len()) == Some(&b'/'))
            {
                return true;
            }
        }
        // 跳过根目录下以 "." 开头的隐藏目录
        if let Some(stripped) = path_str.strip_prefix('/') {
            if let Some(first_component) = stripped.split('/').next() {
                if first_component.starts_with('.') {
                    return true;
                }
            }
        }
        // 跳过任意深度的特定目录（路径包含匹配）
        for skip in [
            "/node_modules/",      // JS 依赖，数量巨大但无需搜索
            "/.git/",              // Git 内部对象
            "/.Trash/",            // 废纸篓
            "/.rustup/toolchains/", // Rust 工具链源码
            "/Library/Containers/",        // ← 新增：跳过其他 App 沙盒容器
            "/Library/Group Containers/",  // ← 新增：跳过 Group 容器
        ] {
            if path_str.contains(skip) {
                return true;
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let skip_prefixes = [
            "/dev", "/proc", "/sys", "/run", "/tmp", "/var/run", "/var/lock",
        ];
        for prefix in &skip_prefixes {
            if path_str.as_bytes().starts_with(prefix.as_bytes())
                && (path_str.len() == prefix.len()
                    || path_str.as_bytes().get(prefix.len()) == Some(&b'/'))
            {
                return true;
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let lower = path_str.to_ascii_lowercase();
        let without_drive = if lower.len() >= 2 && lower.as_bytes()[1] == b':' {
            &lower[2..]
        } else {
            lower.as_ref()
        };
        // 前缀匹配：跳过回收站、系统卷
        let skip_prefixes = [
            "\\$recycle.bin",
            "\\system volume information",
        ];
        for prefix in &skip_prefixes {
            if without_drive == *prefix
                || without_drive.starts_with(&format!("{}\\", prefix))
            {
                return true;
            }
        }
        // Windows 子目录：只跳过 winsxs、installer 等黑洞，不跳过整个 \Windows
        let win_skip = [
            "\\windows\\winsxs",
            "\\windows\\installer",
            "\\windows\\softwaredistribution",
            "\\windows\\servicing",
            "\\windows\\temp",
        ];
        for prefix in &win_skip {
            if without_drive.as_bytes().starts_with(prefix.as_bytes())
                && (without_drive.len() == prefix.len()
                    || without_drive.as_bytes().get(prefix.len()) == Some(&b'\\'))
            {
                return true;
            }
        }
        // 用户临时目录
        if without_drive.contains("\\appdata\\local\\temp") {
            return true;
        }
        // OneDrive 缓存 / 占位符目录（大量云端文件，扫描会触发同步卡死）
        if without_drive.contains("\\appdata\\local\\microsoft\\onedrive") {
            return true;
        }
        // node_modules / .git（与 macOS 对齐）
        if without_drive.contains("\\node_modules\\")
            || without_drive.contains("\\.git\\")
        {
            return true;
        }
        // 跳过根目录下的系统文件
        let skip_files = ["pagefile.sys", "swapfile.sys", "hiberfil.sys"];
        if let Some(fname) = path.file_name() {
            let fname_lower = fname.to_string_lossy().to_ascii_lowercase();
            for f in &skip_files {
                if fname_lower == *f {
                    return true;
                }
            }
        }
    }

    false
}

/// 文件系统监听器使用的监听根目录（与全量扫描根目录不同）
/// macOS/Linux：只监听用户目录，避免监听 / 导致权限或 APFS firmlink 问题
pub fn get_watch_roots_pub() -> Vec<PathBuf> {
    get_watch_roots()
}

fn get_watch_roots() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        // /Users 是 APFS firmlink，无需 Full Disk Access 即可监听
        // 比监听 / 更可靠，且不会因 APFS 卷结构导致事件丢失
        vec![
            PathBuf::from("/Users"),
            PathBuf::from("/Applications"),
        ]
    }

    #[cfg(target_os = "linux")]
    {
        let mut roots = Vec::new();
        if let Some(home) = dirs::home_dir() {
            roots.push(home);
        }
        // 兜底：若无法获取 home，退回 /home
        if roots.is_empty() {
            roots.push(PathBuf::from("/home"));
        }
        roots
    }

    #[cfg(target_os = "windows")]
    {
        get_system_roots()
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        vec![PathBuf::from("/")]
    }
}

#[allow(dead_code)]
fn get_system_roots() -> Vec<PathBuf> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        vec![PathBuf::from("/")]
    }

    #[cfg(target_os = "windows")]
    {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};
        // 并行检测所有盘符，统一 2 秒超时，避免断线网络盘卡死
        let candidates: Vec<PathBuf> = (b'A'..=b'Z')
            .map(|c| PathBuf::from(format!("{}:\\", c as char)))
            .collect();
        let (tx, rx) = mpsc::channel();
        for p in &candidates {
            let p = p.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                let ok = std::fs::read_dir(&p).is_ok();
                tx.send((p, ok)).ok();
            });
        }
        drop(tx);
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut roots = Vec::new();
        while let Some(dur) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(dur) {
                Ok((p, true)) => roots.push(p),
                Ok((_, false)) => {}
                Err(_) => break,
            }
        }
        roots
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        vec![PathBuf::from("/")]
    }
}

// ---------------------------------------------------------------------------
// 索引构建（并行扫描 + 流式写入）
// ---------------------------------------------------------------------------

fn scan_dir_parallel(
    dir: &Path,
    tx: &mpsc::SyncSender<Vec<FileEntry>>,
    counter: &Arc<AtomicUsize>,
) {
    scan_dir_parallel_owned(dir.to_path_buf(), tx.clone(), counter.clone());
}

fn scan_dir_parallel_owned(
    dir: PathBuf,
    tx: mpsc::SyncSender<Vec<FileEntry>>,
    counter: Arc<AtomicUsize>,
) {
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut file_entries = Vec::new();
    let mut dir_paths: Vec<PathBuf> = Vec::new();
    for e in entries {
        let Ok(e) = e else { continue };
        let path = e.path();
        if should_skip_path(&path) { continue; }
        let ft = match e.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_dir() && !ft.is_symlink() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.is_empty() {
                let path_str = path.to_string_lossy().to_string();
                file_entries.push(FileEntry {
                    name: name.clone(),
                    name_lower: name.to_ascii_lowercase(),
                    path: path_str,
                    size: 0,
                    created: 0,
                    modified: 0,
                    is_dir: true,
                    ext: String::new(),
                });
                if file_entries.len() >= 100 {
                    counter.fetch_add(file_entries.len(), Ordering::Relaxed);
                    tx.send(std::mem::take(&mut file_entries)).ok();
                }
            }
            dir_paths.push(path);
            continue;
        }
        if ft.is_file() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.is_empty() { continue; }
            let name_lower = name.to_ascii_lowercase();
            let ext = path.extension()
                .map(|x| x.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let path_str = path.to_string_lossy().to_string();
            // Windows：symlink_metadata 不触发 OneDrive placeholder 文件云下载
            // macOS/Linux：metadata 正常用
            let (size, created, modified) = {
                #[cfg(target_os = "windows")]
                let meta_result = std::fs::symlink_metadata(&path);
                #[cfg(not(target_os = "windows"))]
                let meta_result = e.metadata();
                match meta_result {
                    Ok(m) => (
                        if m.is_file() { m.len() } else { 0 },
                        m.created().ok()
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .map(|d| d.as_secs()).unwrap_or(0),
                        m.modified().ok()
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .map(|d| d.as_secs()).unwrap_or(0),
                    ),
                    Err(_) => (0u64, 0u64, 0u64),
                }
            };
            file_entries.push(FileEntry {
                name, name_lower, path: path_str,
                size, created, modified,
                is_dir: false, ext,
            });
            if file_entries.len() >= 100 {
                counter.fetch_add(file_entries.len(), Ordering::Relaxed);
                tx.send(std::mem::take(&mut file_entries)).ok();
            }
        }
    }
    if !file_entries.is_empty() {
        counter.fetch_add(file_entries.len(), Ordering::Relaxed);
        tx.send(file_entries).ok();
    }
    dir_paths.into_par_iter().for_each(move |dir_path| {
        scan_dir_parallel_owned(dir_path, tx.clone(), counter.clone());
    });
}

/// 将一批 FileEntry 写入 SQLite file_index。
fn flush_batch_conn(conn: &mut Connection, batch: &[FileEntry]) {
    if batch.is_empty() { return; }
    let tx = match conn.transaction() { Ok(t) => t, Err(_) => return };
    {
        let mut stmt = match tx.prepare(
            "INSERT OR REPLACE INTO file_index \
             (name,path,size,created,modified,is_dir,ext,name_lower) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        ) { Ok(s) => s, Err(_) => return };
        for e in batch {
            stmt.execute(params![
                e.name, e.path, e.size as i64, e.created as i64,
                e.modified as i64, e.is_dir as i32, e.ext, e.name_lower,
            ]).ok();
        }
    }
    tx.commit().ok();
}

/// 并行全量建索引：rayon 多核并发遍历 + 独立 DB writer + 独立 emitter。
/// 某个目录慢不阻塞其他目录的扫描，进度数字持续增长。
/// FTS5 在扫描完成后由调用方在后台重建。
pub fn build_index_streaming<F>(db_path: &str, on_progress: F) -> usize
where
    F: Fn(usize, Option<&str>) + Send + Sync + 'static,
{
    const FLUSH_SIZE: usize = 5_000;
    let counter = Arc::new(AtomicUsize::new(0));
    let on_progress = Arc::new(on_progress);
    let done_flag = Arc::new(AtomicBool::new(false));

    // ── emitter 线程：每 50ms 读 atomic counter → on_progress ──────────
    let emitter_counter = counter.clone();
    let emitter_cb = on_progress.clone();
    let emitter_done = done_flag.clone();
    std::thread::spawn(move || {
        while !emitter_done.load(Ordering::Relaxed) {
            let count = emitter_counter.load(Ordering::Relaxed);
            emitter_cb(count, None);
            std::thread::sleep(Duration::from_millis(500));
        }
        emitter_cb(emitter_counter.load(Ordering::Relaxed), None);
    });

    // ── DB writer 线程：从有界 channel 接收条目，攒够 FLUSH_SIZE 再写 ────────
    let (tx, rx) = mpsc::sync_channel::<Vec<FileEntry>>(64);
    let db_path_writer = db_path.to_string();
    let db_thread = std::thread::spawn(move || {
        let mut conn = match Connection::open(&db_path_writer) {
            Ok(c) => c,
            Err(_) => return 0usize,
        };
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=OFF;
             PRAGMA cache_size=-32768;",
        ).ok();
        let mut total = 0usize;
        let mut buffer: Vec<FileEntry> = Vec::with_capacity(FLUSH_SIZE);
        while let Ok(chunk) = rx.recv() {
            buffer.extend(chunk);
            if buffer.len() >= FLUSH_SIZE {
                flush_batch_conn(&mut conn, &buffer);
                total += buffer.len();
                buffer.clear();
            }
        }
        if !buffer.is_empty() {
            flush_batch_conn(&mut conn, &buffer);
            total += buffer.len();
        }
        conn.execute_batch(
            "PRAGMA synchronous=NORMAL;
             PRAGMA cache_size=-8192;",
        ).ok();
        total
    });

    // ── 并行扫描（当前线程协调，rayon 线程池执行）────────────────────────
    let roots = get_watch_roots();
    if roots.is_empty() {
        // 所有盘符都无法访问（如断线网络盘），fallback 到用户目录
        let fallback = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("C:\\Users"));
        eprintln!("[mtool] no accessible drives found, fallback to {:?}", fallback);
        scan_dir_parallel(&fallback, &tx, &counter);
    } else {
        for root in &roots {
            scan_dir_parallel(root, &tx, &counter);
        }
    }
    drop(tx);
    let total = db_thread.join().unwrap_or(0);
    done_flag.store(true, Ordering::Relaxed);
    on_progress(total, None);
    total
}

/// 从 file_index 表批量重建 FTS5 索引（独立连接，synchronous=OFF 加速）。
/// 分批提交（每 BATCH_SIZE 行一个事务），避免长时间持有排他写锁导致 UI 卡死。
/// 成功返回 true，失败返回 false。
pub fn rebuild_fts5_background(db_path: &str) -> bool {
    const BATCH_SIZE: i64 = 50_000;

    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[mtool fts5] failed to open db: {}", e);
            return false;
        }
    };
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=OFF;
         PRAGMA cache_size=-32768;",
    ).ok();

    // 1. 清空旧的 FTS5 数据
    if let Err(e) = conn.execute_batch(
        "BEGIN;
         INSERT INTO file_fts(file_fts) VALUES('delete-all');
         COMMIT;",
    ) {
        eprintln!("[mtool fts5] failed to clear fts5: {}", e);
        conn.execute_batch("PRAGMA synchronous=NORMAL;").ok();
        return false;
    }

    // 2. 分批插入，每批之间释放写锁，让前端 IPC 有机会读取数据库
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM file_index", [], |r| r.get(0),
    ).unwrap_or(0);

    let mut offset: i64 = 0;
    while offset < total {
        let result = conn.execute(
            "INSERT INTO file_fts(path, name_lower) \
             SELECT path, name_lower FROM file_index LIMIT ?1 OFFSET ?2",
            params![BATCH_SIZE, offset],
        );
        if let Err(e) = result {
            eprintln!("[mtool fts5] batch insert failed at offset {}: {}", offset, e);
            conn.execute_batch("PRAGMA synchronous=NORMAL;").ok();
            return false;
        }
        offset += BATCH_SIZE;
        // 让出足够时间，让前端 IPC 有机会获取读锁（避免首次启动 UI 卡死）
        std::thread::sleep(Duration::from_millis(200));
    }

    conn.execute_batch("PRAGMA synchronous=NORMAL;").ok();
    eprintln!("[mtool fts5] rebuild completed successfully ({} rows)", total);
    conn.execute(
        "INSERT OR REPLACE INTO index_meta(key,value) VALUES('fts5_count', ?1)",
        params![total],
    ).ok();
    true
}


// ---------------------------------------------------------------------------
// 查询解析
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct ParsedQuery {
    pub name_terms: Vec<String>,
    pub glob_pattern: Option<String>,
    pub size_filter: Option<SizeFilter>,
    pub content_filter: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SizeFilter {
    pub op: SizeOp,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub enum SizeOp {
    Gt,
    Gte,
    Lt,
    Lte,
    Eq,
}

fn tokenize(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    for ch in s.chars() {
        match ch {
            '"' | '\'' => {
                in_quote = !in_quote;
                current.push(ch);
            }
            ' ' | '\t' if !in_quote => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"'))
            || (s.starts_with('\'') && s.ends_with('\'')))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn parse_size_filter(s: &str) -> Option<SizeFilter> {
    let (op, rest) = if s.starts_with(">=") {
        (SizeOp::Gte, &s[2..])
    } else if s.starts_with("<=") {
        (SizeOp::Lte, &s[2..])
    } else if s.starts_with('>') {
        (SizeOp::Gt, &s[1..])
    } else if s.starts_with('<') {
        (SizeOp::Lt, &s[1..])
    } else {
        (SizeOp::Eq, s)
    };
    let bytes = parse_size_bytes(rest)?;
    Some(SizeFilter { op, bytes })
}

fn parse_size_bytes(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| c.is_alphabetic()).unwrap_or(s.len());
    let (num_str, unit) = s.split_at(split);
    let num: f64 = num_str.trim().parse().ok()?;
    let multiplier: u64 = match unit.to_ascii_lowercase().trim() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1024,
        "m" | "mb" | "mib" => 1024 * 1024,
        "g" | "gb" | "gib" => 1024 * 1024 * 1024,
        "t" | "tb" | "tib" => 1024_u64.pow(4),
        _ => return None,
    };
    Some((num * multiplier as f64).round() as u64)
}

pub fn parse_query(query: &str) -> ParsedQuery {
    let mut result = ParsedQuery::default();
    for token in tokenize(query) {
        let lower = token.to_ascii_lowercase();
        if lower.starts_with("size:") {
            result.size_filter = parse_size_filter(&lower[5..]);
        } else if lower.starts_with("content:") {
            let raw = &token[8..];
            let val = strip_quotes(raw).to_ascii_lowercase();
            if !val.is_empty() {
                result.content_filter = Some(val);
            }
        } else if lower.contains('*') || lower.contains('?') {
            result.glob_pattern = Some(lower);
        } else if !lower.is_empty() {
            result.name_terms.push(lower);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// 内容搜索（字节级，同 cardinal 策略）
// ---------------------------------------------------------------------------

pub fn content_matches(path: &str, needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    if let Ok(meta) = file.metadata() {
        if meta.len() > 50 * 1024 * 1024 {
            return false;
        }
    }
    let overlap = needle.len().saturating_sub(1);
    let mut buffer = vec![0u8; CONTENT_BUFFER_BYTES + overlap];
    let mut carry_len = 0usize;
    loop {
        let Ok(read) = file.read(&mut buffer[carry_len..]) else {
            return false;
        };
        if read == 0 {
            break;
        }
        let chunk_len = carry_len + read;
        let chunk = &mut buffer[..chunk_len];
        chunk[carry_len..].make_ascii_lowercase();
        if find_bytes(chunk, needle) {
            return true;
        }
        let keep = overlap.min(chunk_len);
        if keep > 0 {
            let start = chunk_len - keep;
            chunk.copy_within(start.., 0);
        }
        carry_len = keep;
    }
    false
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

// ---------------------------------------------------------------------------
// 增量更新：单条 upsert / delete
// ---------------------------------------------------------------------------

/// 从路径构建一个 FileEntry，失败返回 None
pub fn entry_from_path(path: &Path) -> Option<FileEntry> {
    let metadata = std::fs::metadata(path).ok()?;
    let name = path.file_name()?.to_string_lossy().to_string();
    if name.is_empty() {
        return None;
    }
    let size = if metadata.is_file() { metadata.len() } else { 0 };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let created = metadata
        .created()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let name_lower = name.to_ascii_lowercase();
    Some(FileEntry {
        name,
        name_lower,
        path: path.to_string_lossy().to_string(),
        size,
        created,
        modified,
        is_dir: metadata.is_dir(),
        ext,
    })
}

/// 在 SQLite 中 upsert 单条记录
pub fn upsert_entry_in_db(conn: &mut Connection, entry: &FileEntry) {
    let tx = match conn.transaction() {
        Ok(t) => t,
        Err(_) => return,
    };
    tx.execute(
        "INSERT OR REPLACE INTO file_index
         (name, path, size, created, modified, is_dir, ext, name_lower)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            entry.name,
            entry.path,
            entry.size as i64,
            entry.created as i64,
            entry.modified as i64,
            entry.is_dir as i32,
            entry.ext,
            entry.name_lower,
        ],
    )
    .ok();
    tx.execute("DELETE FROM file_fts WHERE path=?1", params![entry.path]).ok();
    tx.execute(
        "INSERT INTO file_fts(path, name_lower) VALUES (?1, ?2)",
        params![entry.path, entry.name_lower],
    )
    .ok();
    tx.commit().ok();
}

/// 在 SQLite 中删除单条记录（按路径）
pub fn delete_entry_in_db(conn: &mut Connection, path: &str) {
    let tx = match conn.transaction() {
        Ok(t) => t,
        Err(_) => return,
    };
    tx.execute("DELETE FROM file_index WHERE path=?1", params![path]).ok();
    tx.execute("DELETE FROM file_fts WHERE path=?1", params![path]).ok();
    tx.commit().ok();
}

/// 查询 index_meta 中是否标记为已禁用
pub fn is_index_disabled(db_path: &str) -> bool {
    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    conn.query_row(
        "SELECT value FROM index_meta WHERE key='disabled'",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .map(|v| v == "1")
    .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// SQLite 直接搜索（替代全量内存克隆）
// ---------------------------------------------------------------------------

/// 将 glob pattern 转换为 SQL LIKE 模式（* → %，? → _）
fn glob_to_like(pattern: &str) -> String {
    let mut result = String::new();
    for ch in pattern.chars() {
        match ch {
            '*' => result.push('%'),
            '?' => result.push('_'),
            // 转义 LIKE 的特殊字符
            '%' | '_' | '\\' => { result.push('\\'); result.push(ch); }
            c => result.push(c),
        }
    }
    result
}

fn escape_like(value: &str) -> String {
    let mut result = String::new();
    for ch in value.chars() {
        match ch {
            '%' | '_' | '\\' => {
                result.push('\\');
                result.push(ch);
            }
            _ => result.push(ch),
        }
    }
    result
}

/// 检测 *.ext 纯扩展名 glob，返回扩展名字符串（如 "yaml"）
/// 命中时可直接走 idx_ext 索引，极快
fn extract_ext_from_glob(pattern: &str) -> Option<String> {
    if let Some(rest) = pattern.strip_prefix("*.") {
        if !rest.is_empty() && !rest.contains(['*', '?']) {
            return Some(rest.to_string());
        }
    }
    None
}

/// FTS5 trigram 路径：所有 name_terms >= 3 字符时使用。
/// MATCH 表达式：`"term1" AND "term2"` → trigram 索引，毫秒级 substring 搜索。
fn search_via_fts5(
    conn: &Connection,
    terms: &[String],
    size_filter: Option<&SizeFilter>,
    limit: usize,
) -> Vec<FileEntry> {
    let match_expr = terms
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" AND ");

    let mut extra = String::new();
    if let Some(sf) = size_filter {
        let op = match sf.op {
            SizeOp::Gt => ">", SizeOp::Gte => ">=",
            SizeOp::Lt => "<", SizeOp::Lte => "<=", SizeOp::Eq => "=",
        };
        extra = format!(" AND fi.is_dir = 0 AND fi.size {} {}", op, sf.bytes);
    }

    let sql = format!(
        "SELECT fi.name, fi.path, fi.size, fi.created, fi.modified, \
                fi.is_dir, fi.ext, fi.name_lower \
         FROM file_fts \
         JOIN file_index fi ON fi.path = file_fts.path \
         WHERE file_fts MATCH ?1{} \
         ORDER BY rank \
         LIMIT {}",
        extra, limit
    );

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    stmt.query_map(params![match_expr], |row| {
        Ok(FileEntry {
            name:       row.get(0)?,
            path:       row.get(1)?,
            size:       row.get::<_, i64>(2)? as u64,
            created:    row.get::<_, i64>(3)? as u64,
            modified:   row.get::<_, i64>(4)? as u64,
            is_dir:     row.get::<_, i32>(5)? != 0,
            ext:        row.get(6)?,
            name_lower: row.get(7)?,
        })
    })
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// SQLite LIKE 路径：short terms (< 3 字符) 或 glob / size-only 查询
fn search_via_sqlite(conn: &Connection, parsed: &ParsedQuery, limit: usize) -> Vec<FileEntry> {
    let mut conditions: Vec<String> = Vec::new();
    let mut str_params: Vec<String> = Vec::new();

    // glob pattern
    if let Some(ref pattern) = parsed.glob_pattern {
        if let Some(ext) = extract_ext_from_glob(pattern) {
            // *.yaml → ext = ? —— 命中 idx_ext，毫秒级
            conditions.push("ext = ?".to_string());
            str_params.push(ext);
        } else {
            conditions.push("name_lower LIKE ? ESCAPE '\\'".to_string());
            str_params.push(glob_to_like(pattern));
        }
    }

    // name terms（每个词都要包含）
    for term in &parsed.name_terms {
        conditions.push("name_lower LIKE ? ESCAPE '\\'".to_string());
        str_params.push(format!("%{}%", escape_like(term)));
    }

    // size filter（直接内联数值，不走参数，避免类型复杂度）
    if let Some(ref sf) = parsed.size_filter {
        let op = match sf.op {
            SizeOp::Gt => ">", SizeOp::Gte => ">=",
            SizeOp::Lt => "<", SizeOp::Lte => "<=", SizeOp::Eq => "=",
        };
        conditions.push(format!("is_dir = 0 AND size {} {}", op, sf.bytes));
    }

    let where_sql = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT name,path,size,created,modified,is_dir,ext,name_lower \
         FROM file_index {} LIMIT {}",
        where_sql, limit
    );

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let dyn_params: Vec<&dyn rusqlite::ToSql> =
        str_params.iter().map(|s| s as &dyn rusqlite::ToSql).collect();

    stmt.query_map(dyn_params.as_slice(), |row| {
        Ok(FileEntry {
            name:       row.get(0)?,
            path:       row.get(1)?,
            size:       row.get::<_, i64>(2)? as u64,
            created:    row.get::<_, i64>(3)? as u64,
            modified:   row.get::<_, i64>(4)? as u64,
            is_dir:     row.get::<_, i32>(5)? != 0,
            ext:        row.get(6)?,
            name_lower: row.get(7)?,
        })
    })
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// FTS5 trigram 不支持混合脚本（如 "C扫B"），分词后无法匹配子串。
fn has_mixed_script(term: &str) -> bool {
    let has_ascii_alpha = term.chars().any(|c| c.is_ascii_alphabetic());
    let has_non_ascii = term.chars().any(|c| !c.is_ascii());
    has_ascii_alpha && has_non_ascii
}

/// 搜索路由：
///   1. name_terms 全 >= 3 字符 且 FTS5 有数据 → FTS5 trigram
///   2. 否则 → SQLite LIKE（走 idx_name_lower 前缀扫描）
pub fn search_in_db(
    db_path: &str,
    parsed: &ParsedQuery,
    limit: usize,
    fts5_is_ready: bool,
) -> Vec<FileEntry> {
    if parsed.name_terms.is_empty() {
        let conn = match Connection::open(db_path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let mut results = search_via_sqlite(&conn, parsed, limit);
        sort_by_prefix_match(&mut results, &parsed.name_terms);
        return results;
    }

    // 有 name_terms：先判断能否用 FTS5 trigram（长词，3+ 字符）
    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let fts5_usable = parsed.glob_pattern.is_none()
        && parsed.name_terms.iter().all(|t| {
            t.chars().count() >= 3 && !has_mixed_script(t)
        })
        && fts5_is_ready;

    if fts5_usable {
        let mut results = search_via_fts5(&conn, &parsed.name_terms, parsed.size_filter.as_ref(), limit);
        sort_by_prefix_match(&mut results, &parsed.name_terms);
        return results;
    }

    // 短词（< 3 字符）或 FTS5 未就绪 → SQLite LIKE，走 idx_name_lower 前缀扫描
    let mut results = search_via_sqlite(&conn, parsed, limit);
    sort_by_prefix_match(&mut results, &parsed.name_terms);
    results
}

/// 按前缀匹配优先排序：完全匹配(0) > 前缀匹配(1) > 子串匹配(2)
pub fn sort_by_prefix_match(entries: &mut [FileEntry], terms: &[String]) {
    if terms.is_empty() {
        return;
    }
    entries.sort_by(|a, b| {
        let a_score = prefix_score(&a.name_lower, terms);
        let b_score = prefix_score(&b.name_lower, terms);
        a_score.cmp(&b_score)
    });
}

fn prefix_score(name: &str, terms: &[String]) -> u8 {
    terms.iter().map(|t| {
        if name == t.as_str() { 0 }
        else if name.starts_with(t.as_str()) { 1 }
        else { 2 }
    }).min().unwrap_or(3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn search_in_db_treats_underscore_as_literal_in_name_terms() {
        let db_path = temp_db_path("underscore");
        init_db(&db_path);
        let conn = Connection::open(&db_path).unwrap();

        insert_entry(&conn, "my_file", "/tmp/my_file");
        insert_entry(&conn, "myxfile", "/tmp/myxfile");

        let parsed = ParsedQuery {
            name_terms: vec!["my_file".to_string()],
            ..Default::default()
        };
        let results = search_in_db(&db_path, &parsed, 10, false);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "my_file");
        cleanup_db_files(&db_path);
    }

    #[test]
    fn search_in_db_treats_percent_as_literal_in_name_terms() {
        let db_path = temp_db_path("percent");
        init_db(&db_path);
        let conn = Connection::open(&db_path).unwrap();

        insert_entry(&conn, "100%", "/tmp/100pct");
        insert_entry(&conn, "1000", "/tmp/1000");

        let parsed = ParsedQuery {
            name_terms: vec!["100%".to_string()],
            ..Default::default()
        };
        let results = search_in_db(&db_path, &parsed, 10, false);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "100%");
        cleanup_db_files(&db_path);
    }

    fn temp_db_path(tag: &str) -> String {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir()
            .join(format!("mtool-{tag}-{nanos}.db"))
            .to_string_lossy()
            .to_string()
    }

    fn insert_entry(conn: &Connection, name: &str, path: &str) {
        conn.execute(
            "INSERT INTO file_index(name, path, size, created, modified, is_dir, ext, name_lower) \
             VALUES(?1, ?2, 0, 0, 0, 0, '', ?3)",
            params![name, path, name.to_ascii_lowercase()],
        )
        .unwrap();
    }

    fn cleanup_db_files(db_path: &str) {
        let _ = std::fs::remove_file(db_path);
        let _ = std::fs::remove_file(format!("{}-wal", db_path));
        let _ = std::fs::remove_file(format!("{}-shm", db_path));
    }
}
