use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::UNIX_EPOCH;
use walkdir::{DirEntry, WalkDir};

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
}

impl IndexEngine {
    pub fn new_with_db(db_path: String) -> Self {
        Self {
            status: Arc::new(RwLock::new(IndexStatus::default())),
            db_path,
        }
    }

    pub fn load_from_db(&self) {
        let total = count_entries(&self.db_path);
        let last_built_at = get_last_built_at(&self.db_path);
        let mut s = self.status.write().unwrap();
        s.total = total;
        s.last_built_at = last_built_at;
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
         PRAGMA cache_size=-65536;",
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
/// 保留 index_meta 表，避免丢失 last_built_at / last_exit_at。
fn reset_index_tables(conn: &Connection) {
    conn.execute_batch(
        "DROP TABLE IF EXISTS file_index;
         DROP TABLE IF EXISTS file_fts;",
    )
    .ok();
    conn.execute_batch(SCHEMA_SQL).ok();
}


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

pub fn get_last_exit_at(db_path: &str) -> Option<u64> {
    let conn = Connection::open(db_path).ok()?;
    conn.query_row(
        "SELECT value FROM index_meta WHERE key='last_exit_at'",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|v| v.parse::<u64>().ok())
}

pub fn set_last_exit_at(db_path: &str, ts: u64) {
    if let Ok(conn) = Connection::open(db_path) {
        conn.execute(
            "INSERT OR REPLACE INTO index_meta(key,value) VALUES('last_exit_at',?1)",
            params![ts.to_string()],
        )
        .ok();
    }
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
            // 系统库 / 缓存 / 临时目录
            "/private/var/vm",
            "/private/var/folders",
            "/private/var/db/com.apple.xpc",
            "/private/var/db/dyld",
            "/private/tmp",
            "/private/var/tmp",
            // Spotlight 索引
            "/.Spotlight-V100",
            "/.fseventsd",
            // 崩溃转储
            "/cores",
            // Recovery 分区
            "/Volumes/Recovery",
            // 系统完整性保护区域
            "/System/Library/Caches",
            "/Library/Caches",
            // Xcode 派生数据 / 模拟器（通常体积巨大）
            "/Library/Developer/CoreSimulator",
        ];
        for prefix in &skip_prefixes {
            if path_str.starts_with(prefix) {
                return true;
            }
        }
        // 跳过所有以 "." 开头的隐藏目录（仅根目录下一级），如 /.Trash、/.DocumentRevisions-V100
        if let Some(stripped) = path_str.strip_prefix('/') {
            if let Some(first_component) = stripped.split('/').next() {
                if first_component.starts_with('.') {
                    return true;
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let skip_prefixes = [
            "/dev", "/proc", "/sys", "/run", "/tmp", "/var/run", "/var/lock",
        ];
        for prefix in &skip_prefixes {
            if path_str.starts_with(prefix) {
                return true;
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let lower = path_str.to_ascii_lowercase();
        // 跳过以这些路径开头的目录（含其本身和所有子项）
        let skip_prefixes = [
            "\\windows",
            "\\$recycle.bin",
            "\\system volume information",
            "\\programdata\\microsoft\\windows",
        ];
        // 去掉盘符前缀后匹配（如 c:\windows -> \windows）
        let without_drive = if lower.len() >= 2 && lower.as_bytes()[1] == b':' {
            &lower[2..]
        } else {
            lower.as_ref()
        };
        for prefix in &skip_prefixes {
            if without_drive == *prefix
                || without_drive.starts_with(&format!("{}\\", prefix))
            {
                return true;
            }
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

fn get_system_roots() -> Vec<PathBuf> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        vec![PathBuf::from("/")]
    }

    #[cfg(target_os = "windows")]
    {
        // 枚举 A-Z 盘符
        (b'A'..=b'Z')
            .map(|c| PathBuf::from(format!("{}:\\", c as char)))
            .filter(|p| p.exists())
            .collect()
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        vec![PathBuf::from("/")]
    }
}

// ---------------------------------------------------------------------------
// 索引构建（流式写入，无中间 Vec，峰值内存 ≈ BATCH_SIZE × entry_size ≈ 15 MB）
// ---------------------------------------------------------------------------

/// 从 walkdir::DirEntry 构建 FileEntry；失败返回 None
fn entry_from_dir_entry(dir_entry: &DirEntry) -> Option<FileEntry> {
    let path = dir_entry.path();
    let metadata = dir_entry.metadata().ok()?;
    let name = path.file_name()?.to_string_lossy().to_string();
    if name.is_empty() { return None; }
    let size = if metadata.is_file() { metadata.len() } else { 0 };
    let modified = metadata.modified().ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs()).unwrap_or(0);
    let created = metadata.created().ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs()).unwrap_or(0);
    let ext = path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let name_lower = name.to_ascii_lowercase();
    Some(FileEntry {
        name, name_lower,
        path: path.to_string_lossy().to_string(),
        size, created, modified,
        is_dir: metadata.is_dir(),
        ext,
    })
}

/// 将一批 FileEntry 写入 SQLite（file_index + file_fts），包裹在单次事务中。
fn flush_batch(conn: &mut Connection, batch: &[FileEntry]) {
    if batch.is_empty() { return; }
    let tx = match conn.transaction() { Ok(t) => t, Err(_) => return };
    {
        let mut stmt = match tx.prepare(
            "INSERT OR REPLACE INTO file_index \
             (name,path,size,created,modified,is_dir,ext,name_lower) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        ) { Ok(s) => s, Err(_) => return };
        let mut fts_stmt = match tx.prepare(
            "INSERT INTO file_fts(path, name_lower) VALUES (?1, ?2)",
        ) { Ok(s) => s, Err(_) => return };
        for e in batch {
            stmt.execute(params![
                e.name, e.path, e.size as i64, e.created as i64,
                e.modified as i64, e.is_dir as i32, e.ext, e.name_lower,
            ]).ok();
            fts_stmt.execute(params![e.path, e.name_lower]).ok();
        }
    }
    tx.commit().ok();
}

/// 流式全量建索引：边扫描边写库，每 BATCH_SIZE 条提交一次事务。
/// 峰值内存 ≈ BATCH_SIZE × ~300 B ≈ 15 MB，消灭全量中间 Vec。
/// 每 PROGRESS_INTERVAL 条或每次批次提交后调用 on_progress(已写总数, 当前扫描路径)。
pub fn build_index_streaming<F>(db_path: &str, on_progress: F) -> usize
where
    F: Fn(usize, Option<&str>),
{
    const BATCH_SIZE: usize = 50_000;
    const PROGRESS_INTERVAL: usize = 1_000;
    // 立即发出 0 进度，让前端从"0 个文件"立刻进入"建索引中"状态
    on_progress(0, None);
    let mut conn = match Connection::open(db_path) { Ok(c) => c, Err(_) => return 0 };
    // DROP + 重建代替 DELETE，900K 行清空 <10ms
    reset_index_tables(&conn);
    let roots = get_system_roots();
    let mut batch: Vec<FileEntry> = Vec::with_capacity(BATCH_SIZE);
    let mut total = 0usize;
    for root in &roots {
        let walker = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !should_skip_path(e.path()));
        for result in walker {
            let Ok(dir_entry) = result else { continue };
            let Some(entry) = entry_from_dir_entry(&dir_entry) else { continue };
            let cur_path = entry.path.clone();
            batch.push(entry);
            if batch.len() % PROGRESS_INTERVAL == 0 {
                on_progress(total + batch.len(), Some(&cur_path));
            }
            if batch.len() >= BATCH_SIZE {
                flush_batch(&mut conn, &batch);
                total += batch.len();
                on_progress(total, Some(&cur_path));
                batch.clear();
            }
        }
    }
    if !batch.is_empty() {
        flush_batch(&mut conn, &batch);
        total += batch.len();
        on_progress(total, None);
    }
    total
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
            '"' => {
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
pub fn upsert_entry_in_db(db_path: &str, entry: &FileEntry) {
    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(_) => return,
    };
    conn.execute(
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
    conn.execute("DELETE FROM file_fts WHERE path=?1", params![entry.path]).ok();
    conn.execute(
        "INSERT INTO file_fts(path, name_lower) VALUES (?1, ?2)",
        params![entry.path, entry.name_lower],
    )
    .ok();
}

/// 在 SQLite 中删除单条记录（按路径）
pub fn delete_entry_in_db(db_path: &str, path: &str) {
    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(_) => return,
    };
    conn.execute("DELETE FROM file_index WHERE path=?1", params![path]).ok();
    conn.execute("DELETE FROM file_fts WHERE path=?1", params![path]).ok();
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
        conditions.push("name_lower LIKE ?".to_string());
        str_params.push(format!("%{}%", term));
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

/// 搜索路由：name_terms 全部 >= 3 字符 → FTS5 trigram（毫秒级）；否则 → SQLite LIKE
pub fn search_in_db(db_path: &str, parsed: &ParsedQuery, limit: usize) -> Vec<FileEntry> {
    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let use_fts5 = !parsed.name_terms.is_empty()
        && parsed.glob_pattern.is_none()
        && parsed.name_terms.iter().all(|t| t.chars().count() >= 3);

    if use_fts5 {
        return search_via_fts5(&conn, &parsed.name_terms, parsed.size_filter.as_ref(), limit);
    }

    search_via_sqlite(&conn, parsed, limit)
}

// ---------------------------------------------------------------------------
// 启动增量对账
// ---------------------------------------------------------------------------

/// 扫描 watch roots，将 mtime > since_ts 的文件 upsert 到 DB。
/// 适用于 MTOOL 关闭期间文件被修改/新增的场景。
/// on_progress(scanned, updated) 每 2000 条调一次。
/// 返回本次更新的条目数。
pub fn reconcile_changed_since<F>(
    db_path: &str,
    since_ts: u64,
    on_progress: F,
) -> usize
where
    F: Fn(usize, usize),
{
    let roots = get_watch_roots();
    let mut scanned = 0usize;
    let mut updated = 0usize;

    for root in &roots {
        if !root.exists() {
            continue;
        }
        let walker = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !should_skip_path(e.path()));

        for result in walker {
            let dir_entry = match result {
                Ok(e) => e,
                Err(_) => continue,
            };
            scanned += 1;

            let path = dir_entry.path();
            let meta = match dir_entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };

            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            if scanned % 2000 == 0 {
                on_progress(scanned, updated);
            }

            if mtime <= since_ts {
                continue;
            }

            // mtime > since_ts → 需要 upsert（仅更新 DB，无内存 Vec）
            if let Some(entry) = entry_from_path(path) {
                upsert_entry_in_db(db_path, &entry);
                updated += 1;
            }
        }
    }

    on_progress(scanned, updated);
    updated
}
