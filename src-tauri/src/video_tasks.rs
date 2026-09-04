use base64::{engine::general_purpose::STANDARD, Engine as _};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const ULEARN_HOME: &str = "https://ulearn.cup.com.cn/home";
const BRIDGE_CAPTURE_START_PREFIX: &str = "MTOOL_CAPTURE_START|";
const BRIDGE_CAPTURE_PREFIX: &str = "MTOOL_CAPTURE|";
const BRIDGE_MEDIA_PREFIX: &str = "MTOOL_MEDIA|";
const CAPTURE_CHUNK_SIZE: usize = 800;
const CAPTURE_CHUNK_INTERVAL_MS: u64 = 100;
const CAPTURE_POLL_INTERVAL_MS: u64 = 50;
const CAPTURE_START_TIMEOUT_MS: u64 = 8_000;
const CAPTURE_SCAN_TIMEOUT_MS: u64 = 30_000;
const CAPTURE_IDLE_TIMEOUT_MS: u64 = 5_000;
const CAPTURE_TOTAL_TIMEOUT_MS: u64 = 60_000;
static CAPTURE_COUNTER: AtomicU64 = AtomicU64::new(1);

fn decode_obfuscated_url(encoded: &str) -> String {
    STANDARD
        .decode(encoded)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default()
}

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
            Self::Merchant => "YS学堂",
        }
    }

    fn home(self) -> String {
        match self {
            Self::Ulearn => ULEARN_HOME.to_string(),
            // 运行时解码 "https://ys.../login"
            Self::Merchant => decode_obfuscated_url("aHR0cHM6Ly95c3N0dWR5Lmx6ZHhlZHUuY29tL2xvZ2lu"),
        }
    }

    fn player_label(self) -> String {
        format!("video-task-{}-player", self.key())
    }

    fn browser_label(self) -> String {
        format!("video-task-{}-browser", self.key())
    }

    fn label(self) -> String {
        self.player_label()
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
    active_requests: HashSet<String>,
    started_requests: HashSet<String>,
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
    last_progress_at: i64,
    last_advanced_time: f64,
    current_time: f64,
    duration: f64,
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
    title: String,
    duration_seconds: i64,
    progress: f64,
    sort_order: i64,
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
        Provider::Merchant => {
            let domain = decode_obfuscated_url("bHpkeGVkdS5jb20=");
            host == domain || host.ends_with(&format!(".{domain}"))
        }
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
         SET status='pending',last_error=NULL
         WHERE kind='video' AND status IN('opening','playing','verifying')",
        [],
    )
    .map_err(|error| format!("恢复未完成视频任务失败: {error}"))?;
    let _ = conn.execute(
        "DELETE FROM video_courses
         WHERE status NOT IN ('opening','playing','verifying')
           AND (
             title GLOB '[0-9][0-9]第*期*'
             OR title GLOB '[0-9]第*期*'
             OR title GLOB '第*期*'
             OR title GLOB '[0-9][0-9] 第*期*'
             OR title GLOB '[0-9] 第*期*'
             OR title GLOB '模块[0-9一二三四五六七八九十]*'
             OR title GLOB '阶段[0-9一二三四五六七八九十]*'
           )",
        [],
    );
    let _ = conn.execute(
        "UPDATE video_courses
         SET kind = 'video'
         WHERE kind = 'slides' AND (duration_seconds >= 120 OR title NOT LIKE '%课件%')",
        [],
    );
    let _ = conn.execute(
        "UPDATE video_courses
         SET status = 'pending'
         WHERE kind = 'video' AND status = 'manual'",
        [],
    );
    let _ = conn.execute(
        "UPDATE video_courses
         SET status = 'pending'
         WHERE status IN ('opening', 'playing', 'verifying')",
        [],
    );
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
  const homeUrl = "__HOME_URL__";
  const state = {
    speed: __SPEED__,
    muted: __MUTED__,
    autoPlay: false,
    tracked: new WeakSet(),
    pageLoadedAt: Date.now(),
  };

  const setTitleMessage = (message) => {
    document.title = message;
  };

  const report = (eventName, media) => {
    const cur = Number(media.currentTime) || 0;
    const dur = Number(media.duration) || 0;
    const message = "MTOOL_MEDIA|" + provider + "|" + eventName + "|" + cur + "|" + dur + "|" + Date.now();
    if (window.top === window) setTitleMessage(message);
    else {
      try { window.top.postMessage({ __mtoolMedia: message }, "*"); } catch (_) {}
      try { window.parent.postMessage({ __mtoolMedia: message }, "*"); } catch (_) {}
    }
  };

  if (window.top === window) {
    window.addEventListener("message", (event) => {
      if (event.data && event.data.__mtoolMedia) setTitleMessage(event.data.__mtoolMedia);
    });

    // 快捷键支持：Alt + ← 后退，Alt + → 前进
    window.addEventListener("keydown", (e) => {
      if ((e.altKey || e.metaKey) && e.key === "ArrowLeft") {
        e.preventDefault();
        window.history.back();
      } else if ((e.altKey || e.metaKey) && e.key === "ArrowRight") {
        e.preventDefault();
        window.history.forward();
      }
    });
  }

  const simulateFullClick = (el) => {
    if (!el) return;
    try {
      const rect = el.getBoundingClientRect();
      const x = rect.left + rect.width / 2;
      const y = rect.top + rect.height / 2;
      const eventInit = {
        bubbles: true,
        cancelable: true,
        view: window,
        clientX: x,
        clientY: y,
        screenX: x,
        screenY: y,
        button: 0,
        buttons: 1,
      };
      el.dispatchEvent(new PointerEvent("pointerdown", eventInit));
      el.dispatchEvent(new MouseEvent("mousedown", eventInit));
      el.dispatchEvent(new PointerEvent("pointerup", eventInit));
      el.dispatchEvent(new MouseEvent("mouseup", eventInit));
      el.dispatchEvent(new MouseEvent("click", eventInit));
      if (typeof el.click === "function") el.click();
    } catch (_) {
      try { el.click(); } catch (_) {}
    }
  };

  const track = (media) => {
    if (state.tracked.has(media)) return;
    state.tracked.add(media);
    ["play", "playing", "pause", "ended", "error", "canplay", "canplaythrough", "loadedmetadata", "durationchange"].forEach((name) => {
      media.addEventListener(name, () => report(name, media), true);
    });
    media.addEventListener("timeupdate", () => {
      if (!media.__mtoolLastReport || Date.now() - media.__mtoolLastReport > 600) {
        media.__mtoolLastReport = Date.now();
        report("timeupdate", media);
      }
      if (media.ended) {
        report("ended", media);
      }
    }, true);
    media.addEventListener("ratechange", () => {
      if (Math.abs(media.playbackRate - state.speed) > 0.05) {
        try { media.playbackRate = state.speed; } catch (_) {}
      }
    }, true);
  };

  const isMediaReallyAdvancing = (media) => {
    if (!media || media.paused || media.ended) return false;
    const nowTs = Date.now();
    if (media.__lastTime === undefined || Math.abs(media.currentTime - media.__lastTime) > 0.05) {
      media.__lastTime = media.currentTime;
      media.__lastTimeChangedAt = nowTs;
      return true;
    }
    if (nowTs - (media.__lastTimeChangedAt || nowTs) > 2500) {
      return false;
    }
    return true;
  };

  const isAnyMediaPlaying = (docs) => {
    for (const doc of docs) {
      const medias = doc.querySelectorAll("video, audio");
      for (const media of medias) {
        if (isMediaReallyAdvancing(media)) {
          return true;
        }
      }
    }
    return false;
  };

  const tryPlayMedia = (media) => {
    if (!media || media.ended) return;
    try {
      media.defaultPlaybackRate = state.speed;
      media.playbackRate = state.speed;
      media.muted = state.muted;
      media.defaultMuted = state.muted;
      media.setAttribute("playsinline", "true");
      media.setAttribute("webkit-playsinline", "true");
      media.setAttribute("autoplay", "true");
    } catch (_) {}

    try {
      media.muted = true;
      const res = media.play();
      if (res && res.then) {
        res.then(() => {
          if (!state.muted && !media.paused) {
            window.setTimeout(() => { if (!media.paused) media.muted = false; }, 300);
          }
        }).catch(() => {});
      }
    } catch (_) {}
  };

  const triggerPlayUI = (doc) => {
    if (!doc) return;

    // 1. 自动处理视频中间弹出的互动问答题（选择第一项并提交）
    try {
      const options = doc.querySelectorAll("input[type='radio'], input[type='checkbox'], [class*='quiz'] [class*='option'], [class*='question'] [class*='item'], [class*='answer-item']");
      if (options.length > 0) {
        for (const opt of options) {
          if (opt.offsetWidth > 0 || opt.offsetHeight > 0 || opt.getClientRects().length > 0) {
            simulateFullClick(opt);
            if (typeof opt.click === "function") opt.click();
            break;
          }
        }
      }
    } catch (_) {}

    // 2. 全覆盖识别防挂机、继续、确定、提交等弹窗按钮
    try {
      const allButtons = doc.querySelectorAll("button, a, [role='button'], input[type='button'], input[type='submit'], .ant-btn, .el-button, [class*='btn'], [class*='button']");
      allButtons.forEach((btn) => {
        if (btn.offsetWidth === 0 && btn.offsetHeight === 0 && btn.getClientRects().length === 0) return;
        // 排除底部的播放器控制条切换键
        if (btn.matches(".prism-play-btn, .vjs-play-control, [class*='play-btn'], [class*='playBtn'], [class*='volume']")) return;
        const text = (btn.innerText || btn.value || btn.title || "").replace(/\s+/g, "");
        if (/^(继续学习|继续播放|我知道了|确定|确认|知道了|继续|提交|完成|立即学习|开始学习|好的|交卷|下一步)$/.test(text)) {
          simulateFullClick(btn);
        }
      });
    } catch (_) {}

    // 3. 弹窗右上角关闭按钮
    try {
      const closeButtons = doc.querySelectorAll(".ant-modal-close, .el-dialog__headerbtn, .layui-layer-close, [class*='dialog'] [class*='close'], [class*='modal'] [class*='close'], [aria-label='Close']");
      closeButtons.forEach((btn) => {
        if (btn.offsetWidth > 0 || btn.offsetHeight > 0 || btn.getClientRects().length > 0) {
          simulateFullClick(btn);
        }
      });
    } catch (_) {}

    // 4. 居中大播放按钮
    const bigPlaySelectors = [
      ".prism-big-play-btn", ".vjs-big-play-button", ".pv-big-play-btn",
      ".xgplayer-start", ".tcplayer-center-play", "[class*='big-play']",
      "[class*='center-play']", "[class*='play-mask']", "[class*='player-mask']"
    ];
    try {
      doc.querySelectorAll(bigPlaySelectors.join(",")).forEach((btn) => {
        if (btn.offsetWidth > 0 || btn.offsetHeight > 0 || btn.getClientRects().length > 0) {
          simulateFullClick(btn);
        }
      });
    } catch (_) {}

    // 5. 播放器全局 API
    try {
      if (window.player && typeof window.player.play === "function") window.player.play();
      if (window.aliplayer && typeof window.aliplayer.play === "function") window.aliplayer.play();
      if (window.videoPlayer && typeof window.videoPlayer.play === "function") window.videoPlayer.play();
    } catch (_) {}
  };

  const getAccessibleDocs = () => {
    const docs = [document];
    try {
      document.querySelectorAll("iframe").forEach((frame) => {
        try {
          if (frame.contentDocument && !docs.includes(frame.contentDocument)) {
            docs.push(frame.contentDocument);
          }
        } catch (_) {}
      });
    } catch (_) {}
    return docs;
  };

  const injectNavToolbar = () => {
    if (window.top !== window || document.getElementById("__mtool_nav_toolbar__")) return;
    const bar = document.createElement("div");
    bar.id = "__mtool_nav_toolbar__";
    bar.setAttribute("style", `
      position: fixed;
      bottom: 24px;
      left: 24px;
      z-index: 2147483647;
      display: flex;
      align-items: center;
      gap: 3px;
      padding: 4px 6px;
      background: rgba(15, 23, 42, 0.88);
      backdrop-filter: blur(16px);
      -webkit-backdrop-filter: blur(16px);
      border: 1px solid rgba(255, 255, 255, 0.18);
      border-radius: 9999px;
      box-shadow: 0 8px 24px rgba(0, 0, 0, 0.38);
      color: #f8fafc;
      font-size: 13px;
      user-select: none;
      -webkit-user-select: none;
      transition: opacity 0.2s;
    `);

    const createBtn = (title, svgPath, onClick) => {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.title = title;
      btn.setAttribute("style", `
        display: flex;
        align-items: center;
        justify-content: center;
        width: 28px;
        height: 28px;
        border: none;
        background: transparent;
        color: #e2e8f0;
        border-radius: 50%;
        cursor: pointer;
        outline: none;
        padding: 0;
        transition: background 0.15s, color 0.15s, transform 0.1s;
      `);
      btn.innerHTML = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">${svgPath}</svg>`;
      btn.onmouseenter = () => { btn.style.background = "rgba(255,255,255,0.18)"; btn.style.color = "#ffffff"; };
      btn.onmouseleave = () => { btn.style.background = "transparent"; btn.style.color = "#e2e8f0"; };
      btn.onmousedown = () => { btn.style.transform = "scale(0.92)"; };
      btn.onmouseup = () => { btn.style.transform = "scale(1)"; };
      btn.onclick = (e) => { e.preventDefault(); e.stopPropagation(); onClick(); };
      return btn;
    };

    // 拖拽手柄
    const handle = document.createElement("div");
    handle.title = "按住可拖动位置";
    handle.setAttribute("style", `
      cursor: grab;
      padding: 0 4px;
      display: flex;
      align-items: center;
      color: #94a3b8;
    `);
    handle.innerHTML = `<svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor"><circle cx="9" cy="6" r="2"/><circle cx="15" cy="6" r="2"/><circle cx="9" cy="12" r="2"/><circle cx="15" cy="12" r="2"/><circle cx="9" cy="18" r="2"/><circle cx="15" cy="18" r="2"/></svg>`;

    let isDragging = false;
    let startX = 0, startY = 0, initialLeft = 0, initialTop = 0;

    handle.onmousedown = (e) => {
      isDragging = true;
      handle.style.cursor = "grabbing";
      const rect = bar.getBoundingClientRect();
      startX = e.clientX;
      startY = e.clientY;
      initialLeft = rect.left;
      initialTop = rect.top;
      bar.style.bottom = "auto";
      bar.style.right = "auto";
      bar.style.left = initialLeft + "px";
      bar.style.top = initialTop + "px";
      e.preventDefault();
    };

    window.addEventListener("mousemove", (e) => {
      if (!isDragging) return;
      const dx = e.clientX - startX;
      const dy = e.clientY - startY;
      bar.style.left = Math.max(8, Math.min(window.innerWidth - bar.offsetWidth - 8, initialLeft + dx)) + "px";
      bar.style.top = Math.max(8, Math.min(window.innerHeight - bar.offsetHeight - 8, initialTop + dy)) + "px";
    });

    window.addEventListener("mouseup", () => {
      if (isDragging) {
        isDragging = false;
        handle.style.cursor = "grab";
      }
    });

    // 后退 (Chevron Left)
    const backBtn = createBtn("后退 (Alt+←)", '<path d="m15 18-6-6 6-6"/>', () => window.history.back());
    // 前进 (Chevron Right)
    const forwardBtn = createBtn("前进 (Alt+→)", '<path d="m9 18 6-6-6-6"/>', () => window.history.forward());
    // 刷新
    const refreshBtn = createBtn("刷新页面", '<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>', () => window.location.reload());
    // 首页
    const homeBtn = createBtn("返回平台首页", '<path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><polyline points="9 22 9 12 15 12 15 22"/>', () => { window.location.href = homeUrl; });

    bar.appendChild(handle);
    bar.appendChild(backBtn);
    bar.appendChild(forwardBtn);
    bar.appendChild(refreshBtn);
    bar.appendChild(homeBtn);

    const mount = () => {
      if (document.body && !document.getElementById("__mtool_nav_toolbar__")) {
        document.body.appendChild(bar);
      }
    };
    if (document.body) mount();
    else document.addEventListener("DOMContentLoaded", mount, { once: true });
  };

  const apply = (autoPlay) => {
    injectNavToolbar();
    const shouldPlay = Boolean(autoPlay || state.autoPlay);
    const docs = getAccessibleDocs();

    // 0. 实时检测是否处于登录/SSO/扫码页面
    try {
      const href = (window.location.href || "").toLowerCase();
      const bodyText = (document.body ? document.body.innerText || "" : "").slice(0, 1500);
      const isLoginUrl = href.includes("/login") || href.includes("/sso") || href.includes("/cas/") || href.includes("oauth") || href.includes("auth.");
      const isLoginText = bodyText.includes("扫码登录") || bodyText.includes("cu 扫码登录") || bodyText.includes("账号登录") || bodyText.includes("密码登录") || bodyText.includes("请先登录") || bodyText.includes("统一身份认证");
      if (isLoginUrl || isLoginText) {
        report("need_login", { currentTime: 0, duration: 0 });
        return;
      }
    } catch (_) {}

    // 0.1 真正的模态弹窗完播检测（必须是居中弹出可见对话框，严禁检测页面全局背景文本或导航栏标签）
    const hasVisibleCompletionModal = () => {
      for (const doc of docs) {
        try {
          const dialogs = doc.querySelectorAll("[role='dialog'], .el-dialog, .modal, .ant-modal, .van-dialog, [class*='dialog'], [class*='modal']");
          for (const d of dialogs) {
            if (!d || d.offsetWidth === 0 || d.offsetHeight === 0) continue;
            const t = (d.innerText || "").replace(/\s+/g, "");
            if (
              t.includes("恭喜您已完成") ||
              t.includes("您已完成当前资源的学习") ||
              t.includes("已完成当前资源的学习") ||
              t.includes("当前资源学习完成") ||
              t.includes("恭喜完成学习") ||
              t.includes("已达到学时要求") ||
              t.includes("已获得该课程学分") ||
              t.includes("已完成课件学习")
            ) {
              return true;
            }
          }
        } catch (_) {}
      }
      return false;
    };

    // 0.2 检测课程计划是否已结束/过期（已结束的课程无法继续学习，直接标记为完成）
    const hasExpiredOrEndedNotice = () => {
      for (const doc of docs) {
        try {
          const bodyText = (doc.body ? doc.body.innerText : "") || "";
          const alerts = doc.querySelectorAll(".el-message, .ant-message, [role='alert'], [class*='message'], [class*='toast'], [class*='notice'], [class*='tip'], [class*='alert']");
          for (const a of alerts) {
            if (!a || a.offsetWidth === 0 || a.offsetHeight === 0) continue;
            const t = (a.innerText || "").replace(/\s+/g, "");
            if (
              t.includes("计划已结束") ||
              t.includes("培训已结束") ||
              t.includes("活动已结束") ||
              t.includes("学习已结束") ||
              t.includes("项目已结束") ||
              t.includes("计划已关闭") ||
              t.includes("已超过学习截止时间") ||
              t.includes("已过学习截止时间") ||
              t.includes("课程已下架") ||
              t.includes("报名已结束")
            ) {
              return true;
            }
          }
          const m = bodyText.match(/起止时间\s*[:：]?\s*\d{4}[-/.]\d{1,2}[-/.]\d{1,2}.*?[~至到-]\s*(\d{4}[-/.]\d{1,2}[-/.]\d{1,2}(?:\s+\d{1,2}:\d{1,2}(?::\d{1,2})?)?)/);
          if (m) {
            const endTs = new Date(m[1].replace(/-/g, "/")).getTime();
            if (endTs && !isNaN(endTs) && endTs < Date.now()) {
              return true;
            }
          }
        } catch (_) {}
      }
      return false;
    };

    if (hasExpiredOrEndedNotice()) {
      report("ended", { currentTime: 100, duration: 100 });
      return;
    }

    // 1. 维持倍速、事件跟踪，并主动上报当前播放进度
    const allMedias = [];
    docs.forEach((doc) => {
      doc.querySelectorAll("video, audio").forEach((media) => {
        allMedias.push(media);
        track(media);
        if (Math.abs(media.playbackRate - state.speed) > 0.05) {
          try { media.defaultPlaybackRate = state.speed; media.playbackRate = state.speed; } catch (_) {}
        }
        if (media.muted !== state.muted && !media.paused) {
          try { media.muted = state.muted; } catch (_) {}
        }
        if (media.duration > 0 && !media.paused) {
          report("timeupdate", media);
        }
      });
    });

    // 1.1 若页面中有原生视频/音频：以视频自身的实际播放进度为主！
    if (allMedias.length > 0) {
      // 仅当弹出明确的模态完成对话框时，才上报完播；绝不在常规播放中提前截断
      if (hasVisibleCompletionModal()) {
        report("ended", { currentTime: 100, duration: 100 });
        return;
      }
    } else {
      // 1.2 若页面中未找到原生 video/audio 标签（如 PPT/课件/文档/阅读型课件）：
      // 必须严格在页面真实驻留满 60 秒以上！绝不允许 1~2 秒就判定完成！
      const staySeconds = Math.floor((Date.now() - state.pageLoadedAt) / 1000);
      const targetDuration = 60;
      report("timeupdate", { currentTime: Math.min(targetDuration, staySeconds), duration: targetDuration });
      // 驻留满 60 秒以上，如果弹出了完成弹窗，或者驻留超过 75 秒，正常上报完成
      if (staySeconds >= 60 && hasVisibleCompletionModal()) {
        report("ended", { currentTime: targetDuration, duration: targetDuration });
        return;
      }
      if (staySeconds >= 75) {
        report("ended", { currentTime: targetDuration, duration: targetDuration });
        return;
      }
    }

    if (!shouldPlay) return;

    // 2. 如果已经有视频在正常播放中，绝不要触发任何点击，避免把正在播放的视频点暂停！
    if (isAnyMediaPlaying(docs)) {
      return;
    }

    // 3. 所有视频都处于暂停状态时，先尝试原生 play()
    docs.forEach((doc) => {
      doc.querySelectorAll("video, audio").forEach((media) => {
        if (media.paused && !media.ended) {
          tryPlayMedia(media);
        }
      });
    });

    // 4. 若依然处于暂停，尝试触发大播放按钮与弹窗
    if (!isAnyMediaPlaying(docs)) {
      docs.forEach((doc) => {
        triggerPlayUI(doc);
      });
    }
  };

  state.update = (speedValue, mutedValue, autoPlay) => {
    state.speed = Math.min(2, Math.max(1, Number(speedValue) || 2));
    state.muted = Boolean(mutedValue);
    if (autoPlay !== undefined) state.autoPlay = Boolean(autoPlay);
    apply(Boolean(autoPlay));
  };

  Object.defineProperty(window, "__MTOOL_LEARNING_BRIDGE__", { value: state });
  const start = () => {
    apply(true);
    window.setInterval(() => apply(false), 1500);
  };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start, { once: true });
  else start();
})();
"##;
    TEMPLATE
        .replace("__PROVIDER__", provider.key())
        .replace("__HOME_URL__", &provider.home())
        .replace("__SPEED__", &clamp_speed(speed).to_string())
        .replace("__MUTED__", if muted { "true" } else { "false" })
}

fn browser_nav_script(provider: Provider) -> String {
    const TEMPLATE: &str = r##"
(() => {
  // 1. 全局媒体静音暂停（在所有 frame / iframe 均生效）
  // 选专题窗口专门用于浏览目录和导入专题，防止视频自动出声并避免抢占学习
  try {
    if (!window.__MTOOL_BROWSER_NAV_SHIELD__) {
      window.__MTOOL_BROWSER_NAV_SHIELD__ = true;

      // 使用标准的 play 事件捕获监听，自动静音并暂停，绝不修改页面任何 DOM 结构与样式
      window.addEventListener("play", (e) => {
        try {
          const media = e.target;
          if (media && typeof media.pause === "function") {
            media.muted = true;
            media.pause();
          }
        } catch (_) {}
      }, true);

      const ensurePaused = () => {
        try {
          document.querySelectorAll("video, audio").forEach((m) => {
            if (!m.paused) {
              m.muted = true;
              m.pause();
            }
          });
        } catch (_) {}
      };

      if (document.body) ensurePaused();
      else document.addEventListener("DOMContentLoaded", ensurePaused, { once: true });

      const timer = setInterval(ensurePaused, 1000);
      setTimeout(() => clearInterval(timer), 15000);
    }
  } catch (_) {}

  // 2. 仅在顶层窗口（Top Frame）挂载导航工具栏与快捷键
  if (window.top !== window || document.getElementById("__mtool_nav_toolbar__")) return;
  const homeUrl = "__HOME_URL__";

  // 快捷键支持：Alt + ← 后退，Alt + → 前进
  window.addEventListener("keydown", (e) => {
    if ((e.altKey || e.metaKey) && e.key === "ArrowLeft") {
      e.preventDefault();
      window.history.back();
    } else if ((e.altKey || e.metaKey) && e.key === "ArrowRight") {
      e.preventDefault();
      window.history.forward();
    }
  });

  const bar = document.createElement("div");
  bar.id = "__mtool_nav_toolbar__";
  bar.setAttribute("style", `
    position: fixed;
    bottom: 24px;
    left: 24px;
    z-index: 2147483647;
    display: flex;
    align-items: center;
    gap: 3px;
    padding: 4px 6px;
    background: rgba(15, 23, 42, 0.88);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    border: 1px solid rgba(255, 255, 255, 0.18);
    border-radius: 9999px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.38);
    color: #f8fafc;
    font-size: 13px;
    user-select: none;
    -webkit-user-select: none;
    transition: opacity 0.2s;
  `);

  const createBtn = (title, svgPath, onClick) => {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.title = title;
    btn.setAttribute("style", `
      display: flex;
      align-items: center;
      justify-content: center;
      width: 28px;
      height: 28px;
      border: none;
      background: transparent;
      color: #e2e8f0;
      border-radius: 50%;
      cursor: pointer;
      outline: none;
      padding: 0;
      transition: background 0.15s, color 0.15s, transform 0.1s;
    `);
    btn.innerHTML = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">${svgPath}</svg>`;
    btn.onmouseenter = () => { btn.style.background = "rgba(255,255,255,0.18)"; btn.style.color = "#ffffff"; };
    btn.onmouseleave = () => { btn.style.background = "transparent"; btn.style.color = "#e2e8f0"; };
    btn.onmousedown = () => { btn.style.transform = "scale(0.92)"; };
    btn.onmouseup = () => { btn.style.transform = "scale(1)"; };
    btn.onclick = (e) => { e.preventDefault(); e.stopPropagation(); onClick(); };
    return btn;
  };

  const handle = document.createElement("div");
  handle.title = "按住拖动工具条";
  handle.setAttribute("style", `
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 28px;
    cursor: grab;
    color: #94a3b8;
    padding-left: 2px;
  `);
  handle.innerHTML = `<svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor"><circle cx="8" cy="6" r="2"/><circle cx="16" cy="6" r="2"/><circle cx="8" cy="12" r="2"/><circle cx="16" cy="12" r="2"/><circle cx="8" cy="18" r="2"/><circle cx="16" cy="18" r="2"/></svg>`;

  let isDragging = false;
  let startX = 0, startY = 0, initialLeft = 0, initialTop = 0;
  handle.onmousedown = (e) => {
    isDragging = true;
    handle.style.cursor = "grabbing";
    const rect = bar.getBoundingClientRect();
    startX = e.clientX;
    startY = e.clientY;
    initialLeft = rect.left;
    initialTop = rect.top;
    bar.style.bottom = "auto";
    bar.style.right = "auto";
    bar.style.left = initialLeft + "px";
    bar.style.top = initialTop + "px";
    e.preventDefault();
  };

  window.addEventListener("mousemove", (e) => {
    if (!isDragging) return;
    const dx = e.clientX - startX;
    const dy = e.clientY - startY;
    bar.style.left = Math.max(8, Math.min(window.innerWidth - bar.offsetWidth - 8, initialLeft + dx)) + "px";
    bar.style.top = Math.max(8, Math.min(window.innerHeight - bar.offsetHeight - 8, initialTop + dy)) + "px";
  });

  window.addEventListener("mouseup", () => {
    if (isDragging) {
      isDragging = false;
      handle.style.cursor = "grab";
    }
  });

  const backBtn = createBtn("后退 (Alt+←)", '<path d="m15 18-6-6 6-6"/>', () => window.history.back());
  const forwardBtn = createBtn("前进 (Alt+→)", '<path d="m9 18 6-6-6-6"/>', () => window.history.forward());
  const refreshBtn = createBtn("刷新页面", '<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>', () => window.location.reload());
  const homeBtn = createBtn("返回平台首页", '<path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><polyline points="9 22 9 12 15 12 15 22"/>', () => { window.location.href = homeUrl; });

  bar.appendChild(handle);
  bar.appendChild(backBtn);
  bar.appendChild(forwardBtn);
  bar.appendChild(refreshBtn);
  bar.appendChild(homeBtn);

  const mount = () => {
    if (document.body && !document.getElementById("__mtool_nav_toolbar__")) {
      document.body.appendChild(bar);
    }
  };
  if (document.body) mount();
  else document.addEventListener("DOMContentLoaded", mount, { once: true });
})();
"##;
    TEMPLATE.replace("__HOME_URL__", &provider.home())
}

fn update_media_script(speed: f64, muted: bool, auto_play: bool) -> String {
    format!(
        r#"(() => {{
          const speed = {};
          const muted = {};
          const autoPlay = {};
          if (window.__MTOOL_LEARNING_BRIDGE__) {{
            window.__MTOOL_LEARNING_BRIDGE__.update(speed, muted, autoPlay);
            return;
          }}
          const docs = [document];
          try {{
            document.querySelectorAll("iframe").forEach((frame) => {{
              try {{ if (frame.contentDocument) docs.push(frame.contentDocument); }} catch (_) {{}}
            }});
          }} catch (_) {{}}
          docs.forEach((doc) => {{
            doc.querySelectorAll("video,audio").forEach((media) => {{
              try {{
                media.defaultPlaybackRate = speed;
                media.playbackRate = speed;
                media.muted = muted;
                if (autoPlay && media.paused && !media.ended) {{
                  media.muted = true;
                  media.play().catch(() => {{}});
                }}
              }} catch (_) {{}}
            }});
          }});
        }})();"#,
        clamp_speed(speed),
        if muted { "true" } else { "false" },
        if auto_play { "true" } else { "false" }
    )
}

fn course_click_script(title: &str, locator: &str) -> String {
    let title_json = serde_json::to_string(title).unwrap_or_default();
    let locator_json = serde_json::to_string(locator).unwrap_or_default();
    format!(
        r#"(() => {{
          const targetTitle = {title_json};
          const targetLocator = {locator_json};
          const clean = (value) => String(value || "").replace(/\s+/g, " ").trim();

          // 拦截 window.open 防止弹空白页
          try {{
            window.open = (url) => {{
              const next = clean(url);
              if (next && next !== "about:blank") {{
                try {{ window.location.assign(new URL(next, window.location.href).href); }} catch (_) {{}}
              }}
              return window;
            }};
          }} catch (_) {{}}

          const byLocator = targetLocator ? document.querySelector(targetLocator) : null;
          const all = Array.from(document.querySelectorAll("body *"));
          const byTitle = all.find((el) => clean(el.innerText) === clean(targetTitle)) ||
            all.find((el) => {{
              const text = clean(el.innerText);
              return text.length <= clean(targetTitle).length + 10 && text.includes(clean(targetTitle));
            }});
          let target = byLocator || byTitle;
          if (!target) return;

          let card = target;
          for (let current = target, depth = 0; current && current !== document.body && depth < 8; current = current.parentElement, depth++) {{
            if (current.matches("a[href], [class*='card'], [class*='item'], [class*='course'], [class*='list-item'], [class*='row'], tr, li")) {{
              card = current;
              break;
            }}
          }}

          try {{ card.scrollIntoView({{ block: "center", behavior: "instant" }}); }} catch (_) {{}}

          const actionBtn = Array.from(card.querySelectorAll("button, a, [role='button'], div, span")).find((el) => {{
            const t = clean(el.innerText);
            return /^(去学习|开始学习|继续学习|立即学习|学习中|进入学习|播放)$/.test(t) ||
                   el.matches("[class*='btn-primary'], [class*='study-btn'], [class*='play-btn'], [class*='start']");
          }});

          const anchor = card.matches("a[href]") ? card : card.querySelector("a[href]");
          let clickTarget = actionBtn || anchor || (card.matches("button, [role='button']") ? card : null) || target;

          if (clickTarget.tagName === "A" && clickTarget.getAttribute("href") && !/^javascript:/i.test(clickTarget.getAttribute("href"))) {{
            clickTarget.removeAttribute("target");
            try {{
              window.location.assign(new URL(clickTarget.getAttribute("href"), window.location.href).href);
              return;
            }} catch (_) {{}}
          }}

          clickTarget.querySelectorAll?.("a[target]").forEach((a) => a.removeAttribute("target"));
          if (clickTarget.matches?.("a[target]")) clickTarget.removeAttribute("target");
          try {{
            const rect = clickTarget.getBoundingClientRect();
            const init = {{
              bubbles: true,
              cancelable: true,
              view: window,
              clientX: rect.left + rect.width / 2,
              clientY: rect.top + rect.height / 2,
              button: 0,
            }};
            clickTarget.dispatchEvent(new PointerEvent("pointerdown", init));
            clickTarget.dispatchEvent(new MouseEvent("mousedown", init));
            clickTarget.dispatchEvent(new PointerEvent("pointerup", init));
            clickTarget.dispatchEvent(new MouseEvent("mouseup", init));
            clickTarget.dispatchEvent(new MouseEvent("click", init));
            if (typeof clickTarget.click === "function") clickTarget.click();
          }} catch (_) {{
            try {{ clickTarget.click(); }} catch (_) {{}}
          }}

          window.setTimeout(() => {{
            const alerts = Array.from(document.querySelectorAll(".el-message, .ant-message, [role='alert'], [class*='message'], [class*='toast'], [class*='notice'], [class*='tip']"));
            for (const a of alerts) {{
              const t = (a.innerText || "").replace(/\s+/g, "");
              if (t.includes("计划已结束") || t.includes("培训已结束") || t.includes("活动已结束") || t.includes("已超过学习截止时间")) {{
                const message = "MTOOL_MEDIA|merchant|ended|100|100|" + Date.now();
                if (window.top === window) document.title = message;
                else {{
                  try {{ window.top.postMessage({{ __mtoolMedia: message }}, "*"); }} catch (_) {{}}
                }}
                break;
              }}
            }}
          }}, 800);
        }})();"#
    )
}

fn capture_script(request_id: &str, provider: Provider) -> String {
    const TEMPLATE: &str = r##"
(() => {
  const requestId = "__REQUEST_ID__";
  const originalTitle = document.title;
  window.__MTOOL_CAPTURE_REQUEST__ = requestId;
  document.title = "MTOOL_CAPTURE_START|" + requestId;
  window.setTimeout(() => {
    try {
      const provider = "__PROVIDER__";
    const clean = (value) => String(value || "").replace(/\s+/g, " ").trim();
    const ownText = (element) => clean(Array.from(element.childNodes || [])
      .filter((node) => node.nodeType === Node.TEXT_NODE).map((node) => node.textContent).join(" "));
    const visible = (element) => {
      if (!element) return false;
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
    const countMatches = (str, regex) => (String(str || "").match(regex) || []).length;
    const isTagOrBadge = (s) => /^(知识|课程|考试|测验|测试|课件|文档|阅读材料|参考资料|视频|音频|图文|直播|ppt|pptx|pdf|word|excel|线下课|线上课|面授|面授课|公开课|问卷|调查问卷|评价表|满意度评价|调研|签到|打卡|活动|讨论|实操|练习|作业|大纲|目录|必修|选修|必修课|选修课|必修学分|选修学分|已完成|已学完|已学习|未学习|学习中|已考试|已通过|未通过|进行中|全部|展开|收起|去学习|立即学习|开始学习|重新学习|继续学习|查看|详情|上次学习|试看|播放中|\d{1,2})$/i.test(clean(s));
    const isMeta = (s) => /(学习时长|必修学分|选修学分|进度\s*[:：]?|学时\s*[:：]?\s*\d+|学分\s*[:：]?\s*\d+|起止时间|得分|正确率|总分|题数|时长\s*[:：]|考试时长|课程数|浏览人数|学习人数)/i.test(s);
    const isSiteOrUiTitle = (s) => /^(YS学堂|银商学堂|银联乐学|中国银联|乐学|首页|个人中心|学习中心|学习地图|考试中心|赛事中心|全部|培训管理|培训介绍|培训内容|专题介绍|课程大纲|乐学圈|我的学习|我的课程|课程详情|专题详情|全部课程|培训项目|学习任务|登录|加入自学|已加入)$/i.test(s);

    const detectCourseKind = (title, text, durationSeconds, container) => {
      const cleanTitle = String(title || "").trim();
      const cleanText = String(text || "").trim();

      // 1. 【全局与 DOM 播放器检测 - 最高优先级】
      // 若当前页面存在视频播放器（<video> 或 .prism-player），或当前条目挂载了视频元素/播放图标
      if (typeof document !== "undefined" && document.querySelector && document.querySelector("video, .prism-player, [class*='player']")) {
        const isExamInPlayer = /(期末考试|结业考试|随堂测验|模拟考试|在线考试|阶段测验|课后测验|综合测试|结业测试|试卷)|^.*(考试|测验)$/.test(cleanTitle) ||
          /(^|\s)(考试|测验|试卷)(\s|$)/.test(cleanText);
        if (isExamInPlayer) return "exam";
        return "video";
      }

      if (container && container.querySelector) {
        if (container.querySelector("video, audio") || (container.matches && container.matches("video, audio"))) {
          return "video";
        }
        const hasVideoFeature = container.querySelector(
          "[class*='video'], [class*='player'], [class*='play-btn'], [class*='play_btn'], [class*='play-icon'], [class*='play_icon'], [data-type*='video'], svg[class*='play'], i[class*='play'], [class*='icon-play']"
        );
        if (hasVideoFeature) {
          return "video";
        }
      }

      // 2. 考试与测验检测
      const isTechTesting = /(软件测试|压力测试|接口测试|性能测试|自动化测试|测试用例|测试开发|单元测试|测试方法|测试体系|测试流程|测试实战|测试理论)/.test(cleanTitle);

      const hasExplicitExamBadge = /(^|\s)(考试|测验|试卷)(\s|$)/.test(cleanText);
      const hasExamKeywordInTitle = /(期末考试|结业考试|随堂测验|模拟考试|在线考试|阶段测验|课后测验|综合测试|结业测试|试卷)|^.*(考试|测验)$/.test(cleanTitle) ||
        (!isTechTesting && /(考试|测验)/.test(cleanTitle));

      if (hasExplicitExamBadge || hasExamKeywordInTitle) {
        return "exam";
      }

      // 问卷/评价：仅匹配明确的问卷词汇，严禁单独匹配“调研”或“调查”误伤普通课题（如“市场调研”、“社会调查”）
      const hasSurveyBadge = /(^|\s)(问卷|调查问卷|调研问卷|评价表|满意度评价)(\s|$)/.test(cleanText);
      const hasSurveyTitle = /(问卷|调查问卷|调研问卷|满意度调查|评价表|满意度评价|课后评价|教学评价)/.test(cleanTitle);

      if (hasSurveyBadge || hasSurveyTitle) {
        return "exam";
      }

      // 3. 【真实时长特征保护】
      // 凡是具有明确时长（>= 120 秒，即 >= 2 分钟）的课程，绝非静态 PPT，直接判定为视频！
      const hasRealisticDuration = Number(durationSeconds) >= 120;
      if (hasRealisticDuration) {
        return "video";
      }

      // 4. 精确收窄 PPT/课件判定
      // 严禁包含式全词匹配 /ppt/i！《做PPT》、《学PPT》、《职场PPT排版》、《豆包生成PPT》本质全部是视频课！
      // 仅当标题以明确课件为后缀（如：xxx培训-课件、xxx【课件】、xxx_PPT）或 DOM 中有明确独立的 PPT/课件徽章时才判定为 slides
      const hasExplicitSlidesBadge = /(^|\s)(ppt课件|课件|幻灯片)(\s|$)/i.test(cleanText) ||
        (container && container.querySelectorAll &&
         Array.from(container.querySelectorAll("[class*='tag'], [class*='badge']")).some((el) => /^(课件|ppt课件|幻灯片)$/i.test(clean(el.innerText))));

      const hasSlidesSuffixInTitle = /[-_（(【\[\s](课件|幻灯片|ppt课件)[)）\]\s]?$/i.test(cleanTitle) ||
        /^.*[-_]课件$/i.test(cleanTitle);

      if (hasExplicitSlidesBadge || hasSlidesSuffixInTitle) {
        return "slides";
      }

      // 5. 资料/文档检测
      const hasMaterialBadge = /(^|\s)(阅读材料|参考资料|手册)(\s|$)/.test(cleanText);
      const hasMaterialSuffixInTitle = /[-_（(【\[\s](文档|阅读材料|参考资料|资料|pdf|手册)[)）\]\s]?$/i.test(cleanTitle);
      if (hasMaterialBadge || hasMaterialSuffixInTitle) {
        return "material";
      }

      // 6. 线下课检测
      if (/线下课|面授|签到|打卡/.test(cleanTitle) || /(^|\s)(线下课|面授)(\s|$)/.test(cleanText)) {
        return "material";
      }

      return "video";
    };

    const isElementCompleted = (element) => {
      if (!element) return false;
      const row = (element.closest && element.closest("li, tr, [class*='item'], [class*='chapter'], [class*='section'], [class*='node'], [class*='row']")) || element.parentElement || element;
      const combinedText = clean((row.innerText || "") + " " + (element.innerText || ""));

      // 1. 文本匹配与对勾字符
      if (/(已完成|已学完|已学习|已学|已考|考试合格|进度\s*[:：]?\s*100%)/.test(combinedText)) {
        return true;
      }
      if (/[✓✔☑✅]/.test(combinedText)) {
        return true;
      }

      // 2. 显式属性与无障碍标记
      if (row.querySelector && row.querySelector("[title*='完成'], [title*='已学'], [title*='通过'], [aria-label*='完成'], [aria-label*='已学'], [aria-label*='通过']")) {
        return true;
      }

      // 3. 收集可能展示图标或状态的元素
      const targets = [
        row,
        element,
        ...(element.previousElementSibling ? [element.previousElementSibling] : []),
        ...(row.querySelectorAll ? Array.from(row.querySelectorAll("i, span, em, svg, [class*='icon'], [class*='status'], [class*='state'], [class*='badge'], [class*='check'], [class*='finish'], [class*='success']")) : [])
      ];

      for (const el of targets) {
        if (!el) continue;
        const cls = String((el.className && typeof el.className === "string" ? el.className : (el.getAttribute && el.getAttribute("class"))) || "").toLowerCase();
        if (
          /(^|[\s_-])(check|checked|checkmark|success|succ|finish|finished|completed|complete|learned|done|pass|passed|wancheng|xuanzhong|is-finish|is-complete|status-1|state-1)([\s_-]|$)/i.test(cls) ||
          /(circle-check|check-circle|icon-check|icon-success|van-icon-success|el-icon-check|anticon-check)/i.test(cls)
        ) {
          return true;
        }

        if (el.tagName && el.tagName.toLowerCase() === "svg") {
          const svgHtml = (el.innerHTML || "").toLowerCase();
          if (/(polyline|check|finish|success|wancheng|xuanzhong)/i.test(svgHtml)) {
            return true;
          }
          const useEl = el.querySelector && el.querySelector("use");
          if (useEl) {
            const href = String(useEl.getAttribute("href") || useEl.getAttribute("xlink:href") || "").toLowerCase();
            if (/(check|finish|success|wancheng)/.test(href)) {
              return true;
            }
          }
          const subPaths = el.querySelectorAll ? el.querySelectorAll("path, polyline, polygon") : [];
          if (subPaths.length >= 2 || ((el.querySelector && el.querySelector("circle")) && subPaths.length >= 1)) {
            return true;
          }
        }

        if (el.getAttribute) {
          const dataStatus = String(el.getAttribute("data-status") || el.getAttribute("data-state") || el.getAttribute("data-type") || "").toLowerCase();
          if (/(finish|completed|success|done|1)/.test(dataStatus)) {
            return true;
          }
        }
      }
      return false;
    };

    const isPhaseOrSectionHeader = (t) => {
      const s = clean(t);
      if (!s) return false;
      return /^(\d{1,2}\s*)?(第[0-9一二三四五六七八九十百\d]+[期阶段部分步回篇讲节章]|模块\s*[0-9一二三四五六七八九十\d]|阶段\s*[0-9一二三四五六七八九十\d])/.test(s) ||
             /^\d{1,2}\s+(第[0-9一二三四五六七八九十百\d]+[期阶段部分步回篇讲节章]|模块)/.test(s) ||
             /^(\d{1,2}\s*)?(第.+[期阶段部分步回篇]|模块\d+)\s*[:：]/.test(s);
    };

    const scoreTitleCandidate = (t) => {
      if (isPhaseOrSectionHeader(t)) return -100;
      let score = 0;
      if (/^(\d{1,2}[\s.、-]|第.+[讲节章步回集课])/.test(t)) score += 15;
      if (t.length >= 4 && t.length <= 60) score += 5;
      if (!/(进度|时长|作者|人看过|人学过)/.test(t)) score += 2;
      return score;
    };

    const titleFrom = (container) => {
      if (!container) return "";
      // 1. 优先查找明确代表标题的元素，排除常见小标签/徽章/按钮类名
      const titleCandidates = Array.from(
        container.querySelectorAll("h1, h2, h3, h4, h5, [class*='title'], [class*='name'], [class*='catalog'], [class*='chapter'], [class*='lesson'], a")
      )
        .filter((el) => {
          if (!visible(el)) return false;
          const cls = String(el.className || "").toLowerCase();
          if (cls.includes("tag") || cls.includes("badge") || cls.includes("status") || cls.includes("btn") || cls.includes("icon")) {
            return false;
          }
          const t = clean(el.innerText);
          return t && t.length >= 2 && !isTagOrBadge(t) && !isMeta(t) && !/^\d{1,2}$/.test(t) && !isSiteOrUiTitle(t) && !isPhaseOrSectionHeader(t);
        })
        .map((el) => clean(el.innerText));

      if (titleCandidates.length > 0) {
        titleCandidates.sort((a, b) => scoreTitleCandidate(b) - scoreTitleCandidate(a) || b.length - a.length);
        return titleCandidates[0];
      }

      // 2. 回退：按行清洗并打分
      const lines = String(container.innerText || "")
        .split(/\n+/)
        .map(clean)
        .filter(Boolean);
      const validLines = lines.filter((line) => line.length >= 2 && !isTagOrBadge(line) && !isMeta(line) && !isSiteOrUiTitle(line) && !isPhaseOrSectionHeader(line));
      if (validLines.length > 0) {
        validLines.sort((a, b) => scoreTitleCandidate(b) - scoreTitleCandidate(a) || b.length - a.length);
        return validLines[0];
      }
      return "";
    };

    const isCourseCard = (element) => {
      if (!element || element === document.body) return false;
      const text = clean(element.innerText);
      if (text.length < 6 || text.length > 1200) return false;

      // 必须排除课程详情总览大卡片（包含原创作者、贡献者、学习人数、课程介绍等元信息）
      if (/(原创作者|贡献者|学习人数|完成任务数|超越员工数|课程介绍|专题介绍|培训介绍|主讲老师\s*[:：])/.test(text)) {
        return false;
      }
      // 必须排除章节总标题面板头部（如“章节 (2) 时长：446分钟”）
      if (/^章节\s*\(\d+\)/.test(text) || /^目录\s*\(/.test(text)) {
        return false;
      }
      // 必须排除期次/阶段/模块纯层级大标题
      if (isPhaseOrSectionHeader(text)) {
        return false;
      }

      // 如果当前元素包含多个子任务（多个学习时长、多个学分或多个完成状态），说明是分组容器而非单门课程
      const selfDurations = countMatches(text, /学习时长\s*[:：]?\s*\d+/g);
      const selfStatusCount = countMatches(text, /(已完成|已学完|已学习|未学习|学习中|已考试)/g);
      const selfCredits = countMatches(text, /(必修学分|选修学分|学分)\s*[:：]?\s*\d+/g);
      if (selfDurations > 1 || selfStatusCount > 1 || selfCredits > 1) {
        return false;
      }

      const title = titleFrom(element);
      if (!title || isTagOrBadge(title) || isMeta(title) || isPhaseOrSectionHeader(title)) return false;

      if (provider === "merchant") {
        const hasMeta = /学习时长|进度|学分|已完成|已考试/.test(text) || /考试/.test(text);
        if (!hasMeta) return false;
        const parent = element.parentElement;
        if (parent && parent !== document.body) {
          const parentText = clean(parent.innerText);
          const parentDurations = countMatches(parentText, /学习时长\s*[:：]?\s*\d+/g);
          const parentStatusCount = countMatches(parentText, /(已完成|已学完|已学习|未学习|学习中|已考试)/g);
          const parentCredits = countMatches(parentText, /(必修学分|选修学分|学分)\s*[:：]?\s*\d+/g);

          // 若父级包含多个任务（说明父级是分组列表），当前元素就是独立的子项目卡片，严禁向上冒泡吞并！
          if (parentDurations > 1 || parentStatusCount > 1 || parentCredits > 1) {
            return true;
          }

          const parentTitle = titleFrom(parent);
          if (
            parentTitle &&
            parentTitle.length > title.length &&
            !isTagOrBadge(parentTitle) &&
            !isPhaseOrSectionHeader(parentTitle) &&
            parentText.length < 600
          ) {
            return false;
          }
        }
        return true;
      } else {
        const hasMeta = /(学时|学分)/.test(text) && /(未学习|已学习|学习中)/.test(text);
        if (!hasMeta) return false;
        const parent = element.parentElement;
        if (parent && parent !== document.body) {
          const parentText = clean(parent.innerText);
          const parentCount = countMatches(parentText, /(未学习|已学习|学习中)/g);
          if (parentCount > 1) {
            return true;
          }
          if (parentText.length < 800 && parentCount === 1) {
            return false;
          }
        }
        return true;
      }
    };

    const courseContainer = (marker) => {
      let element = marker;
      for (let depth = 0; element && element !== document.body && depth < 10; depth++, element = element.parentElement) {
        if (isCourseCard(element)) {
          return element;
        }
      }
      return null;
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
      for (let depth = 0; current && current.parentElement && depth < 8; depth++, current = current.parentElement) {
        const siblings = Array.from(current.parentElement.children);
        const index = siblings.indexOf(current);
        for (let offset = index - 1; offset >= 0; offset--) {
          const text = clean(siblings[offset].innerText);
          if (text && text.length <= 80 && isPhaseOrSectionHeader(text)) {
            const firstLine = text.split(/\n+/).map(clean).find(isPhaseOrSectionHeader) || text;
            return firstLine;
          }
        }
        const headers = Array.from(current.parentElement.querySelectorAll("h1, h2, h3, h4, h5, [class*='header'], [class*='title'], [class*='phase'], [class*='section']"));
        for (const h of headers) {
          if (h !== current && !current.contains(h)) {
            const ht = clean(h.innerText);
            if (ht && ht.length <= 80 && isPhaseOrSectionHeader(ht)) {
              return ht;
            }
          }
        }
      }
      return "";
    };

    const isNavOrHeader = (el) => {
      if (!el) return false;
      if (el.closest && el.closest("nav, header, [class*='navbar'], [class*='nav-'], [class*='menu']")) return true;
      const text = clean(el.innerText || "");
      if (/(学习中心|个人中心|教学管理|学习地图|考试中心|赛事中心|简体中文|消息通知)/.test(text)) return true;
      return false;
    };

    const isInvalidTopicTitle = (s) =>
      !s ||
      s.length < 2 ||
      s.length > 80 ||
      /^\d+\s*分钟$/.test(s) ||
      /^\d+:\d+$/.test(s) ||
      /^\d+\s*人看过$/.test(s) ||
      /^章节\s*\(\d+\)$/.test(s) ||
      /^时长\s*[:：]/.test(s) ||
      /^(标清|高清|超清|倍速|\d+(\.\d+)?倍速|全屏|音量|收起目录|展开目录|课程介绍|主讲老师|收藏|已收藏)$/.test(s) ||
      /^(银商学堂|YS学堂|银联乐学|中国银联|量见[·•]云课堂|量见云课堂)$/i.test(s) ||
      isSiteOrUiTitle(s) ||
      isTagOrBadge(s) ||
      isMeta(s);

    const findTopicTitle = () => {
      // 0. 优先通过页面信息头部特征锚点（起止时间、学习人数、学习进度、原创作者、主讲老师、收藏等）精准定位大标题（兼容专题页图2、课程详情页图3、播放页）
      const metaAnchor = Array.from(document.querySelectorAll("body *")).find((el) => {
        if (!visible(el) || isNavOrHeader(el)) return false;
        const t = clean(el.innerText);
        if (t.length < 2 || t.length > 50) return false;
        return (
          /^(起止时间|学习人数|学习进度|完成任务数|超越员工数|原创作者|贡献者|主讲老师|视频课|收藏|已收藏)$/.test(t) ||
          /^(起止时间|学习人数|学习进度|原创作者|贡献者|主讲老师)\s*[:：]/.test(t) ||
          /^\d+\s*人看过$/.test(t)
        );
      });
      if (metaAnchor) {
        let card = metaAnchor.parentElement;
        for (let d = 0; card && card !== document.body && d < 6; d++, card = card.parentElement) {
          // 优先查找该信息卡内的主标题元素（h1~h4 或 class 含有 title/name 的元素）
          const titleCandidates = Array.from(card.querySelectorAll("h1, h2, h3, h4, [class*='title'], [class*='name']"))
            .filter((el) => {
              if (!visible(el) || isNavOrHeader(el)) return false;
              if (el.closest(".prism-controlbar, .vjs-control-bar, [class*='control-bar'], [class*='speed-list'], [class*='chapter'], [class*='catalog'], [class*='section']")) return false;
              const text = clean(el.innerText);
              return (
                text.length >= 2 &&
                text.length <= 80 &&
                !isInvalidTopicTitle(text) &&
                !/(起止时间|学习人数|学习进度|完成任务数|超越员工数|原创作者|贡献者|主讲老师|收藏|已收藏|人看过|视频课|课程介绍|培训内容|评论|默认封面|目录|返回)/.test(text)
              );
            })
            .map((el) => clean(el.innerText));
          if (titleCandidates.length > 0) {
            return titleCandidates[0];
          }

          const lines = (card.innerText || "").split(/\n+/).map(clean).filter(Boolean);
          const valid = lines.find((l) =>
            l.length >= 2 &&
            l.length <= 80 &&
            !isInvalidTopicTitle(l) &&
            !/(起止时间|学习人数|学习进度|完成任务数|超越员工数|原创作者|贡献者|主讲老师|收藏|已收藏|人看过|视频课|课程介绍|培训内容|评论|默认封面|目录|返回)/.test(l)
          );
          if (valid) return valid;
        }
      }

      // 1. 查找页面上的课程/专题主标题
      const courseMainTitles = Array.from(document.querySelectorAll("h1, h2, h3, [class*='course-title'], [class*='project-title'], [class*='train-title'], [class*='training-title'], [class*='detail-title'], [class*='main-title'], [class*='video-title'], [class*='course-name']"))
        .filter((el) => {
          if (!visible(el) || isNavOrHeader(el)) return false;
          if (el.closest(".prism-controlbar, .vjs-control-bar, [class*='control-bar'], [class*='speed-list'], [class*='chapter'], [class*='catalog'], [class*='section']")) return false;
          const text = clean(el.innerText);
          return (
            text.length >= 2 &&
            text.length <= 80 &&
            !isInvalidTopicTitle(text) &&
            !/(起止时间|学习人数|学习进度|完成任务数|超越员工数|原创作者|贡献者|主讲老师|收藏|已收藏|人看过|视频课|课程介绍|培训内容|评论|默认封面|目录)/.test(text)
          );
        })
        .map((el) => clean(el.innerText));
      if (courseMainTitles.length > 0) {
        return courseMainTitles[0];
      }

      // 2. 银联乐学的“课程大纲”使用 chapterTitle 标识专题名
      const chapterTitleElements = Array.from(document.querySelectorAll(".chapterTitle"));
      for (const element of chapterTitleElements) {
        if (!visible(element)) continue;
        const text = clean(element.getAttribute("title") || element.innerText);
        if (!isInvalidTopicTitle(text)) return text;
      }

      // 3. 页面标题清洗（如“天龙八步™-极简项目管理 - 量见·云课堂 - 学习端”）
      let docTitle = clean(originalTitle);
      docTitle = docTitle
        .replace(/^MTOOL\s*·\s*[^·]+\s*·\s*/i, "")
        .replace(/\s*[-_|\s]\s*(银商学堂|YS学堂|银联乐学|中国银联|培训平台|专题详情|课程详情|量见[·•]云课堂|量见云课堂|云课堂|学习端|播放端).*$/i, "")
        .trim();
      if (docTitle && !isInvalidTopicTitle(docTitle) && docTitle.length >= 2) {
        return docTitle;
      }

      const topicMetaPatterns = [/起止时间/, /课程数/, /浏览人数/, /学习人数/, /学习进度/, /完成标准/, /章节进度/];
      // 4. 扫描当前 DOM 中的可见文本
      const directCandidates = Array.from(document.querySelectorAll("body *"))
        .filter((el) => visible(el) && !isNavOrHeader(el) && el.getClientRects().length > 0)
        .map((el) => ({ element: el, text: ownText(el) }))
        .filter(({ text }) => !isInvalidTopicTitle(text) && text.length >= 4);
      const occurrences = new Map();
      directCandidates.forEach(({ text }) => occurrences.set(text, (occurrences.get(text) || 0) + 1));

      const rankedCandidates = directCandidates.map(({ element, text }) => {
        let score = (occurrences.get(text) || 0) > 1 ? 12 : 0;
        if (/^H[1-5]$/.test(element.tagName)) score += 6;
        if (/title|name/i.test(String(element.className || ""))) score += 2;
        const style = window.getComputedStyle(element);
        const fontSize = Number.parseFloat(style.fontSize || "0");
        const fontWeight = Number.parseInt(style.fontWeight || "0", 10);
        if (fontSize >= 24) score += 5;
        else if (fontSize >= 18) score += 2;
        if (fontWeight >= 600) score += 2;

        let context = element.parentElement;
        for (let depth = 0; context && context !== document.body && depth < 7; depth++, context = context.parentElement) {
          const contextText = clean(context.innerText);
          if (contextText.length > 3000) continue;
          const markerCount = topicMetaPatterns.filter((pattern) => pattern.test(contextText)).length;
          if (markerCount >= 2) {
            score += Math.max(5, 11 - depth);
            break;
          }
          if (markerCount === 1) score += 2;
        }
        return { text, score };
      }).sort((left, right) => right.score - left.score || right.text.length - left.text.length);

      if (rankedCandidates.length > 0 && rankedCandidates[0].score >= 8) {
        return rankedCandidates[0].text;
      }

      return location.hostname || "未知专题";
    };

    // 1. 优先定位右侧章节目录面板（如量见·云课堂/银商学堂右侧章节列表面板）
    const catalogPanel = Array.from(document.querySelectorAll("body *")).find((el) => {
      if (!visible(el) || isNavOrHeader(el)) return false;
      if (el.closest(".prism-player, [class*='player'], [class*='control-bar'], [class*='controls']")) return false;
      const text = clean(el.innerText);
      return /^(章节\s*\(?\d+\)?|目录|课程目录|章节列表)/.test(text) && text.length > 20 && text.length < 8000;
    });

    const parseCatalogCourses = (panel) => {
      if (!panel) return [];
      const candidateItems = Array.from(
        panel.querySelectorAll("li, div, a, [class*='item'], [class*='chapter'], [class*='section'], [class*='node']")
      ).filter((el) => {
        if (!visible(el) || isNavOrHeader(el)) return false;
        if (el.closest(".prism-player, [class*='player'], [class*='control-bar'], [class*='controls'], [class*='speed'], [class*='quality'], [class*='intro'], [class*='teacher']")) return false;
        const text = clean(el.innerText);
        if (text.length < 3 || text.length > 150) return false;
        if (/(倍速|标清|高清|超清|人看过|课程介绍|主讲老师|收起目录|展开目录|00:00)/.test(text)) return false;
        if (/^(章节\s*\(?\d+\)?|时长\s*[:：]|\d+\s*分钟$)/.test(text)) return false;

        const hasDuration = /(\d+)\s*分钟/.test(text);
        const hasChapterMarker = /^(导入|第\d+[期讲节章步回集课]|模块\d+|\d{1,2}[\s.-、])/.test(text);
        const hasProgressOrStatus = /进度\s*[:：]?\s*\d+(?:\.\d+)?%/.test(text) || /(已完成|未学习|学习中|上次学习)/.test(text);
        if (!hasDuration && !hasChapterMarker && !hasProgressOrStatus) return false;

        // 排除包含多个不同章节大标题的祖先容器（如整个目录列表 ul/div）
        const children = Array.from(el.children);
        const subChapters = children.filter((c) => {
          const ct = clean(c.innerText);
          return /^(导入|第\d+[期讲节章步回集课]|模块\d+|\d{1,2}[\s.-、])/.test(ct);
        });
        if (subChapters.length > 1) return false;
        return true;
      });

      const catalogSeen = new Set();
      const parsed = [];
      candidateItems.forEach((item) => {
        const text = clean(item.innerText);
        const lines = text.split(/\n+/).map(clean).filter(Boolean);
        const validLines = lines.filter((l) =>
          l.length >= 2 &&
          !isTagOrBadge(l) &&
          !isMeta(l) &&
          !/^\d+\s*分钟$/.test(l) &&
          !/^(上次学习|已完成|未开始|播放中|试看|\d+:\d+)$/.test(l)
        );
        validLines.sort((a, b) => scoreTitleCandidate(b) - scoreTitleCandidate(a) || b.length - a.length);
        const rawTitle = validLines[0] || lines[0] || "";
        const title = rawTitle.replace(/\s*\d+\s*分钟.*$/, "").replace(/\s*上次学习.*$/, "").trim();
        if (!title || title.length < 2 || isInvalidTopicTitle(title) || isPhaseOrSectionHeader(title) || catalogSeen.has(title)) return;
        catalogSeen.add(title);

        const locator = cssPath(item);
        const url = linkFrom(item);
        const externalId = externalIdFrom(item, url, locator, title);

        let durMatch = text.match(/(\d+)\s*分钟/);
        if (!durMatch && item.parentElement) {
          durMatch = clean(item.parentElement.innerText).match(/(\d+)\s*分钟/);
        }
        let durationSeconds = 0;
        if (durMatch) {
          durationSeconds = (Number(durMatch[1]) || 0) * 60;
        }

        const progressMatch = text.match(/进度\s*[:：]?\s*(\d+(?:\.\d+)?)%/);
        const completed = isElementCompleted(item) || (progressMatch ? Number(progressMatch[1]) >= 100 : false);
        const progress = completed ? 100 : (progressMatch ? Number(progressMatch[1]) : 0);

        const itemKind = detectCourseKind(title, text, durationSeconds, item);

        parsed.push({
          externalId,
          title,
          url,
          locator,
          sectionTitle: "",
          kind: itemKind,
          durationSeconds,
          progress,
          completed
        });
      });
      return parsed;
    };

    let courses = [];
    const catalogCourses = parseCatalogCourses(catalogPanel);
    if (catalogCourses.length > 0) {
      courses = catalogCourses;
    } else {
      // 2. 常规专题页基于 markers 扫描
      const markers = Array.from(document.querySelectorAll("body *")).filter((element) => {
        if (!visible(element)) return false;
        const text = clean(element.innerText);
        if (provider === "merchant") {
          return /^(已完成|已考试|未学习|学习中|去学习|立即学习)$/.test(text) || /^进度\s*[:：]?\s*\d+(?:\.\d+)?%$/.test(text);
        }
        return /^(未学习|已学习|学习中)$/.test(text);
      });

      const seenElements = new Set();
      const seenTitles = new Set();
      markers.forEach((marker) => {
        const container = courseContainer(marker);
        if (!container || seenElements.has(container)) return;
        seenElements.add(container);

        const title = titleFrom(container);
        if (!title || isTagOrBadge(title) || isMeta(title) || isPhaseOrSectionHeader(title) || seenTitles.has(title)) return;
        seenTitles.add(title);

        const text = clean(container.innerText);
        const locator = cssPath(container);
        const url = linkFrom(container);
        const externalId = externalIdFrom(container, url, locator, title);

        const progressMatch = text.match(/进度\s*[:：]?\s*(\d+(?:\.\d+)?)%/);
        const completed = isElementCompleted(container) || (progressMatch ? Number(progressMatch[1]) >= 100 : false);
        const progress = completed ? 100 : (progressMatch ? Number(progressMatch[1]) : 0);

        const durationMatch = text.match(/学习时长\s*[:：]?\s*(\d+)\s*分钟/) || text.match(/学时\s*[:：]?\s*(\d+)/) || text.match(/时长\s*[:：]?\s*(\d+)/);
        let durationSeconds = 0;
        if (durationMatch) {
          const val = Number(durationMatch[1]) || 0;
          if (/学时/.test(durationMatch[0])) {
            durationSeconds = val * 45 * 60;
          } else {
            durationSeconds = val * 60;
          }
        }

        const kind = detectCourseKind(title, text, durationSeconds, container);

        courses.push({
          externalId,
          title,
          url,
          locator,
          sectionTitle: sectionTitleFrom(container),
          kind,
          durationSeconds,
          progress,
          completed
        });
      });

      // 3. 若 markers 依然未匹配到，回退在全局 body 中搜索章节目录列表
      if (courses.length === 0) {
        courses = parseCatalogCourses(document.body);
      }
    }

    courses = courses.filter((c) => {
      if (!c.title || isInvalidTopicTitle(c.title)) return false;
      if (isPhaseOrSectionHeader(c.title)) return false;
      return true;
    });

    const bodyText = clean(document.body.innerText);
    const expiredMatch = bodyText.match(/起止时间\s*[:：]?\s*\d{4}[-/.]\d{1,2}[-/.]\d{1,2}.*?[~至到-]\s*(\d{4}[-/.]\d{1,2}[-/.]\d{1,2}(?:\s+\d{1,2}:\d{1,2}(?::\d{1,2})?)?)/);
    let isTopicExpired = false;
    if (expiredMatch) {
      const endTs = new Date(expiredMatch[1].replace(/-/g, "/")).getTime();
      if (endTs && !isNaN(endTs) && endTs < Date.now()) {
        isTopicExpired = true;
      }
    }
    if (/(计划已结束|培训已结束|活动已结束|学习已结束|项目已结束)/.test(bodyText)) {
      isTopicExpired = true;
    }

    if (isTopicExpired) {
      courses.forEach((c) => {
        c.completed = true;
        c.progress = 100;
      });
    }

    const topicTitle = findTopicTitle();
    const topicProgressMatch = bodyText.match(/学习进度\s*[:：]?\s*(\d+(?:\.\d+)?)%/);
    const countMatch = bodyText.match(/完成任务数\s*(\d+)\s*\/\s*(\d+)/) || bodyText.match(/完成标准\s*(\d+)\s*\/\s*(\d+)/);
    const completedCount = isTopicExpired ? courses.length : (countMatch ? Number(countMatch[1]) : courses.filter((item) => item.completed).length);
    const totalCount = countMatch ? Number(countMatch[2]) : courses.length;
    const payload = {
      title: String(topicTitle || "未知专题"),
      url: location.href,
      progress: isTopicExpired ? 100 : (topicProgressMatch ? Number(topicProgressMatch[1]) : (totalCount ? completedCount / totalCount * 100 : 0)),
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
    chunks.forEach((chunk, index) => {
      window.setTimeout(() => {
        if (window.__MTOOL_CAPTURE_REQUEST__ !== requestId) return;
        document.title = "MTOOL_CAPTURE|" + requestId + "|" + index + "|" + chunks.length + "|" + encoded.length + "|" + chunk;
        if (index === chunks.length - 1) window.setTimeout(() => { document.title = originalTitle; }, __RESTORE_DELAY__);
      }, index * __CHUNK_INTERVAL__);
    });
    } catch (err) {
      const errorPayload = {
        title: "错误",
        url: location.href,
        progress: 0,
        totalCount: 0,
        completedCount: 0,
        courses: []
      };
      const bytes = new TextEncoder().encode(JSON.stringify(errorPayload));
      let binary = "";
      bytes.forEach((byte) => { binary += String.fromCharCode(byte); });
      const encoded = btoa(binary);
      if (window.__MTOOL_CAPTURE_REQUEST__ === requestId) {
        document.title = "MTOOL_CAPTURE|" + requestId + "|0|1|" + encoded.length + "|" + encoded;
      }
    }
  }, 0);
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
    if let Some(request_id) = title.strip_prefix(BRIDGE_CAPTURE_START_PREFIX) {
        let mut exchange = captures.lock().unwrap_or_else(|error| error.into_inner());
        if exchange.active_requests.contains(request_id) {
            exchange.started_requests.insert(request_id.to_string());
        }
        return true;
    }

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
        if !exchange.active_requests.contains(&request_id) {
            return true;
        }
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
            exchange.active_requests.remove(&request_id);
            exchange.started_requests.remove(&request_id);
            exchange.completed.insert(request_id, parsed);
        }
        return true;
    }

    if let Some(payload) = title.strip_prefix(BRIDGE_MEDIA_PREFIX) {
        let parts: Vec<&str> = payload.split('|').collect();
        if parts.len() >= 4 && parts[0] == provider.key() {
            let event = parts[1];
            let current_time = parts[2].parse::<f64>().unwrap_or(0.0);
            let duration = parts[3].parse::<f64>().unwrap_or(0.0);
            let mut state = runtime.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(active) = state.active.get_mut(provider.key()) {
                active.last_media_at = now();
                if current_time > active.last_advanced_time + 0.1 {
                    active.last_advanced_time = current_time;
                    active.last_progress_at = now();
                }
                if current_time > 0.0 {
                    active.current_time = current_time;
                }
                if duration > 0.0 {
                    active.duration = duration;
                }
                if event == "ended" {
                    active.phase = "ended".to_string();
                    active.phase_since = now();
                } else if event == "need_login" {
                    active.phase = "need_login".to_string();
                    active.phase_since = now();
                } else if event == "error" {
                    active.phase = "error".to_string();
                    active.phase_since = now();
                } else if active.phase == "opening"
                    && matches!(event, "play" | "playing" | "timeupdate")
                {
                    active.phase = "playing".to_string();
                    active.phase_since = now();
                }
            }
        }
        return true;
    }
    false
}

async fn ensure_player_window(
    app: &AppHandle,
    state: &VideoTaskState,
    provider: Provider,
    show: bool,
) -> Result<tauri::WebviewWindow, String> {
    let label = provider.player_label();
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
        .title(format!("MTOOL · {} · 播放窗口", provider.name()))
        .inner_size(1280.0, 820.0)
        .min_inner_size(640.0, 480.0)
        .visible(false)
        .focused(false)
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
        .map_err(|error| format!("打开{}播放窗口失败: {error}", provider.name()))?;

    let win_for_close = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = win_for_close.hide();
        }
    });

    if show {
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
    } else {
        let _ = window.hide();
    }
    Ok(window)
}

async fn ensure_browser_window(
    app: &AppHandle,
    state: &VideoTaskState,
    provider: Provider,
    show: bool,
) -> Result<tauri::WebviewWindow, String> {
    let label = provider.browser_label();
    if let Some(window) = app.get_webview_window(&label) {
        if show {
            window.show().map_err(|error| error.to_string())?;
            window.unminimize().map_err(|error| error.to_string())?;
            window.set_focus().map_err(|error| error.to_string())?;
        }
        return Ok(window);
    }
    let url = provider
        .home()
        .parse::<tauri::Url>()
        .map_err(|error| error.to_string())?;
    let captures = state.captures.clone();
    let runtime = state.runtime.clone();
    let provider_for_title = provider;
    let app_for_popup = app.clone();
    let popup_label = label.clone();
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        .title(format!("MTOOL · {} · 选专题", provider.name()))
        .inner_size(1280.0, 820.0)
        .min_inner_size(640.0, 480.0)
        .visible(show)
        .focused(show)
        .initialization_script_for_all_frames(browser_nav_script(provider))
        .on_document_title_changed(move |window, title| {
            if !handle_bridge_title(&title, provider_for_title, &captures, &runtime) {
                let _ =
                    window.set_title(&format!("MTOOL · {} · {title}", provider_for_title.name()));
            }
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
        .map_err(|error| format!("打开{}浏览窗口失败: {error}", provider.name()))?;

    let win_for_close = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = win_for_close.hide();
        }
    });

    Ok(window)
}

async fn ensure_window(
    app: &AppHandle,
    state: &VideoTaskState,
    provider: Provider,
    show: bool,
) -> Result<tauri::WebviewWindow, String> {
    ensure_player_window(app, state, provider, show).await
}

async fn capture_current(
    app: &AppHandle,
    state: &VideoTaskState,
    provider: Provider,
) -> Result<PageTopicCapture, String> {
    let window = app
        .get_webview_window(&provider.browser_label())
        .or_else(|| app.get_webview_window(&provider.player_label()))
        .ok_or_else(|| format!("请先点击【登录并选择专题】打开{}", provider.name()))?;
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
            exchange.active_requests.insert(request_id.clone());
            exchange.started_requests.remove(&request_id);
            exchange.buffers.remove(&request_id);
            exchange.completed.remove(&request_id);
        }
        if let Err(error) = window.eval(capture_script(&request_id, provider)) {
            let mut exchange = state
                .captures
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            exchange.active_requests.remove(&request_id);
            exchange.started_requests.remove(&request_id);
            exchange.buffers.remove(&request_id);
            exchange.completed.remove(&request_id);
            return Err(format!("读取专题页面失败: {error}"));
        }

        let attempt_started = Instant::now();
        let mut bridge_started = false;
        let mut received_chunks = 0usize;
        let mut total_chunks = 0usize;
        let mut encoded_len = 0usize;
        let mut last_progress = attempt_started;
        let mut finished = None;

        loop {
            tokio::time::sleep(Duration::from_millis(CAPTURE_POLL_INTERVAL_MS)).await;
            let (result, started, received, total, length) = {
                let mut exchange = state
                    .captures
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let result = exchange.completed.remove(&request_id);
                let started = exchange.started_requests.contains(&request_id);
                let (received, total, length) = exchange
                    .buffers
                    .get(&request_id)
                    .map(|buffer| {
                        (
                            buffer.chunks.iter().filter(|chunk| chunk.is_some()).count(),
                            buffer.total,
                            buffer.encoded_len,
                        )
                    })
                    .unwrap_or((0, 0, 0));
                (result, started, received, total, length)
            };

            if let Some(result) = result {
                finished = Some(result);
                break;
            }
            if started || received > 0 {
                bridge_started = true;
            }
            if received > received_chunks {
                received_chunks = received;
                total_chunks = total;
                encoded_len = length;
                last_progress = Instant::now();
            }

            let elapsed = attempt_started.elapsed();
            let timed_out = elapsed >= Duration::from_millis(CAPTURE_TOTAL_TIMEOUT_MS)
                || (!bridge_started && elapsed >= Duration::from_millis(CAPTURE_START_TIMEOUT_MS))
                || (bridge_started
                    && received_chunks == 0
                    && elapsed >= Duration::from_millis(CAPTURE_SCAN_TIMEOUT_MS))
                || (received_chunks > 0
                    && last_progress.elapsed() >= Duration::from_millis(CAPTURE_IDLE_TIMEOUT_MS));
            if timed_out {
                break;
            }
        }

        let late_result = {
            let mut exchange = state
                .captures
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let result = exchange.completed.remove(&request_id);
            bridge_started |= exchange.started_requests.remove(&request_id);
            if let Some(buffer) = exchange.buffers.remove(&request_id) {
                let received = buffer.chunks.iter().filter(|chunk| chunk.is_some()).count();
                if received > received_chunks {
                    received_chunks = received;
                    total_chunks = buffer.total;
                    encoded_len = buffer.encoded_len;
                }
            }
            exchange.active_requests.remove(&request_id);
            result
        };
        if finished.is_none() {
            finished = late_result;
        }
        if let Some(result) = finished {
            match result {
                Ok(capture) => return Ok(capture),
                Err(error) if attempt == 0 => {
                    last_error = error;
                    continue;
                }
                Err(error) => return Err(format!("{error}；自动重试后仍未成功")),
            }
        }

        last_error = if received_chunks > 0 && total_chunks > 0 {
            format!(
                "专题页面数据回传中断（已收到 {received_chunks}/{total_chunks} 段，共 {encoded_len} 字符）"
            )
        } else if bridge_started {
            "专题页面脚本已启动，但页面扫描在 30 秒内未生成数据".to_string()
        } else {
            "专题页面脚本未能启动，请等待页面加载完成后重试".to_string()
        };
        eprintln!(
            "[mtool video task] capture attempt {}/2 failed: {}",
            attempt + 1,
            last_error
        );
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

    let valid_courses: Vec<_> = capture
        .courses
        .into_iter()
        .filter(|c| !is_phase_or_section_title(&c.title))
        .collect();

    let mut current_course_ids = Vec::new();
    let mut manual = 0usize;
    let mut completed = 0usize;
    for (index, course) in valid_courses.iter().enumerate() {
        let kind = normalize_kind(&course.kind);
        let external_id = if course.external_id.trim().is_empty() {
            format!("{}-{index}", course.title)
        } else {
            course.external_id.clone()
        };
        let course_id = stable_id(&[&topic_id, &external_id]);
        current_course_ids.push(course_id.clone());
        let status = if course.completed {
            completed += 1;
            "completed"
        } else if kind == "video" || kind == "slides" {
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
                   duration_seconds=CASE WHEN excluded.duration_seconds > 0 THEN excluded.duration_seconds ELSE video_courses.duration_seconds END,
                   progress=CASE
                     WHEN excluded.status='completed' THEN 100.0
                     WHEN video_courses.status='completed' THEN 100.0
                     WHEN excluded.progress > 0.0 THEN excluded.progress
                     ELSE video_courses.progress
                   END,
                   status=CASE
                     WHEN excluded.status='completed' THEN 'completed'
                     WHEN video_courses.status IN('opening','playing','verifying') THEN video_courses.status
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
    if !current_course_ids.is_empty() {
        let placeholders = current_course_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "DELETE FROM video_courses WHERE topic_id = ?1 AND status NOT IN ('opening', 'playing', 'verifying') AND id NOT IN ({placeholders})"
        );
        let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::new();
        params_vec.push(&topic_id);
        for id in &current_course_ids {
            params_vec.push(id);
        }
        transaction
            .execute(&sql, rusqlite::params_from_iter(params_vec))
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())?;

    let mut runtime = state
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for (index, course) in valid_courses.iter().enumerate() {
        if course.completed {
            let external_id = if course.external_id.trim().is_empty() {
                format!("{}-{index}", course.title)
            } else {
                course.external_id.clone()
            };
            let course_id = stable_id(&[&topic_id, &external_id]);
            runtime.active.retain(|_, active| active.course_id != course_id);
        }
    }
    Ok(ImportSummary {
        topic_id,
        topic_title: capture.title,
        imported: valid_courses.len(),
        completed,
        manual,
    })
}

fn is_phase_or_section_title(title: &str) -> bool {
    let t = title.trim();
    let trimmed = t.trim_start_matches(|c: char| c.is_ascii_digit() || c.is_whitespace() || c == '.' || c == '-' || c == '、');
    if trimmed.starts_with('第') && (trimmed.contains('期') || trimmed.contains("阶段") || trimmed.contains("部分") || trimmed.contains('篇') || trimmed.contains('讲') || trimmed.contains('节') || trimmed.contains('章')) {
        return true;
    }
    if trimmed.starts_with("模块") || trimmed.starts_with("阶段") {
        return true;
    }
    false
}

fn load_course(path: &PathBuf, course_id: &str) -> Result<CourseRecord, String> {
    let conn = Connection::open(path).map_err(|error| error.to_string())?;
    conn.query_row(
        "SELECT id,topic_id,provider,url,locator,kind,title,duration_seconds,progress,sort_order FROM video_courses WHERE id=?1",
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
                title: row.get(6)?,
                duration_seconds: row.get(7)?,
                progress: row.get(8)?,
                sort_order: row.get(9)?,
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
    // 队列自动播放时 (auto_play = true) 保持窗口隐藏；仅当用户手动点击“打开考试/打开内容”时才显示窗口
    let window = ensure_window(app, state, course.provider, !auto_play).await?;
    if auto_play {
        let _ = window.hide();
    }
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
        let topic_url = topic_url(state.db_path.as_ref(), &course.topic_id)?;
        let url = topic_url
            .parse::<tauri::Url>()
            .map_err(|error| error.to_string())?;
        window.navigate(url).map_err(|error| error.to_string())?;
        let click_script = course_click_script(&course.title, &course.locator);
        let click_window = window.clone();
        let click_provider = course.provider;
        tauri::async_runtime::spawn(async move {
            for delay in [1200, 2500, 4500, 8000] {
                tokio::time::sleep(Duration::from_millis(delay)).await;
                let current_url = click_window.url().ok();
                if current_url.as_ref().is_some_and(|current| {
                    current.as_str() != topic_url && provider_accepts_url(click_provider, current)
                }) {
                    break;
                }
                if current_url
                    .as_ref()
                    .is_none_or(|current| !provider_accepts_url(click_provider, current))
                {
                    let Ok(recovery_url) = topic_url.parse::<tauri::Url>() else {
                        break;
                    };
                    if click_window.navigate(recovery_url).is_err() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                }
                let _ = click_window.eval(&click_script);
            }
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
            for delay in [1000, 2200, 4000, 6500] {
                tokio::time::sleep(Duration::from_millis(delay)).await;
                let _ = play_window.eval(update_media_script(settings.speed, settings.muted, true));
            }
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
        "SELECT id FROM video_courses WHERE status='pending' AND kind IN ('video', 'slides') AND provider=?1
         ORDER BY sort_order,updated_at LIMIT 1"
    } else {
        "SELECT id FROM video_courses WHERE status='pending' AND kind IN ('video', 'slides')
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
        ensure_window(app, state, course.provider, false).await?;
        return Ok(false);
    }
    open_course(app, state, &course, true).await?;
    let timestamp = now();
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    let _ = conn.execute(
        "UPDATE video_courses SET status='pending',updated_at=?2 WHERE provider=?1 AND id != ?3 AND status IN ('opening','playing','verifying')",
        params![course.provider.key(), timestamp, course.id],
    );
    conn.execute(
        "UPDATE video_courses SET status='opening',last_error=NULL,updated_at=?2 WHERE id=?1",
        params![course.id, timestamp],
    )
    .map_err(|error| error.to_string())?;
    let initial_duration = course.duration_seconds as f64;
    let initial_time = if initial_duration > 0.0 && course.progress > 0.0 {
        (course.progress / 100.0) * initial_duration
    } else {
        0.0
    };
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
                phase: "opening".to_string(),
                phase_since: timestamp,
                last_media_at: timestamp,
                last_progress_at: timestamp,
                last_advanced_time: initial_time,
                current_time: initial_time,
                duration: initial_duration,
            },
        );
    Ok(true)
}



#[tauri::command]
pub fn get_video_task_dashboard(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
) -> Result<VideoTaskDashboard, String> {
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    let runtime_active = state
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .active
        .clone();

    let mut topic_stmt = conn
        .prepare(
            "SELECT id,provider,title,url,progress,total_count,completed_count,last_synced_at
             FROM video_topics ORDER BY rowid ASC",
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

        let mut mapped_courses = Vec::new();
        for mut course in courses {
            if let Some(active) = runtime_active.values().find(|a| a.course_id == course.id) {
                course.status = active.phase.clone();
                if active.duration > 0.0 {
                    course.duration_seconds = active.duration as i64;
                    course.progress =
                        ((active.current_time / active.duration) * 100.0).clamp(0.0, 100.0);
                }
            } else if matches!(course.status.as_str(), "opening" | "playing" | "verifying") {
                course.status = "pending".to_string();
            }
            if course.status != "attention" {
                course.last_error = None;
            }
            stats.total += 1;
            match course.status.as_str() {
                "completed" => stats.completed += 1,
                "pending" => stats.pending += 1,
                "opening" | "playing" | "verifying" => stats.running += 1,
                "manual" => stats.manual += 1,
                "attention" => stats.attention += 1,
                _ => {}
            }
            mapped_courses.push(course);
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
            courses: mapped_courses,
        });
    }
    let sources = [Provider::Ulearn, Provider::Merchant]
        .into_iter()
        .map(|provider| {
            let browser_win = app.get_webview_window(&provider.browser_label());
            let player_win = app.get_webview_window(&provider.player_label());
            let window = browser_win.as_ref().or(player_win.as_ref());
            SourceStatus {
                provider: provider.key().to_string(),
                name: provider.name().to_string(),
                home_url: provider.home().to_string(),
                window_open: browser_win.is_some() || player_win.is_some(),
                current_url: window.and_then(|window| window.url().ok().map(|url| url.to_string())),
            }
        })
        .collect();
    let mut settings = state
        .settings
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    if stats.running == 0 && stats.pending == 0 && settings.running {
        settings.running = false;
        if let Ok(mut lock) = state.settings.lock() {
            lock.running = false;
        }
        let _ = persist_settings(state.db_path.as_ref(), &settings);
    }
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
    let provider = Provider::parse(&provider)?;
    ensure_browser_window(&app, state.inner(), provider, true)
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
    let window = ensure_browser_window(&app, state.inner(), provider, false).await?;
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
pub async fn open_video_topic(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    topic_id: String,
) -> Result<(), String> {
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
    let window = ensure_browser_window(&app, state.inner(), provider, true).await?;
    let target_url = if !url.trim().is_empty() {
        url
    } else {
        provider.home().to_string()
    };
    if let Ok(parsed) = target_url.parse::<tauri::Url>() {
        let _ = window.navigate(parsed);
    }
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    Ok(())
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
        if let Some(window) = app.get_webview_window(&provider.player_label()) {
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
        if let Some(window) = app.get_webview_window(&provider.player_label()) {
            let _ = window.hide();
            let _ = window.eval(update_media_script(settings.speed, settings.muted, true));
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn show_video_learning_window(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    provider: String,
) -> Result<(), String> {
    let provider = Provider::parse(&provider)?;
    ensure_browser_window(&app, state.inner(), provider, true).await?;
    Ok(())
}

#[tauri::command]
pub fn hide_video_learning_window(app: AppHandle, provider: String) -> Result<(), String> {
    let provider = Provider::parse(&provider)?;
    if let Some(window) = app.get_webview_window(&provider.browser_label()) {
        window.hide().map_err(|error| error.to_string())?;
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
            let _ = window.eval(update_media_script(settings.speed, settings.muted, false));
            let _ = window.eval(
                "if (window.__MTOOL_LEARNING_BRIDGE__) { window.__MTOOL_LEARNING_BRIDGE__.update(1, true, false); }
                 document.querySelectorAll('video,audio').forEach((media)=>{ try { media.pause(); } catch(_) {} });"
            );
        }
    }

    let active_courses = {
        let mut runtime = state
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        std::mem::take(&mut runtime.active)
    };

    if let Ok(conn) = Connection::open(state.db_path.as_ref()) {
        for (_key, active) in active_courses {
            if active.duration > 0.0 && active.current_time > 0.0 {
                let progress = ((active.current_time / active.duration) * 100.0).clamp(0.0, 100.0);
                let _ = conn.execute(
                    "UPDATE video_courses SET status='pending',progress=?2,duration_seconds=?3,updated_at=?4 WHERE id=?1 AND status IN('opening','playing','verifying')",
                    params![active.course_id, progress, active.duration as i64, now()],
                );
            } else {
                let _ = conn.execute(
                    "UPDATE video_courses SET status='pending',updated_at=?2 WHERE id=?1 AND status IN('opening','playing','verifying')",
                    params![active.course_id, now()],
                );
            }
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn pause_video_course(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    course_id: String,
) -> Result<(), String> {
    let course = load_course(state.db_path.as_ref(), &course_id)?;
    let timestamp = now();

    // 1. 停止当前窗口内的视频播放
    if let Some(window) = app.get_webview_window(&course.provider.label()) {
        let _ = window.eval(
            "if (window.__MTOOL_LEARNING_BRIDGE__) { window.__MTOOL_LEARNING_BRIDGE__.update(1, true, false); }
             document.querySelectorAll('video,audio').forEach((media)=>{ try { media.pause(); } catch(_) {} });"
        );
    }

    // 2. 从 active 中获取播放进度并移除 active
    let (current_time, duration) = {
        let mut runtime = state
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(active) = runtime.active.get(course.provider.key()) {
            if active.course_id == course_id {
                let time_and_dur = (active.current_time, active.duration);
                runtime.active.remove(course.provider.key());
                time_and_dur
            } else {
                (0.0, 0.0)
            }
        } else {
            (0.0, 0.0)
        }
    };

    // 3. 更新数据库：将当前课程设为 pending 并记录当前进度
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    if duration > 0.0 && current_time > 0.0 {
        let progress = ((current_time / duration) * 100.0).clamp(0.0, 100.0);
        let _ = conn.execute(
            "UPDATE video_courses SET status='pending',progress=?2,duration_seconds=?3,updated_at=?4 WHERE id=?1",
            params![course.id, progress, duration as i64, timestamp],
        );
    } else {
        let _ = conn.execute(
            "UPDATE video_courses SET status='pending',updated_at=?2 WHERE id=?1",
            params![course.id, timestamp],
        );
    }

    // 4. 确保全局队列处于运行状态，自动播放下一个视频
    {
        let mut settings = state
            .settings
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        settings.running = true;
        let _ = persist_settings(state.db_path.as_ref(), &settings);
    }

    // 5. 优先寻找同专题下 sort_order > current.sort_order 的下一个待播放课程
    let next_course_id: Option<String> = conn
        .query_row(
            "SELECT id FROM video_courses
             WHERE status='pending' AND kind IN ('video', 'slides') AND provider=?1 AND topic_id=?2 AND sort_order > ?3
             ORDER BY sort_order ASC LIMIT 1",
            params![course.provider.key(), course.topic_id, course.sort_order],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .or_else(|| {
            conn.query_row(
                "SELECT id FROM video_courses
                 WHERE status='pending' AND kind IN ('video', 'slides') AND provider=?1 AND id != ?2
                 ORDER BY sort_order, updated_at LIMIT 1",
                params![course.provider.key(), course.id],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten()
        });

    drop(conn);

    // 6. 如果有下一门待播放视频，立即开启播放下一门
    if let Some(next_id) = next_course_id {
        let next_course = load_course(state.db_path.as_ref(), &next_id)?;
        open_course(&app, state.inner(), &next_course, true).await?;
        let next_ts = now();
        let conn2 = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
        let _ = conn2.execute(
            "UPDATE video_courses SET status='pending',updated_at=?2 WHERE provider=?1 AND id != ?3 AND status IN ('opening','playing','verifying')",
            params![next_course.provider.key(), next_ts, next_course.id],
        );
        conn2.execute(
            "UPDATE video_courses SET status='opening',last_error=NULL,updated_at=?2 WHERE id=?1",
            params![next_course.id, next_ts],
        )
        .map_err(|error| error.to_string())?;
        let next_duration = next_course.duration_seconds as f64;
        let next_time = if next_duration > 0.0 && next_course.progress > 0.0 {
            (next_course.progress / 100.0) * next_duration
        } else {
            0.0
        };
        state
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .insert(
                next_course.provider.key().to_string(),
                ActiveCourse {
                    course_id: next_course.id,
                    topic_id: next_course.topic_id,
                    provider: next_course.provider,
                    phase: "opening".to_string(),
                    phase_since: next_ts,
                    last_media_at: next_ts,
                    last_progress_at: next_ts,
                    last_advanced_time: next_time,
                    current_time: next_time,
                    duration: next_duration,
                },
            );
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
            // 预留 2 秒缓冲时间，确保网课平台完成完播网络上报后再切集
            if now() - active.phase_since < 2 {
                continue;
            }
            let timestamp = now();
            if let Ok(conn) = Connection::open(state.db_path.as_ref()) {
                let duration_secs = if active.duration > 0.0 {
                    active.duration as i64
                } else {
                    0
                };
                let _ = conn.execute(
                    "UPDATE video_courses SET status='completed',progress=100.0,duration_seconds=CASE WHEN duration_seconds > 0 THEN duration_seconds ELSE ?2 END,last_error=NULL,updated_at=?3 WHERE id=?1",
                    params![active.course_id, duration_secs, timestamp],
                );
                let _ = conn.execute(
                    "UPDATE video_topics SET 
                     completed_count = (SELECT COUNT(*) FROM video_courses WHERE topic_id=?1 AND status='completed'),
                     total_count = (SELECT COUNT(*) FROM video_courses WHERE topic_id=?1),
                     progress = ROUND((CAST((SELECT COUNT(*) FROM video_courses WHERE topic_id=?1 AND status='completed') AS REAL) / MAX(1, (SELECT COUNT(*) FROM video_courses WHERE topic_id=?1))) * 100.0, 1),
                     last_synced_at = ?2
                     WHERE id=?1",
                    params![active.topic_id, timestamp],
                );
            }
            state
                .runtime
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .active
                .remove(active.provider.key());
        } else if active.phase == "opening" || active.phase == "playing" {
            if let Some(window) = app.get_webview_window(&active.provider.label()) {
                let _ = window.eval(update_media_script(settings.speed, settings.muted, true));
            }
            if active.phase == "playing" && active.duration > 0.0 {
                let progress = ((active.current_time / active.duration) * 100.0).clamp(0.0, 100.0);
                if let Ok(conn) = Connection::open(state.db_path.as_ref()) {
                    let _ = conn.execute(
                        "UPDATE video_courses SET progress=?2,duration_seconds=?3,updated_at=?4 WHERE id=?1",
                        params![active.course_id, progress, active.duration as i64, now()],
                    );
                }
            }
            // 1. 播放卡住检测：如果进度停滞（包括 0% 状态）超过 5 分钟（300 秒），自动跳过并播放下一门
            let stall_duration = now() - active.last_progress_at;
            if stall_duration >= 300 {
                let conn =
                    Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
                let last_error = if active.current_time <= 0.1 {
                    "视频卡在0%超过5分钟，已自动跳过并开始播放下一门".to_string()
                } else {
                    let mins = (active.current_time / 60.0).floor() as i64;
                    format!("视频播放卡住超过5分钟未推进（停在约{mins}分钟），已自动跳过并开始播放下一门")
                };
                let progress = if active.duration > 0.0 && active.current_time > 0.0 {
                    ((active.current_time / active.duration) * 100.0).clamp(0.0, 100.0)
                } else {
                    0.0
                };
                conn.execute(
                    "UPDATE video_courses SET status='attention',progress=?2,last_error=?3,updated_at=?4 WHERE id=?1",
                    params![active.course_id, progress, last_error, now()],
                )
                .map_err(|error| error.to_string())?;
                state
                    .runtime
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .active
                    .remove(active.provider.key());
                continue;
            }

            // 2. 网页探活超时检测（90 秒没有任何网页心跳）
            let last_activity = if active.phase == "opening" {
                active.phase_since
            } else {
                active.last_media_at
            };
            if now() - last_activity > 90 {
                let conn =
                    Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
                let is_login_url = app
                    .get_webview_window(&active.provider.label())
                    .and_then(|window| window.url().ok())
                    .map(|url| {
                        let s = url.as_str().to_lowercase();
                        s.contains("/login") || s.contains("/sso") || s.contains("/cas/") || s.contains("oauth")
                    })
                    .unwrap_or(false);

                let last_error = if is_login_url {
                    if let Some(window) = app.get_webview_window(&active.provider.label()) {
                        let _ = window.show();
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                    }
                    "平台登录已失效或需要扫码登录，请点击“打开登录”完成登录"
                } else if active.phase == "opening" {
                    "未检测到可播放的视频，请打开课程检查"
                } else {
                    app.get_webview_window(&active.provider.label())
                        .and_then(|window| window.url().ok())
                        .filter(|url| provider_accepts_url(active.provider, url))
                        .map(|_| "未检测到可持续播放的视频，请打开课程检查")
                        .unwrap_or("课程页面未成功打开或已进入空白页，请重试后检查")
                };
                conn.execute(
                    "UPDATE video_courses SET status='attention',last_error=?2 WHERE id=?1",
                    params![active.course_id, last_error],
                )
                .map_err(|error| error.to_string())?;
                state
                    .runtime
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .active
                    .remove(active.provider.key());
            }
        } else if active.phase == "need_login" {
            let conn =
                Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
            conn.execute(
                "UPDATE video_courses SET status='attention',last_error='平台登录已失效或需要扫码登录，请点击“打开登录”完成登录' WHERE id=?1",
                params![active.course_id],
            )
            .map_err(|error| error.to_string())?;
            if let Some(window) = app.get_webview_window(&active.provider.label()) {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
            state
                .runtime
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .active
                .remove(active.provider.key());
        } else if active.phase == "error" {
            let conn =
                Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
            conn.execute(
                "UPDATE video_courses SET status='attention',last_error='播放发生异常，请打开课程检查' WHERE id=?1",
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
    let mut any_started = false;
    if settings.cross_site_parallel {
        for provider in [Provider::Ulearn, Provider::Merchant] {
            if !active_providers.iter().any(|key| key == provider.key()) {
                if let Ok(true) = start_one(&app, state.inner(), Some(provider)).await {
                    any_started = true;
                }
            }
        }
    } else if active_providers.is_empty() {
        if let Ok(true) = start_one(&app, state.inner(), None).await {
            any_started = true;
        }
    }

    let active_count = state
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .active
        .len();
    if active_count == 0 && !any_started {
        let mut settings = state
            .settings
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if settings.running {
            settings.running = false;
            let _ = persist_settings(state.db_path.as_ref(), &settings);
        }
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
    if course.kind != "video" {
        // 课件、考试、资料属于用户手动交互内容，在前台选专题/浏览窗口打开，完全不打扰后台视频播放队列
        let window = ensure_browser_window(&app, state.inner(), course.provider, true).await?;
        if !course.url.trim().is_empty() {
            if let Ok(url) = course.url.parse::<tauri::Url>() {
                let _ = window.navigate(url);
            }
        } else {
            let topic_url = topic_url(state.db_path.as_ref(), &course.topic_id)?;
            if let Ok(url) = topic_url.parse::<tauri::Url>() {
                let _ = window.navigate(url);
                let click_script = course_click_script(&course.title, &course.locator);
                let click_window = window.clone();
                let click_provider = course.provider;
                tauri::async_runtime::spawn(async move {
                    for delay in [1200, 2500, 4500, 8000] {
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                        let current_url = click_window.url().ok();
                        if current_url.as_ref().is_some_and(|current| {
                            current.as_str() != topic_url && provider_accepts_url(click_provider, current)
                        }) {
                            break;
                        }
                        let _ = click_window.eval(&click_script);
                    }
                });
            }
        }
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return Ok(());
    }

    let is_currently_running = {
        let runtime = state
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        runtime
            .active
            .get(course.provider.key())
            .map(|active| active.course_id == course_id)
            .unwrap_or(false)
    };
    if is_currently_running {
        let window = ensure_player_window(&app, state.inner(), course.provider, true).await?;
        window.show().map_err(|error| error.to_string())?;
        window.unminimize().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    } else {
        let old_active = {
            let mut runtime = state
                .runtime
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            runtime.active.remove(course.provider.key())
        };
        if let Some(old) = old_active {
            if let Ok(conn) = Connection::open(state.db_path.as_ref()) {
                if old.duration > 0.0 && old.current_time > 0.0 {
                    let progress = ((old.current_time / old.duration) * 100.0).clamp(0.0, 100.0);
                    let _ = conn.execute(
                        "UPDATE video_courses SET status='pending',progress=?2,duration_seconds=?3,updated_at=?4 WHERE id=?1",
                        params![old.course_id, progress, old.duration as i64, now()],
                    );
                } else {
                    let _ = conn.execute(
                        "UPDATE video_courses SET status='pending',updated_at=?2 WHERE id=?1",
                        params![old.course_id, now()],
                    );
                }
            }
        }
    }
    open_course(&app, state.inner(), &course, false).await?;
    let timestamp = now();
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    let _ = conn.execute(
        "UPDATE video_courses SET status='pending',updated_at=?2 WHERE provider=?1 AND id != ?3 AND status IN ('opening','playing','verifying')",
        params![course.provider.key(), timestamp, course.id],
    );
    conn.execute(
        "UPDATE video_courses SET status='opening',last_error=NULL,updated_at=?2 WHERE id=?1",
        params![course.id, timestamp],
    )
    .map_err(|error| error.to_string())?;
    let initial_duration = course.duration_seconds as f64;
    let initial_time = if initial_duration > 0.0 && course.progress > 0.0 {
        (course.progress / 100.0) * initial_duration
    } else {
        0.0
    };
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
                phase: "opening".to_string(),
                phase_since: timestamp,
                last_media_at: timestamp,
                last_progress_at: timestamp,
                last_advanced_time: initial_time,
                current_time: initial_time,
                duration: initial_duration,
            },
        );
    Ok(())
}

#[tauri::command]
pub fn retry_video_course(
    state: tauri::State<'_, VideoTaskState>,
    course_id: String,
) -> Result<(), String> {
    let course = load_course(state.db_path.as_ref(), &course_id)?;
    let next_status = if course.kind == "video" || course.kind == "slides" {
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
pub fn complete_video_course(
    state: tauri::State<'_, VideoTaskState>,
    course_id: String,
) -> Result<(), String> {
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    let timestamp = now();
    let topic_id: String = conn
        .query_row(
            "SELECT topic_id FROM video_courses WHERE id=?1",
            params![course_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    conn.execute(
        "UPDATE video_courses SET status='completed',progress=100.0,last_error=NULL,updated_at=?2 WHERE id=?1",
        params![course_id, timestamp],
    )
    .map_err(|error| error.to_string())?;
    conn.execute(
        "UPDATE video_topics SET 
         completed_count = (SELECT COUNT(*) FROM video_courses WHERE topic_id=?1 AND status='completed'),
         total_count = (SELECT COUNT(*) FROM video_courses WHERE topic_id=?1),
         progress = ROUND((CAST((SELECT COUNT(*) FROM video_courses WHERE topic_id=?1 AND status='completed') AS REAL) / MAX(1, (SELECT COUNT(*) FROM video_courses WHERE topic_id=?1))) * 100.0, 1),
         last_synced_at = ?2
         WHERE id=?1",
        params![topic_id, timestamp],
    )
    .map_err(|error| error.to_string())?;
    let mut runtime = state.runtime.lock().unwrap_or_else(|error| error.into_inner());
    runtime.active.retain(|_, active| active.course_id != course_id);
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

#[tauri::command]
pub fn reset_video_topic(
    state: tauri::State<'_, VideoTaskState>,
    topic_id: String,
) -> Result<(), String> {
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    conn.execute(
        "UPDATE video_courses SET status='pending',progress=0,last_error=NULL,updated_at=?2
         WHERE topic_id=?1 AND kind IN ('video', 'slides')",
        params![topic_id, now()],
    )
    .map_err(|error| error.to_string())?;
    conn.execute(
        "UPDATE video_topics SET progress=0,completed_count=0,last_synced_at=?2 WHERE id=?1",
        params![topic_id, now()],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn reset_video_course(
    state: tauri::State<'_, VideoTaskState>,
    course_id: String,
) -> Result<(), String> {
    let course = load_course(state.db_path.as_ref(), &course_id)?;
    let next_status = if course.kind == "video" || course.kind == "slides" {
        "pending"
    } else {
        "manual"
    };
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    conn.execute(
        "UPDATE video_courses SET status=?2,progress=0,last_error=NULL,updated_at=?3 WHERE id=?1",
        params![course_id, next_status, now()],
    )
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
    fn course_click_prefers_saved_locator_and_reuses_current_window() {
        let script = course_click_script("课程标题", "#saved-course");
        let locator_index = script.find("byLocator").expect("locator lookup exists");
        let title_index = script.find("byTitle").expect("title fallback exists");
        assert!(locator_index < title_index);
        assert!(script.contains("window.open = (url)"));
        assert!(script.contains("return window"));
        assert!(script.contains("#saved-course"));
    }

    #[test]
    fn capture_script_preserves_page_title_before_bridge_handshake() {
        let script = capture_script("request-1", Provider::Ulearn);
        let original_title_index = script
            .find("const originalTitle = document.title;")
            .expect("original page title is captured");
        let handshake_index = script
            .find("document.title = \"MTOOL_CAPTURE_START|\" + requestId;")
            .expect("capture handshake exists");

        assert!(original_title_index < handshake_index);
        assert!(script.contains("let docTitle = clean(originalTitle);"));
        assert_eq!(
            script
                .matches("const originalTitle = document.title;")
                .count(),
            1
        );
    }

    #[test]
    fn capture_script_ranks_visible_dom_topic_titles_before_hostname_fallback() {
        let script = capture_script("request-1", Provider::Ulearn);
        let chapter_title_index = script
            .find("document.querySelectorAll(\".chapterTitle\")")
            .expect("ulearn chapter title selector exists");
        let candidate_index = script
            .find("const directCandidates = Array.from")
            .expect("visible DOM title candidates are collected");
        let fallback_index = script
            .find("return location.hostname")
            .expect("hostname fallback exists");

        assert!(chapter_title_index < candidate_index);
        assert!(candidate_index < fallback_index);
        assert!(script.contains("element.getAttribute(\"title\") || element.innerText"));
        assert!(script.contains("occurrences.get(text)"));
        assert!(script.contains("topicMetaPatterns.filter"));
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
    fn capture_bridge_ignores_late_chunks_from_inactive_requests() {
        let captures = Arc::new(Mutex::new(CaptureExchange::default()));
        let runtime = Arc::new(Mutex::new(RuntimeState::default()));
        let title = "MTOOL_CAPTURE|expired|0|1|4|e30=";

        assert!(handle_bridge_title(
            title,
            Provider::Ulearn,
            &captures,
            &runtime,
        ));
        let exchange = captures.lock().unwrap_or_else(|error| error.into_inner());
        assert!(exchange.buffers.is_empty());
        assert!(exchange.completed.is_empty());
    }

    #[test]
    fn capture_bridge_tracks_start_and_completes_active_request() {
        let captures = Arc::new(Mutex::new(CaptureExchange::default()));
        let runtime = Arc::new(Mutex::new(RuntimeState::default()));
        let request_id = "active";
        let json = r#"{"title":"专题","url":"https://example.com/topic","progress":0,"totalCount":0,"completedCount":0,"courses":[]}"#;
        let encoded = STANDARD.encode(json.as_bytes());
        let split_at = encoded.len() / 2;
        let chunks = [&encoded[..split_at], &encoded[split_at..]];
        captures
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active_requests
            .insert(request_id.to_string());

        assert!(handle_bridge_title(
            &format!("{BRIDGE_CAPTURE_START_PREFIX}{request_id}"),
            Provider::Ulearn,
            &captures,
            &runtime,
        ));
        for (index, chunk) in chunks.iter().enumerate() {
            assert!(handle_bridge_title(
                &format!(
                    "{BRIDGE_CAPTURE_PREFIX}{request_id}|{index}|{}|{}|{chunk}",
                    chunks.len(),
                    encoded.len(),
                ),
                Provider::Ulearn,
                &captures,
                &runtime,
            ));
        }

        let mut exchange = captures.lock().unwrap_or_else(|error| error.into_inner());
        assert!(!exchange.active_requests.contains(request_id));
        assert!(!exchange.started_requests.contains(request_id));
        let capture = exchange
            .completed
            .remove(request_id)
            .expect("completed request exists")
            .expect("completed request decodes");
        assert_eq!(capture.title, "专题");
    }

    #[test]
    fn playback_bridge_integrates_full_autoplay_and_ui_triggers() {
        let bridge = bridge_script(Provider::Ulearn, 2.0, true);
        let update = update_media_script(2.0, true, true);

        assert!(bridge.contains("tryPlayMedia"));
        assert!(bridge.contains("triggerPlayUI"));
        assert!(bridge.contains("simulateFullClick"));
        assert!(bridge.contains("window.setInterval(() => apply(false), 1500)"));

        assert!(update.contains("window.__MTOOL_LEARNING_BRIDGE__.update(speed, muted, autoPlay)"));
    }

    #[test]
    fn media_progress_promotes_opening_course_to_playing() {
        let captures = Arc::new(Mutex::new(CaptureExchange::default()));
        let runtime = Arc::new(Mutex::new(RuntimeState::default()));
        runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .insert(
                "ulearn".to_string(),
                ActiveCourse {
                    course_id: "course-1".to_string(),
                    topic_id: "topic-1".to_string(),
                    provider: Provider::Ulearn,
                    phase: "opening".to_string(),
                    phase_since: now(),
                    last_media_at: now(),
                    last_progress_at: now(),
                    last_advanced_time: 0.0,
                    current_time: 0.0,
                    duration: 0.0,
                },
            );

        assert!(handle_bridge_title(
            "MTOOL_MEDIA|ulearn|timeupdate|15.5|900",
            Provider::Ulearn,
            &captures,
            &runtime,
        ));
        let state = runtime.lock().unwrap_or_else(|error| error.into_inner());
        let active = state.active.get("ulearn").expect("active course exists");
        assert_eq!(active.phase, "playing");
        assert_eq!(active.current_time, 15.5);
        assert_eq!(active.duration, 900.0);
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
        let domain = decode_obfuscated_url("bHpkeGVkdS5jb20=");
        let course = |id: &str, title: &str, kind: &str, completed: bool| PageCourseCapture {
            external_id: id.to_string(),
            title: title.to_string(),
            url: format!("https://{domain}/course/{id}"),
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
                url: format!("https://{domain}/study/test"),
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

    #[test]
    fn progress_stall_tracking_detects_stall() {
        let captures = Arc::new(Mutex::new(CaptureExchange::default()));
        let runtime = Arc::new(Mutex::new(RuntimeState::default()));
        let start_time = now() - 301;
        runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .insert(
                "ulearn".to_string(),
                ActiveCourse {
                    course_id: "course-stall".to_string(),
                    topic_id: "topic-1".to_string(),
                    provider: Provider::Ulearn,
                    phase: "playing".to_string(),
                    phase_since: start_time,
                    last_media_at: now(),
                    last_progress_at: start_time,
                    last_advanced_time: 0.0,
                    current_time: 0.0,
                    duration: 600.0,
                },
            );

        // Heartbeat with 0.0 progress arrives: should NOT update last_progress_at
        assert!(handle_bridge_title(
            "MTOOL_MEDIA|ulearn|timeupdate|0.0|600",
            Provider::Ulearn,
            &captures,
            &runtime,
        ));
        {
            let state = runtime.lock().unwrap_or_else(|error| error.into_inner());
            let active = state.active.get("ulearn").expect("active course exists");
            assert_eq!(active.last_progress_at, start_time);
            assert!(now() - active.last_progress_at >= 300);
        }

        // When progress actually advances, last_progress_at updates
        assert!(handle_bridge_title(
            "MTOOL_MEDIA|ulearn|timeupdate|2.5|600",
            Provider::Ulearn,
            &captures,
            &runtime,
        ));
        {
            let state = runtime.lock().unwrap_or_else(|error| error.into_inner());
            let active = state.active.get("ulearn").expect("active course exists");
            assert!(active.last_progress_at >= now() - 1);
            assert_eq!(active.current_time, 2.5);
        }
    }

    #[test]
    fn capture_script_filters_video_and_ppt_badges_and_prefers_catalog_panel() {
        let script = capture_script("test_req", Provider::Merchant);
        assert!(script.contains("视频|音频|图文|直播|ppt"));
        assert!(script.contains("parseCatalogCourses"));
        assert!(script.contains("scoreTitleCandidate"));
        assert!(script.contains("catalogCourses.length > 0"));
    }

    #[test]
    fn browser_nav_script_suppresses_autoplay_safely() {
        let script = browser_nav_script(Provider::Merchant);
        assert!(script.contains("__MTOOL_BROWSER_NAV_SHIELD__"));
        assert!(script.contains("window.addEventListener(\"play\""));
        assert!(script.contains("ensurePaused"));
        assert!(script.contains("__mtool_nav_toolbar__"));
        assert!(script.contains(&Provider::Merchant.home()));
    }

    #[test]
    fn capture_script_detects_course_kind_accurately_without_survey_false_positive() {
        let script = capture_script("test_req", Provider::Merchant);
        assert!(script.contains("detectCourseKind"));
        assert!(script.contains("软件测试|压力测试|接口测试"));
        assert!(script.contains("问卷|调查问卷|调研问卷|评价表"));
        assert!(script.contains("const itemKind = detectCourseKind(title, text, durationSeconds, item);"));
        assert!(script.contains("const kind = detectCourseKind(title, text, durationSeconds, container);"));
        assert!(script.contains("hasRealisticDuration = Number(durationSeconds) >= 120;"));
    }

    #[test]
    fn capture_script_prevents_group_container_from_swallowing_phase_sub_courses() {
        let script = capture_script("test_req", Provider::Merchant);
        assert!(script.contains("selfDurations > 1 || selfStatusCount > 1 || selfCredits > 1"));
        assert!(script.contains("parentDurations > 1 || parentStatusCount > 1 || parentCredits > 1"));
    }

    #[test]
    fn test_is_phase_or_section_title() {
        assert!(is_phase_or_section_title("01第一期：AI背景下的新型网络安全社工攻击"));
        assert!(is_phase_or_section_title("01 第一期：AI背景下的新型网络安全社工攻击"));
        assert!(is_phase_or_section_title("第一期：AI背景下的新型网络安全社工攻击"));
        assert!(is_phase_or_section_title("02第二期：数据安全新态势新要求"));
        assert!(is_phase_or_section_title("模块一：网络安全法解读"));
        assert!(is_phase_or_section_title("阶段1 基础知识"));

        assert!(!is_phase_or_section_title("AI背景下的新型网络安全社工攻击"));
        assert!(!is_phase_or_section_title("AI背景下的新型网络安全社工攻击培训-课件"));
        assert!(!is_phase_or_section_title("AI背景下的新型网络安全社工攻击考试"));
    }

    #[test]
    fn capture_script_filters_phase_headers_completely() {
        let script = capture_script("test_req", Provider::Merchant);
        assert!(script.contains("isPhaseOrSectionHeader"));
        assert!(script.contains("if (isPhaseOrSectionHeader(c.title)) return false;"));
    }
}

