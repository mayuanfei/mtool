use base64::{engine::general_purpose::STANDARD, Engine as _};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const ULEARN_HOME: &str = "https://ulearn.cup.com.cn/home";
const MERCHANT_HOME: &str = "https://ysstudy.lzdxedu.com/login";
const BRIDGE_CAPTURE_PREFIX: &str = "MTOOL_CAPTURE|";
const BRIDGE_MEDIA_PREFIX: &str = "MTOOL_MEDIA|";
const CAPTURE_CHUNK_SIZE: usize = 320;
const CAPTURE_CHUNK_INTERVAL_MS: u64 = 100;
static CAPTURE_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
enum Provider {
    Ulearn,
    Merchant,
}

impl Provider {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "ulearn" => Ok(Self::Ulearn),
            "merchant" => Ok(Self::Merchant),
            _ => Err("不支持的学习平台".to_string()),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Ulearn => "ulearn",
            Self::Merchant => "merchant",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Ulearn => "银联乐学",
            Self::Merchant => "银商学堂",
        }
    }

    fn home(self) -> &'static str {
        match self {
            Self::Ulearn => ULEARN_HOME,
            Self::Merchant => MERCHANT_HOME,
        }
    }

    fn label(self) -> String {
        format!("video-task-{}", self.key())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoTaskSettings {
    speed: f64,
    muted: bool,
    cross_site_parallel: bool,
    running: bool,
}

impl Default for VideoTaskSettings {
    fn default() -> Self {
        Self {
            speed: 2.0,
            muted: true,
            cross_site_parallel: false,
            running: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageCourseCapture {
    external_id: String,
    title: String,
    url: String,
    locator: String,
    section_title: String,
    kind: String,
    duration_seconds: i64,
    progress: f64,
    completed: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageTopicCapture {
    title: String,
    url: String,
    progress: f64,
    total_count: i64,
    completed_count: i64,
    courses: Vec<PageCourseCapture>,
}

#[derive(Default)]
struct CaptureBuffer {
    total: usize,
    encoded_len: usize,
    chunks: Vec<Option<String>>,
}

#[derive(Default)]
struct CaptureExchange {
    buffers: HashMap<String, CaptureBuffer>,
    completed: HashMap<String, Result<PageTopicCapture, String>>,
}

#[derive(Clone, Debug)]
struct ActiveCourse {
    course_id: String,
    topic_id: String,
    provider: Provider,
    phase: String,
    phase_since: i64,
    last_media_at: i64,
}

#[derive(Default)]
struct RuntimeState {
    active: HashMap<String, ActiveCourse>,
}

#[derive(Clone)]
pub struct VideoTaskState {
    db_path: Arc<PathBuf>,
    settings: Arc<Mutex<VideoTaskSettings>>,
    captures: Arc<Mutex<CaptureExchange>>,
    runtime: Arc<Mutex<RuntimeState>>,
}

impl Default for VideoTaskState {
    fn default() -> Self {
        let data_dir = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
        let app_dir = data_dir.join("mtool");
        let _ = std::fs::create_dir_all(&app_dir);
        let db_path = app_dir.join("mtool_video_tasks.db");
        if let Err(error) = init_db(&db_path) {
            eprintln!("[mtool video task] database init failed: {error}");
        }
        let mut settings = load_settings(&db_path).unwrap_or_default();
        // 应用重启后保持暂停，避免在用户未确认时自动恢复学习队列。
        settings.running = false;
        let _ = persist_settings(&db_path, &settings);
        Self {
            db_path: Arc::new(db_path),
            settings: Arc::new(Mutex::new(settings)),
            captures: Arc::new(Mutex::new(CaptureExchange::default())),
            runtime: Arc::new(Mutex::new(RuntimeState::default())),
        }
    }
}

#[derive(Clone, Debug)]
struct CourseRecord {
    id: String,
    topic_id: String,
    provider: Provider,
    url: String,
    locator: String,
    kind: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatus {
    provider: String,
    name: String,
    home_url: String,
    window_open: bool,
    current_url: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseItem {
    id: String,
    title: String,
    url: String,
    section_title: String,
    kind: String,
    duration_seconds: i64,
    progress: f64,
    status: String,
    last_error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicItem {
    id: String,
    provider: String,
    title: String,
    url: String,
    progress: f64,
    total_count: i64,
    completed_count: i64,
    last_synced_at: i64,
    courses: Vec<CourseItem>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueStats {
    total: usize,
    completed: usize,
    pending: usize,
    running: usize,
    manual: usize,
    attention: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoTaskDashboard {
    settings: VideoTaskSettings,
    sources: Vec<SourceStatus>,
    topics: Vec<TopicItem>,
    stats: QueueStats,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    topic_id: String,
    topic_title: String,
    imported: usize,
    completed: usize,
    manual: usize,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn stable_id(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn clamp_speed(speed: f64) -> f64 {
    if speed.is_finite() {
        speed.clamp(1.0, 2.0)
    } else {
        2.0
    }
}

fn provider_accepts_url(provider: Provider, url: &tauri::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    match provider {
        Provider::Ulearn => host == "cup.com.cn" || host.ends_with(".cup.com.cn"),
        Provider::Merchant => host == "lzdxedu.com" || host.ends_with(".lzdxedu.com"),
    }
}

fn init_db(path: &PathBuf) -> Result<(), String> {
    let conn =
        Connection::open(path).map_err(|error| format!("打开视频任务数据库失败: {error}"))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS video_settings (
           key TEXT PRIMARY KEY,
           value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS video_topics (
           id TEXT PRIMARY KEY,
           provider TEXT NOT NULL,
           title TEXT NOT NULL,
           url TEXT NOT NULL,
           progress REAL NOT NULL DEFAULT 0,
           total_count INTEGER NOT NULL DEFAULT 0,
           completed_count INTEGER NOT NULL DEFAULT 0,
           last_synced_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS video_courses (
           id TEXT PRIMARY KEY,
           topic_id TEXT NOT NULL REFERENCES video_topics(id) ON DELETE CASCADE,
           provider TEXT NOT NULL,
           external_id TEXT NOT NULL,
           title TEXT NOT NULL,
           url TEXT NOT NULL DEFAULT '',
           locator TEXT NOT NULL DEFAULT '',
           section_title TEXT NOT NULL DEFAULT '',
           kind TEXT NOT NULL DEFAULT 'video',
           duration_seconds INTEGER NOT NULL DEFAULT 0,
           progress REAL NOT NULL DEFAULT 0,
           status TEXT NOT NULL DEFAULT 'pending',
           sort_order INTEGER NOT NULL DEFAULT 0,
           last_error TEXT,
           updated_at INTEGER NOT NULL,
           UNIQUE(topic_id, external_id)
         );
         CREATE INDEX IF NOT EXISTS idx_video_courses_queue
           ON video_courses(status, provider, sort_order);",
    )
    .map_err(|error| format!("初始化视频任务数据库失败: {error}"))?;
    conn.execute(
        "UPDATE video_courses
         SET status='pending',last_error='上次运行被中断，已重新加入队列'
         WHERE kind='video' AND status IN('playing','verifying')",
        [],
    )
    .map_err(|error| format!("恢复未完成视频任务失败: {error}"))?;
    Ok(())
}

fn load_settings(path: &PathBuf) -> Result<VideoTaskSettings, String> {
    let conn = Connection::open(path).map_err(|error| error.to_string())?;
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM video_settings WHERE key='settings'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(value
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default())
}

fn persist_settings(path: &PathBuf, settings: &VideoTaskSettings) -> Result<(), String> {
    let conn = Connection::open(path).map_err(|error| error.to_string())?;
    let value = serde_json::to_string(settings).map_err(|error| error.to_string())?;
    conn.execute(
        "INSERT INTO video_settings(key,value) VALUES('settings',?1)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![value],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn bridge_script(provider: Provider, speed: f64, muted: bool) -> String {
    const TEMPLATE: &str = r##"
(() => {
  if (window.__MTOOL_LEARNING_BRIDGE__) return;
  const provider = "__PROVIDER__";
  const state = { speed: __SPEED__, muted: __MUTED__, tracked: new WeakSet() };
  const setTitleMessage = (message) => {
    const previous = document.title;
    document.title = message;
    window.setTimeout(() => {
      if (document.title === message) document.title = previous;
    }, 80);
  };
  const report = (eventName, media) => {
    const message = "MTOOL_MEDIA|" + provider + "|" + eventName + "|" +
      (Number(media.currentTime) || 0) + "|" + (Number(media.duration) || 0);
    if (window.top === window) setTitleMessage(message);
    else {
      try { window.top.postMessage({ __mtoolMedia: message }, "*"); } catch (_) {}
    }
  };
  if (window.top === window) {
    window.addEventListener("message", (event) => {
      if (event.data && event.data.__mtoolMedia) setTitleMessage(event.data.__mtoolMedia);
    });
  }
  const track = (media) => {
    if (state.tracked.has(media)) return;
    state.tracked.add(media);
    ["play", "pause", "ended", "error"].forEach((name) => {
      media.addEventListener(name, () => report(name, media), true);
    });
    media.addEventListener("timeupdate", () => {
      if (!media.__mtoolLastReport || Date.now() - media.__mtoolLastReport > 5000) {
        media.__mtoolLastReport = Date.now();
        report("timeupdate", media);
      }
    }, true);
  };
  const apply = () => {
    document.querySelectorAll("video,audio").forEach((media) => {
      track(media);
      try { media.defaultPlaybackRate = state.speed; media.playbackRate = state.speed; } catch (_) {}
      try { media.muted = state.muted; } catch (_) {}
    });
  };
  state.update = (speedValue, mutedValue) => {
    state.speed = Math.min(2, Math.max(1, Number(speedValue) || 2));
    state.muted = Boolean(mutedValue);
    apply();
  };
  Object.defineProperty(window, "__MTOOL_LEARNING_BRIDGE__", { value: state });
  const start = () => {
    apply();
    new MutationObserver(apply).observe(document.documentElement || document, { childList: true, subtree: true });
    window.setInterval(apply, 1000);
  };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start, { once: true });
  else start();
})();
"##;
    TEMPLATE
        .replace("__PROVIDER__", provider.key())
        .replace("__SPEED__", &clamp_speed(speed).to_string())
        .replace("__MUTED__", if muted { "true" } else { "false" })
}

fn update_media_script(speed: f64, muted: bool, auto_play: bool) -> String {
    format!(
        r#"(() => {{
          const speed = {};
          const muted = {};
          if (window.__MTOOL_LEARNING_BRIDGE__) window.__MTOOL_LEARNING_BRIDGE__.update(speed, muted);
          document.querySelectorAll("video,audio").forEach((media) => {{
            try {{ media.defaultPlaybackRate = speed; media.playbackRate = speed; media.muted = muted; }} catch (_) {{}}
            {}
          }});
        }})();"#,
        clamp_speed(speed),
        if muted { "true" } else { "false" },
        if auto_play {
            "try { const result = media.play(); if (result && result.catch) result.catch(() => {}); } catch (_) {}"
        } else {
            ""
        }
    )
}

fn capture_script(request_id: &str, provider: Provider) -> String {
    const TEMPLATE: &str = r##"
(() => {
  const requestId = "__REQUEST_ID__";
  const provider = "__PROVIDER__";
  const clean = (value) => String(value || "").replace(/\s+/g, " ").trim();
  const ownText = (element) => clean(Array.from(element.childNodes || [])
    .filter((node) => node.nodeType === Node.TEXT_NODE).map((node) => node.textContent).join(" "));
  const visible = (element) => {
    const style = window.getComputedStyle(element);
    return style.display !== "none" && style.visibility !== "hidden";
  };
  const cssPath = (element) => {
    if (!element || element === document.body) return "body";
    const parts = [];
    let current = element;
    while (current && current !== document.body && parts.length < 7) {
      if (current.id) { parts.unshift("#" + CSS.escape(current.id)); break; }
      let part = current.tagName.toLowerCase();
      const siblings = current.parentElement ? Array.from(current.parentElement.children)
        .filter((item) => item.tagName === current.tagName) : [];
      if (siblings.length > 1) part += ":nth-of-type(" + (siblings.indexOf(current) + 1) + ")";
      parts.unshift(part);
      current = current.parentElement;
    }
    return parts.join(" > ");
  };
  const courseContainer = (marker) => {
    let element = marker;
    for (let depth = 0; element && depth < 8; depth++, element = element.parentElement) {
      const text = clean(element.innerText);
      if (text.length >= 12 && text.length <= 900) {
        if (provider === "merchant" && /学习时长/.test(text) && /进度/.test(text)) return element;
        if (provider === "ulearn" && /(学时|学分)/.test(text) && /(未学习|已学习|学习中)/.test(text)) return element;
      }
    }
    return marker.parentElement || marker;
  };
  const titleFrom = (container) => {
    const lines = String(container.innerText || "").split(/\n+/).map(clean).filter(Boolean);
    const ignored = /^(未学习|已学习|学习中|已完成|已考试|课程|知识|考试|选修)$/;
    const meta = /(学习时长|必修学分|选修学分|进度\s*[:：]|学时\s|学分\s)/;
    return lines.find((line) => line.length > 2 && !ignored.test(line) && !meta.test(line)) || lines[0] || "未命名课程";
  };
  const linkFrom = (container) => {
    const anchor = container.matches && container.matches("a[href]") ? container : container.querySelector("a[href]");
    if (!anchor) return "";
    const href = anchor.getAttribute("href") || "";
    if (!href || /^javascript:/i.test(href)) return "";
    try { return new URL(href, location.href).href; } catch (_) { return ""; }
  };
  const externalIdFrom = (container, url, locator, title) => {
    let element = container;
    for (let depth = 0; element && depth < 5; depth++, element = element.parentElement) {
      const data = element.dataset || {};
      const value = data.courseId || data.contentId || data.knowledgeId || data.resourceId || data.id;
      if (value) return String(value);
    }
    return url || locator || title;
  };
  const sectionTitleFrom = (container) => {
    if (provider !== "merchant") return "";
    let current = container;
    for (let depth = 0; current && current.parentElement && depth < 5; depth++, current = current.parentElement) {
      const siblings = Array.from(current.parentElement.children);
      const index = siblings.indexOf(current);
      for (let offset = index - 1; offset >= 0; offset--) {
        const text = clean(siblings[offset].innerText);
        if (text && text.length <= 80 && (/^\d{1,2}\s/.test(text) || /^第.+期/.test(text))) return text;
      }
    }
    return "";
  };
  const markers = Array.from(document.querySelectorAll("body *")).filter((element) => {
    if (!visible(element)) return false;
    const text = ownText(element);
    if (provider === "merchant") return /^(已完成|已考试)$/.test(text) || /^进度\s*[:：]?\s*\d+(?:\.\d+)?%$/.test(text);
    return /^(未学习|已学习|学习中)$/.test(text);
  });
  const seen = new Set();
  const courses = [];
  markers.forEach((marker) => {
    const container = courseContainer(marker);
    const text = clean(container.innerText);
    const title = titleFrom(container);
    const locator = cssPath(container);
    const url = linkFrom(container);
    const externalId = externalIdFrom(container, url, locator, title);
    if (!title || seen.has(externalId)) return;
    seen.add(externalId);
    const progressMatch = text.match(/进度\s*[:：]?\s*(\d+(?:\.\d+)?)%/);
    const progress = progressMatch ? Number(progressMatch[1]) : (/(已学习|已完成|已考试)/.test(text) ? 100 : 0);
    const durationMatch = text.match(/学习时长\s*[:：]?\s*(\d+)\s*分钟/);
    let kind = "video";
    if (/考试/.test(title) || /(^|\s)考试(\s|$)/.test(text)) kind = "exam";
    else if (/课件/.test(title)) kind = "slides";
    else if (/(文档|阅读材料|参考资料)/.test(title)) kind = "material";
    courses.push({
      externalId,
      title,
      url,
      locator,
      sectionTitle: sectionTitleFrom(container),
      kind,
      durationSeconds: durationMatch ? Number(durationMatch[1]) * 60 : 0,
      progress,
      completed: progress >= 100 || /(已学习|已完成|已考试)/.test(text)
    });
  });
  const bodyText = clean(document.body.innerText);
  const topicTitle = clean((document.querySelector("h1,h2,[class*='title']") || {}).textContent) || document.title || location.hostname;
  const topicProgressMatch = bodyText.match(/学习进度\s*[:：]?\s*(\d+(?:\.\d+)?)%/);
  const countMatch = bodyText.match(/完成任务数\s*(\d+)\s*\/\s*(\d+)/);
  const completedCount = countMatch ? Number(countMatch[1]) : courses.filter((item) => item.completed).length;
  const totalCount = countMatch ? Number(countMatch[2]) : courses.length;
  const payload = {
    title: topicTitle,
    url: location.href,
    progress: topicProgressMatch ? Number(topicProgressMatch[1]) : (totalCount ? completedCount / totalCount * 100 : 0),
    totalCount,
    completedCount,
    courses
  };
  const bytes = new TextEncoder().encode(JSON.stringify(payload));
  let binary = "";
  bytes.forEach((byte) => { binary += String.fromCharCode(byte); });
  const encoded = btoa(binary);
  const chunkSize = __CHUNK_SIZE__;
  const chunks = encoded.match(new RegExp(".{1," + chunkSize + "}", "g")) || [""];
  const originalTitle = document.title;
  chunks.forEach((chunk, index) => {
    window.setTimeout(() => {
      document.title = "MTOOL_CAPTURE|" + requestId + "|" + index + "|" + chunks.length + "|" + encoded.length + "|" + chunk;
      if (index === chunks.length - 1) window.setTimeout(() => { document.title = originalTitle; }, __RESTORE_DELAY__);
    }, index * __CHUNK_INTERVAL__);
  });
})();
"##;
    TEMPLATE
        .replace("__REQUEST_ID__", request_id)
        .replace("__PROVIDER__", provider.key())
        .replace("__CHUNK_SIZE__", &CAPTURE_CHUNK_SIZE.to_string())
        .replace("__CHUNK_INTERVAL__", &CAPTURE_CHUNK_INTERVAL_MS.to_string())
        .replace(
            "__RESTORE_DELAY__",
            &(CAPTURE_CHUNK_INTERVAL_MS * 2).to_string(),
        )
}

fn decode_capture_buffer(buffer: &CaptureBuffer) -> Result<PageTopicCapture, String> {
    let joined = buffer
        .chunks
        .iter()
        .filter_map(|part| part.as_ref())
        .cloned()
        .collect::<String>();
    if joined.len() != buffer.encoded_len {
        return Err(format!(
            "专题页面数据分块不完整（应为 {} 字符，实际 {} 字符）",
            buffer.encoded_len,
            joined.len()
        ));
    }
    STANDARD
        .decode(joined)
        .map_err(|error| format!("解析专题页面数据失败: {error}"))
        .and_then(|bytes| String::from_utf8(bytes).map_err(|error| error.to_string()))
        .and_then(|json| {
            serde_json::from_str::<PageTopicCapture>(&json)
                .map_err(|error| format!("专题页面数据格式错误: {error}"))
        })
}

fn handle_bridge_title(
    title: &str,
    provider: Provider,
    captures: &Arc<Mutex<CaptureExchange>>,
    runtime: &Arc<Mutex<RuntimeState>>,
) -> bool {
    if let Some(payload) = title.strip_prefix(BRIDGE_CAPTURE_PREFIX) {
        let parts: Vec<&str> = payload.splitn(5, '|').collect();
        if parts.len() != 5 {
            return true;
        }
        let request_id = parts[0].to_string();
        let index = parts[1].parse::<usize>().unwrap_or(0);
        let total = parts[2].parse::<usize>().unwrap_or(0);
        let encoded_len = parts[3].parse::<usize>().unwrap_or(0);
        let chunk = parts[4].to_string();
        if total == 0 || encoded_len == 0 || index >= total {
            return true;
        }
        let mut exchange = captures.lock().unwrap_or_else(|error| error.into_inner());
        let buffer = exchange
            .buffers
            .entry(request_id.clone())
            .or_insert_with(|| CaptureBuffer {
                total,
                encoded_len,
                chunks: vec![None; total],
            });
        if buffer.total != total || buffer.encoded_len != encoded_len {
            *buffer = CaptureBuffer {
                total,
                encoded_len,
                chunks: vec![None; total],
            };
        }
        buffer.chunks[index] = Some(chunk);
        if buffer.chunks.iter().all(Option::is_some) {
            let parsed = decode_capture_buffer(buffer);
            exchange.buffers.remove(&request_id);
            exchange.completed.insert(request_id, parsed);
        }
        return true;
    }

    if let Some(payload) = title.strip_prefix(BRIDGE_MEDIA_PREFIX) {
        let parts: Vec<&str> = payload.split('|').collect();
        if parts.len() >= 4 && parts[0] == provider.key() {
            let event = parts[1];
            let mut state = runtime.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(active) = state.active.get_mut(provider.key()) {
                active.last_media_at = now();
                if event == "ended" {
                    active.phase = "ended".to_string();
                    active.phase_since = now();
                } else if event == "error" {
                    active.phase = "error".to_string();
                    active.phase_since = now();
                }
            }
        }
        return true;
    }
    false
}

async fn ensure_window(
    app: &AppHandle,
    state: &VideoTaskState,
    provider: Provider,
    show: bool,
) -> Result<tauri::WebviewWindow, String> {
    let label = provider.label();
    if let Some(window) = app.get_webview_window(&label) {
        if show {
            window.show().map_err(|error| error.to_string())?;
            window.unminimize().map_err(|error| error.to_string())?;
            window.set_focus().map_err(|error| error.to_string())?;
        }
        return Ok(window);
    }
    let settings = state
        .settings
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let url = provider
        .home()
        .parse::<tauri::Url>()
        .map_err(|error| error.to_string())?;
    let captures = state.captures.clone();
    let runtime = state.runtime.clone();
    let provider_for_title = provider;
    let app_for_popup = app.clone();
    let popup_label = label.clone();
    let settings_state = state.settings.clone();
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        .title(format!("MTOOL · {}", provider.name()))
        .inner_size(1280.0, 820.0)
        .min_inner_size(640.0, 480.0)
        .visible(show)
        .initialization_script_for_all_frames(bridge_script(
            provider,
            settings.speed,
            settings.muted,
        ))
        .on_document_title_changed(move |window, title| {
            if !handle_bridge_title(&title, provider_for_title, &captures, &runtime) {
                let _ =
                    window.set_title(&format!("MTOOL · {} · {title}", provider_for_title.name()));
            }
        })
        .on_page_load(move |window, _payload| {
            let settings = settings_state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            let _ = window.eval(update_media_script(settings.speed, settings.muted, false));
        })
        .on_new_window(move |url, _features| {
            if matches!(url.scheme(), "http" | "https") {
                if let Some(window) = app_for_popup.get_webview_window(&popup_label) {
                    let _ = window.navigate(url);
                }
            }
            tauri::webview::NewWindowResponse::Deny
        })
        .build()
        .map_err(|error| format!("打开{}失败: {error}", provider.name()))?;
    Ok(window)
}

async fn capture_current(
    app: &AppHandle,
    state: &VideoTaskState,
    provider: Provider,
) -> Result<PageTopicCapture, String> {
    let window = app
        .get_webview_window(&provider.label())
        .ok_or_else(|| format!("请先打开并登录{}", provider.name()))?;
    let current_url = window.url().map_err(|error| error.to_string())?;
    if !provider_accepts_url(provider, &current_url) {
        return Err(format!(
            "当前页面不属于{}，请进入该平台的专题课程列表页",
            provider.name()
        ));
    }
    let mut last_error = "读取专题页面超时，请确认当前窗口停留在专题课程列表页".to_string();
    for attempt in 0..2 {
        let request_id = format!(
            "{}-{}",
            provider.key(),
            CAPTURE_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        {
            let mut exchange = state
                .captures
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            exchange.buffers.remove(&request_id);
            exchange.completed.remove(&request_id);
        }
        window
            .eval(capture_script(&request_id, provider))
            .map_err(|error| format!("读取专题页面失败: {error}"))?;
        let mut should_retry = false;
        for _ in 0..300 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let result = state
                .captures
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .completed
                .remove(&request_id);
            if let Some(result) = result {
                match result {
                    Ok(capture) => return Ok(capture),
                    Err(error) if attempt == 0 => {
                        last_error = error;
                        should_retry = true;
                        break;
                    }
                    Err(error) => return Err(format!("{error}；自动重试后仍未成功")),
                }
            }
        }
        state
            .captures
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .buffers
            .remove(&request_id);
        if attempt == 0 && !should_retry {
            last_error = "读取专题页面超时，请确认当前窗口停留在专题课程列表页".to_string();
        }
    }
    Err(format!("{last_error}；自动重试后仍未成功"))
}

fn normalize_kind(kind: &str) -> &'static str {
    match kind {
        "exam" => "exam",
        "slides" => "slides",
        "material" => "material",
        _ => "video",
    }
}

fn import_capture(
    state: &VideoTaskState,
    provider: Provider,
    capture: PageTopicCapture,
) -> Result<ImportSummary, String> {
    if capture.courses.is_empty() {
        return Err("当前页面没有识别到课程，请确认已进入专题课程列表页".to_string());
    }
    let topic_id = stable_id(&[provider.key(), &capture.url]);
    let timestamp = now();
    let mut conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    conn.execute("PRAGMA foreign_keys=ON", []).ok();
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    transaction
        .execute(
            "INSERT INTO video_topics(id,provider,title,url,progress,total_count,completed_count,last_synced_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(id) DO UPDATE SET
               title=excluded.title,url=excluded.url,progress=excluded.progress,
               total_count=excluded.total_count,completed_count=excluded.completed_count,
               last_synced_at=excluded.last_synced_at",
            params![
                topic_id,
                provider.key(),
                capture.title,
                capture.url,
                capture.progress,
                capture.total_count,
                capture.completed_count,
                timestamp
            ],
        )
        .map_err(|error| error.to_string())?;

    let mut manual = 0usize;
    let mut completed = 0usize;
    for (index, course) in capture.courses.iter().enumerate() {
        let kind = normalize_kind(&course.kind);
        let external_id = if course.external_id.trim().is_empty() {
            format!("{}-{index}", course.title)
        } else {
            course.external_id.clone()
        };
        let course_id = stable_id(&[&topic_id, &external_id]);
        let status = if course.completed {
            completed += 1;
            "completed"
        } else if kind == "video" {
            "pending"
        } else {
            manual += 1;
            "manual"
        };
        transaction
            .execute(
                "INSERT INTO video_courses(
                   id,topic_id,provider,external_id,title,url,locator,section_title,kind,
                   duration_seconds,progress,status,sort_order,last_error,updated_at
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,NULL,?14)
                 ON CONFLICT(topic_id,external_id) DO UPDATE SET
                   title=excluded.title,url=excluded.url,locator=excluded.locator,
                   section_title=excluded.section_title,kind=excluded.kind,
                   duration_seconds=excluded.duration_seconds,progress=excluded.progress,
                   status=CASE
                     WHEN excluded.status='completed' THEN 'completed'
                     WHEN video_courses.status IN('playing','verifying') THEN video_courses.status
                     ELSE excluded.status
                   END,
                   sort_order=excluded.sort_order,
                   last_error=CASE WHEN excluded.status='completed' THEN NULL ELSE video_courses.last_error END,
                   updated_at=excluded.updated_at",
                params![
                    course_id,
                    topic_id,
                    provider.key(),
                    external_id,
                    course.title,
                    course.url,
                    course.locator,
                    course.section_title,
                    kind,
                    course.duration_seconds,
                    course.progress,
                    status,
                    index as i64,
                    timestamp
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(ImportSummary {
        topic_id,
        topic_title: capture.title,
        imported: capture.courses.len(),
        completed,
        manual,
    })
}

fn load_course(path: &PathBuf, course_id: &str) -> Result<CourseRecord, String> {
    let conn = Connection::open(path).map_err(|error| error.to_string())?;
    conn.query_row(
        "SELECT id,topic_id,provider,url,locator,kind FROM video_courses WHERE id=?1",
        params![course_id],
        |row| {
            let provider: String = row.get(2)?;
            Ok(CourseRecord {
                id: row.get(0)?,
                topic_id: row.get(1)?,
                provider: Provider::parse(&provider).unwrap_or(Provider::Ulearn),
                url: row.get(3)?,
                locator: row.get(4)?,
                kind: row.get(5)?,
            })
        },
    )
    .map_err(|error| format!("读取课程失败: {error}"))
}

fn topic_url(path: &PathBuf, topic_id: &str) -> Result<String, String> {
    let conn = Connection::open(path).map_err(|error| error.to_string())?;
    conn.query_row(
        "SELECT url FROM video_topics WHERE id=?1",
        params![topic_id],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}

async fn open_course(
    app: &AppHandle,
    state: &VideoTaskState,
    course: &CourseRecord,
    auto_play: bool,
) -> Result<(), String> {
    let window = ensure_window(app, state, course.provider, true).await?;
    if !course.url.is_empty() {
        let url = course
            .url
            .parse::<tauri::Url>()
            .map_err(|error| format!("课程网址无效: {error}"))?;
        if !provider_accepts_url(course.provider, &url) {
            return Err("课程链接跳转到了非教学平台域名，已阻止自动打开".to_string());
        }
        window.navigate(url).map_err(|error| error.to_string())?;
    } else {
        let url = topic_url(state.db_path.as_ref(), &course.topic_id)?
            .parse::<tauri::Url>()
            .map_err(|error| error.to_string())?;
        window.navigate(url).map_err(|error| error.to_string())?;
        let selector = serde_json::to_string(&course.locator).map_err(|error| error.to_string())?;
        let click_window = window.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1800)).await;
            let _ = click_window.eval(format!(
                "(() => {{ const target=document.querySelector({selector}); if(target) target.click(); }})();"
            ));
        });
    }
    if auto_play {
        let settings = state
            .settings
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let play_window = window.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(2200)).await;
            let _ = play_window.eval(update_media_script(settings.speed, settings.muted, true));
        });
    }
    Ok(())
}

fn next_pending(
    path: &PathBuf,
    provider: Option<Provider>,
) -> Result<Option<CourseRecord>, String> {
    let conn = Connection::open(path).map_err(|error| error.to_string())?;
    let sql = if provider.is_some() {
        "SELECT id FROM video_courses WHERE status='pending' AND kind='video' AND provider=?1
         ORDER BY sort_order,updated_at LIMIT 1"
    } else {
        "SELECT id FROM video_courses WHERE status='pending' AND kind='video'
         ORDER BY updated_at,sort_order LIMIT 1"
    };
    let id: Option<String> = if let Some(provider) = provider {
        conn.query_row(sql, params![provider.key()], |row| row.get(0))
            .optional()
            .map_err(|error| error.to_string())?
    } else {
        conn.query_row(sql, [], |row| row.get(0))
            .optional()
            .map_err(|error| error.to_string())?
    };
    id.map(|id| load_course(path, &id)).transpose()
}

async fn start_one(
    app: &AppHandle,
    state: &VideoTaskState,
    provider: Option<Provider>,
) -> Result<bool, String> {
    let Some(course) = next_pending(state.db_path.as_ref(), provider)? else {
        return Ok(false);
    };
    if app.get_webview_window(&course.provider.label()).is_none() {
        ensure_window(app, state, course.provider, true).await?;
        return Ok(false);
    }
    open_course(app, state, &course, true).await?;
    let timestamp = now();
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    conn.execute(
        "UPDATE video_courses SET status='playing',last_error=NULL,updated_at=?2 WHERE id=?1",
        params![course.id, timestamp],
    )
    .map_err(|error| error.to_string())?;
    state
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .active
        .insert(
            course.provider.key().to_string(),
            ActiveCourse {
                course_id: course.id,
                topic_id: course.topic_id,
                provider: course.provider,
                phase: "playing".to_string(),
                phase_since: timestamp,
                last_media_at: timestamp,
            },
        );
    Ok(true)
}

async fn verify_active(
    app: &AppHandle,
    state: &VideoTaskState,
    active: ActiveCourse,
) -> Result<(), String> {
    let capture = capture_current(app, state, active.provider).await?;
    import_capture(state, active.provider, capture)?;
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    let status: String = conn
        .query_row(
            "SELECT status FROM video_courses WHERE id=?1",
            params![active.course_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if status != "completed" {
        conn.execute(
            "UPDATE video_courses SET status='attention',last_error='播放已结束，但平台尚未确认完成，请打开课程检查后重新同步' WHERE id=?1",
            params![active.course_id],
        )
        .map_err(|error| error.to_string())?;
    }
    state
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .active
        .remove(active.provider.key());
    Ok(())
}

#[tauri::command]
pub fn get_video_task_dashboard(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
) -> Result<VideoTaskDashboard, String> {
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    let mut topic_stmt = conn
        .prepare(
            "SELECT id,provider,title,url,progress,total_count,completed_count,last_synced_at
             FROM video_topics ORDER BY last_synced_at DESC",
        )
        .map_err(|error| error.to_string())?;
    let topic_rows = topic_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(topic_stmt);

    let mut topics = Vec::new();
    let mut stats = QueueStats::default();
    for (id, provider, title, url, progress, total_count, completed_count, last_synced_at) in
        topic_rows
    {
        let mut course_stmt = conn
            .prepare(
                "SELECT id,title,url,section_title,kind,duration_seconds,progress,status,last_error
                 FROM video_courses WHERE topic_id=?1 ORDER BY sort_order,title",
            )
            .map_err(|error| error.to_string())?;
        let courses = course_stmt
            .query_map(params![id], |row| {
                Ok(CourseItem {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    url: row.get(2)?,
                    section_title: row.get(3)?,
                    kind: row.get(4)?,
                    duration_seconds: row.get(5)?,
                    progress: row.get(6)?,
                    status: row.get(7)?,
                    last_error: row.get(8)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        for course in &courses {
            stats.total += 1;
            match course.status.as_str() {
                "completed" => stats.completed += 1,
                "pending" => stats.pending += 1,
                "playing" | "verifying" => stats.running += 1,
                "manual" => stats.manual += 1,
                "attention" => stats.attention += 1,
                _ => {}
            }
        }
        topics.push(TopicItem {
            id,
            provider,
            title,
            url,
            progress,
            total_count,
            completed_count,
            last_synced_at,
            courses,
        });
    }
    let sources = [Provider::Ulearn, Provider::Merchant]
        .into_iter()
        .map(|provider| {
            let window = app.get_webview_window(&provider.label());
            SourceStatus {
                provider: provider.key().to_string(),
                name: provider.name().to_string(),
                home_url: provider.home().to_string(),
                window_open: window.is_some(),
                current_url: window.and_then(|window| window.url().ok().map(|url| url.to_string())),
            }
        })
        .collect();
    let settings = state
        .settings
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    Ok(VideoTaskDashboard {
        settings,
        sources,
        topics,
        stats,
    })
}

#[tauri::command]
pub async fn open_video_learning_site(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    provider: String,
) -> Result<(), String> {
    ensure_window(&app, state.inner(), Provider::parse(&provider)?, true)
        .await
        .map(|_| ())
}

#[tauri::command]
pub async fn import_current_video_topic(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    provider: String,
) -> Result<ImportSummary, String> {
    let provider = Provider::parse(&provider)?;
    let capture = capture_current(&app, state.inner(), provider).await?;
    import_capture(state.inner(), provider, capture)
}

#[tauri::command]
pub async fn sync_video_topic(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    topic_id: String,
) -> Result<ImportSummary, String> {
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    let (provider, url): (String, String) = conn
        .query_row(
            "SELECT provider,url FROM video_topics WHERE id=?1",
            params![topic_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;
    drop(conn);
    let provider = Provider::parse(&provider)?;
    let window = ensure_window(&app, state.inner(), provider, false).await?;
    window
        .navigate(
            url.parse::<tauri::Url>()
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let capture = capture_current(&app, state.inner(), provider).await?;
    import_capture(state.inner(), provider, capture)
}

#[tauri::command]
pub fn update_video_task_settings(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    mut settings: VideoTaskSettings,
) -> Result<(), String> {
    settings.speed = clamp_speed(settings.speed);
    *state
        .settings
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = settings.clone();
    persist_settings(state.db_path.as_ref(), &settings)?;
    for provider in [Provider::Ulearn, Provider::Merchant] {
        if let Some(window) = app.get_webview_window(&provider.label()) {
            window
                .eval(update_media_script(settings.speed, settings.muted, false))
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn start_video_queue(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
) -> Result<(), String> {
    let mut settings = state
        .settings
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    settings.running = true;
    persist_settings(state.db_path.as_ref(), &settings)?;
    for provider in [Provider::Ulearn, Provider::Merchant] {
        if let Some(window) = app.get_webview_window(&provider.label()) {
            let _ = window.eval(update_media_script(settings.speed, settings.muted, true));
        }
    }
    Ok(())
}

#[tauri::command]
pub fn pause_video_queue(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
) -> Result<(), String> {
    let mut settings = state
        .settings
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    settings.running = false;
    persist_settings(state.db_path.as_ref(), &settings)?;
    for provider in [Provider::Ulearn, Provider::Merchant] {
        if let Some(window) = app.get_webview_window(&provider.label()) {
            let _ = window
                .eval("document.querySelectorAll('video,audio').forEach((media)=>media.pause());");
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn tick_video_queue(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
) -> Result<(), String> {
    let settings = state
        .settings
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    if !settings.running {
        return Ok(());
    }
    let active_list = state
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .active
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for active in active_list {
        if active.phase == "ended" {
            let url = topic_url(state.db_path.as_ref(), &active.topic_id)?;
            if let Some(window) = app.get_webview_window(&active.provider.label()) {
                window
                    .navigate(
                        url.parse::<tauri::Url>()
                            .map_err(|error| error.to_string())?,
                    )
                    .map_err(|error| error.to_string())?;
            }
            let conn =
                Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
            conn.execute(
                "UPDATE video_courses SET status='verifying' WHERE id=?1",
                params![active.course_id],
            )
            .map_err(|error| error.to_string())?;
            if let Some(item) = state
                .runtime
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .active
                .get_mut(active.provider.key())
            {
                item.phase = "verifying".to_string();
                item.phase_since = now();
            }
        } else if active.phase == "verifying" && now() - active.phase_since >= 3 {
            verify_active(&app, state.inner(), active).await?;
        } else if active.phase == "error"
            || (active.phase == "playing" && now() - active.last_media_at > 90)
        {
            let conn =
                Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
            conn.execute(
                "UPDATE video_courses SET status='attention',last_error='未检测到可持续播放的视频，请打开课程检查' WHERE id=?1",
                params![active.course_id],
            )
            .map_err(|error| error.to_string())?;
            state
                .runtime
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .active
                .remove(active.provider.key());
        }
    }
    let active_providers = state
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .active
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    if settings.cross_site_parallel {
        for provider in [Provider::Ulearn, Provider::Merchant] {
            if !active_providers.iter().any(|key| key == provider.key()) {
                let _ = start_one(&app, state.inner(), Some(provider)).await?;
            }
        }
    } else if active_providers.is_empty() {
        let _ = start_one(&app, state.inner(), None).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn open_video_course(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    course_id: String,
) -> Result<(), String> {
    let course = load_course(state.db_path.as_ref(), &course_id)?;
    open_course(&app, state.inner(), &course, false).await
}

#[tauri::command]
pub fn retry_video_course(
    state: tauri::State<'_, VideoTaskState>,
    course_id: String,
) -> Result<(), String> {
    let course = load_course(state.db_path.as_ref(), &course_id)?;
    let next_status = if course.kind == "video" {
        "pending"
    } else {
        "manual"
    };
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    conn.execute(
        "UPDATE video_courses SET status=?2,last_error=NULL,updated_at=?3 WHERE id=?1",
        params![course_id, next_status, now()],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn remove_video_topic(
    state: tauri::State<'_, VideoTaskState>,
    topic_id: String,
) -> Result<(), String> {
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    conn.execute("PRAGMA foreign_keys=ON", []).ok();
    conn.execute("DELETE FROM video_topics WHERE id=?1", params![topic_id])
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_speed_never_exceeds_platform_limit() {
        assert_eq!(clamp_speed(0.5), 1.0);
        assert_eq!(clamp_speed(1.5), 1.5);
        assert_eq!(clamp_speed(8.0), 2.0);
    }

    #[test]
    fn non_video_kinds_are_explicit() {
        assert_eq!(normalize_kind("exam"), "exam");
        assert_eq!(normalize_kind("slides"), "slides");
        assert_eq!(normalize_kind("material"), "material");
        assert_eq!(normalize_kind("unknown"), "video");
    }

    #[test]
    fn stable_ids_are_repeatable_and_scoped() {
        let one = stable_id(&["merchant", "topic", "course"]);
        assert_eq!(one, stable_id(&["merchant", "topic", "course"]));
        assert_ne!(one, stable_id(&["ulearn", "topic", "course"]));
    }

    #[test]
    fn capture_buffer_rejects_truncated_title_chunks() {
        let buffer = CaptureBuffer {
            total: 1,
            encoded_len: 8,
            chunks: vec![Some("e30=".to_string())],
        };
        let error = decode_capture_buffer(&buffer).expect_err("truncated capture must fail");
        assert!(error.contains("分块不完整"));
    }

    #[test]
    fn capture_buffer_decodes_complete_payload() {
        let json = r#"{"title":"专题","url":"https://example.com/topic","progress":0,"totalCount":0,"completedCount":0,"courses":[]}"#;
        let encoded = STANDARD.encode(json.as_bytes());
        let buffer = CaptureBuffer {
            total: 2,
            encoded_len: encoded.len(),
            chunks: vec![
                Some(encoded[..4].to_string()),
                Some(encoded[4..].to_string()),
            ],
        };
        let capture = decode_capture_buffer(&buffer).expect("complete capture decodes");
        assert_eq!(capture.title, "专题");
    }

    #[test]
    fn import_separates_video_exam_and_slides() {
        let path = std::env::temp_dir().join(format!(
            "mtool-video-task-test-{}.db",
            CAPTURE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        init_db(&path).expect("init test database");
        let state = VideoTaskState {
            db_path: Arc::new(path.clone()),
            settings: Arc::new(Mutex::new(VideoTaskSettings::default())),
            captures: Arc::new(Mutex::new(CaptureExchange::default())),
            runtime: Arc::new(Mutex::new(RuntimeState::default())),
        };
        let course = |id: &str, title: &str, kind: &str, completed: bool| PageCourseCapture {
            external_id: id.to_string(),
            title: title.to_string(),
            url: format!("https://ysstudy.lzdxedu.com/course/{id}"),
            locator: String::new(),
            section_title: "第一期".to_string(),
            kind: kind.to_string(),
            duration_seconds: 60,
            progress: if completed { 100.0 } else { 0.0 },
            completed,
        };
        let summary = import_capture(
            &state,
            Provider::Merchant,
            PageTopicCapture {
                title: "测试专题".to_string(),
                url: "https://ysstudy.lzdxedu.com/study/test".to_string(),
                progress: 33.3,
                total_count: 3,
                completed_count: 1,
                courses: vec![
                    course("video", "视频课程", "video", false),
                    course("exam", "课程考试", "exam", false),
                    course("slides", "课程课件", "slides", true),
                ],
            },
        )
        .expect("import capture");
        assert_eq!(summary.imported, 3);
        assert_eq!(summary.completed, 1);
        assert_eq!(summary.manual, 1);

        let conn = Connection::open(&path).expect("open test database");
        let video_status: String = conn
            .query_row(
                "SELECT status FROM video_courses WHERE kind='video'",
                [],
                |row| row.get(0),
            )
            .expect("video status");
        let exam_status: String = conn
            .query_row(
                "SELECT status FROM video_courses WHERE kind='exam'",
                [],
                |row| row.get(0),
            )
            .expect("exam status");
        let slide_status: String = conn
            .query_row(
                "SELECT status FROM video_courses WHERE kind='slides'",
                [],
                |row| row.get(0),
            )
            .expect("slides status");
        assert_eq!(video_status, "pending");
        assert_eq!(exam_status, "manual");
        assert_eq!(slide_status, "completed");
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}
