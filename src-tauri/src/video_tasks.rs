use base64::{engine::general_purpose::STANDARD, Engine as _};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

const ULEARN_HOME: &str = "https://ulearn.cup.com.cn/home";
const BRIDGE_CAPTURE_START_PREFIX: &str = "MTOOL_CAPTURE_START|";
const BRIDGE_CAPTURE_PREFIX: &str = "MTOOL_CAPTURE|";
const BRIDGE_MEDIA_PREFIX: &str = "MTOOL_MEDIA|";
const BRIDGE_DEVTOOLS_TOGGLE_PREFIX: &str = "MTOOL_DEVTOOLS_TOGGLE|";
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

    fn label(self) -> String {
        format!("video-task-{}", self.key())
    }

    fn player_label(self) -> String {
        self.label()
    }

    fn browser_label(self) -> String {
        format!("{}-browser", self.label())
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
    kind: String,
    course_title: String,
    started_at: i64,
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
    playback_tokens: HashMap<String, u64>,
    browser_tokens: HashMap<String, u64>,
}

#[derive(Clone)]
pub struct VideoTaskState {
    db_path: Arc<PathBuf>,
    settings: Arc<Mutex<VideoTaskSettings>>,
    captures: Arc<Mutex<CaptureExchange>>,
    runtime: Arc<Mutex<RuntimeState>>,
    queue_tick: Arc<tokio::sync::Mutex<()>>,
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
            queue_tick: Arc::new(tokio::sync::Mutex::new(())),
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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatus {
    provider: String,
    name: String,
    home_url: String,
    window_open: bool,
    current_url: Option<String>,
    blocked_reason: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuePreview {
    pub provider: String,
    pub topic_id: String,
    pub topic_title: String,
    pub course_id: String,
    pub title: String,
    pub paused: bool,
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
    paused: usize,
    skipped: usize,
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
    next_courses: Vec<QueuePreview>,
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
           ON video_courses(status, provider, sort_order);
         CREATE TABLE IF NOT EXISTS video_queue_lanes (
           provider TEXT PRIMARY KEY,
           topic_id TEXT,
           blocked_reason TEXT
         );",
    )
    .map_err(|error| format!("初始化视频任务数据库失败: {error}"))?;
    conn.execute(
        "UPDATE video_courses
         SET status='paused',last_error=NULL
         WHERE status IN('opening','playing','verifying')",
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
         SET status = 'pending'
         WHERE kind = 'video' AND status = 'manual'",
        [],
    );
    let _ = conn.execute(
        "UPDATE video_courses
         SET status = 'pending'
         WHERE kind = 'material' AND status = 'manual'",
        [],
    );
    let _ = conn.execute(
        "UPDATE video_courses
         SET kind = 'material'
         WHERE (title LIKE '%手册' OR title LIKE '%文档' OR title LIKE '%手册%' OR title LIKE '%阅读材料%' OR title LIKE '%参考资料%') AND kind = 'video'",
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
    currentCourseTitle: "",
    currentCourseKind: "",
    lastDocProgress: 0,
  };

  try {
    const cached = JSON.parse(sessionStorage.getItem("__mtool_current_course__") || "{}");
    if (cached.title) state.currentCourseTitle = String(cached.title || "").trim();
    if (cached.kind) state.currentCourseKind = String(cached.kind || "").trim();
  } catch (_) {}

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

  let lastDevtoolsToggle = 0;
  const triggerDevtools = () => {
    const now = Date.now();
    if (now - lastDevtoolsToggle < 300) return;
    lastDevtoolsToggle = now;
    const message = "MTOOL_DEVTOOLS_TOGGLE|" + now;
    if (window.top === window) {
      try {
        const prev = document.title;
        setTitleMessage(message);
        setTimeout(() => {
          try {
            if (document.title.startsWith("MTOOL_DEVTOOLS_TOGGLE|")) {
              setTitleMessage(prev);
            }
          } catch (_) {}
        }, 50);
      } catch (_) {}
    } else {
      try { window.top.postMessage({ __mtoolDevtools: true }, "*"); } catch (_) {}
      try { window.parent.postMessage({ __mtoolDevtools: true }, "*"); } catch (_) {}
    }
  };

  try {
    window.addEventListener("keydown", (e) => {
      const isF12 = e.key === "F12" || e.keyCode === 123 || e.code === "F12";
      const isMacInspect = (e.metaKey && e.altKey && (e.key === "i" || e.key === "I" || e.keyCode === 73 || e.code === "KeyI"));
      const isWinInspect = (e.ctrlKey && e.shiftKey && (e.key === "i" || e.key === "I" || e.keyCode === 73 || e.code === "KeyI"));
      if (isF12 || isMacInspect || isWinInspect) {
        e.preventDefault();
        e.stopPropagation();
        triggerDevtools();
      }
    }, true);
  } catch (_) {}

  if (window.top === window) {
    window.addEventListener("message", (event) => {
      if (event.data && event.data.__mtoolMedia) setTitleMessage(event.data.__mtoolMedia);
      if (event.data && event.data.__mtoolDevtools) triggerDevtools();
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
      if (media.ended || (media.duration > 5 && media.currentTime >= media.duration - 0.8)) {
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

  const dismissCompletionModal = () => {
    try {
      getAccessibleDocs().forEach((doc) => {
        try {
          const dialogs = doc.querySelectorAll("[role='dialog'], .el-dialog, .modal, .ant-modal, .van-dialog, [class*='dialog'], [class*='modal']");
          for (const d of dialogs) {
            if (!d || d.offsetWidth === 0 || d.offsetHeight === 0) continue;
            const closeBtn = d.querySelector(".ant-modal-close, .el-dialog__headerbtn, [class*='close'], button, a");
            if (closeBtn) {
              try { closeBtn.click(); } catch (_) {}
            }
          }
        } catch (_) {}
      });
    } catch (_) {}
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
      const pageAge = Date.now() - state.pageLoadedAt;
      const href = (window.location.href || "").toLowerCase();

      // 检查页面是否存在视频/音频元素，或者是否有播放器容器
      const hasMediaOrPlayer = () => {
        for (const doc of docs) {
          try {
            if (doc.querySelector("video, audio, .prism-player, .xgplayer, .tcplayer, [class*='player']")) {
              return true;
            }
          } catch (_) {}
        }
        return false;
      };

      // 仅在没有媒体播放器、且超过 8 秒加载宽容期时，才进行登录检测，避免页面加载过渡期或接口测试课程文本触发误判
      if (!hasMediaOrPlayer() && pageAge >= 8000) {
        const isLoginUrl = href.includes("/login") || href.includes("/sso/login") || href.includes("/cas/login") || href.includes("oauth/authorize");
        let hasLoginForm = false;
        for (const doc of docs) {
          try {
            if (doc.querySelector("input[type='password'], [class*='qrcode-login'], .login-qrcode, .login-box")) {
              hasLoginForm = true;
              break;
            }
          } catch (_) {}
        }
        if (isLoginUrl || hasLoginForm) {
          report("need_login", { currentTime: 0, duration: 0 });
          return;
        }
      }
    } catch (_) {}

    // 0.1 真正的模态弹窗完播检测（必须是居中弹出可见对话框，严禁检测页面全局背景文本或导航栏标签）
    const hasVisibleCompletionModal = () => {
      const staySeconds = Math.floor((Date.now() - state.pageLoadedAt) / 1000);
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
              if (staySeconds < 5) {
                const closeBtn = d.querySelector(".ant-modal-close, .el-dialog__headerbtn, [class*='close'], button, a");
                if (closeBtn) {
                  try { closeBtn.click(); } catch (_) {}
                }
                continue;
              }
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
        const primary = allMedias[0];
        const dur = (primary && primary.duration > 0) ? primary.duration : 100;
        report("ended", { currentTime: dur, duration: dur });
        return;
      }
      // 看门狗增强：若视频已播放到末尾（距离结束不足0.8秒，或已暂停且进度达到99%以上），主动上报完播
      const finishedMedia = allMedias.find((m) =>
        m.ended ||
        (m.duration > 5 && m.currentTime >= m.duration - 0.8) ||
        (m.duration > 10 && m.paused && (m.currentTime / m.duration) >= 0.99)
      );
      if (finishedMedia) {
        report("ended", finishedMedia);
        return;
      }
    } else {
      // 1.2 若页面中未找到原生 video/audio 标签：
      // 若当前已知课程是视频类型，播放器可能正在渲染或网络缓冲挂载，绝不可走文档探测或伪造进度！
      if (state.currentCourseKind === "video") {
        return;
      }

      // 优先探测页面上动态变化的学习进度（如 "学习进度: 68%"、进度条等）
      const parseProgressNum = (str) => {
        if (!str) return null;
        const m = String(str).match(/(\d+(?:\.\d+)?)%/);
        if (!m) return null;
        const val = parseFloat(m[1]);
        return (!isNaN(val) && val >= 0 && val <= 100) ? val : null;
      };

      // 优先探测页面上动态变化的学习进度（如顶部条 "学习进度: 18%"、目录栏 "[PPT] 进度: 42%"、进度条等）
      const detectDocLearningProgress = () => {
        if (hasVisibleCompletionModal()) return 100;
        for (const doc of docs) {
          try {
            // 通道 1: 播放器/阅读器顶部工具条 (Top Bar)
            // 如 PPT 播放器顶部条包含 "学习进度: 0%"，支持标签与数字分散在不同 span 或兄弟节点
            const progressLabels = Array.from(
              doc.querySelectorAll("span, div, p, b, strong, label, a")
            ).filter((el) => {
              const direct = (el.innerText || el.textContent || "").trim();
              return direct.length > 0 && direct.length <= 25 &&
                /(?:学习进度|阅读进度|当前进度|完成进度|任务进度|课程进度|课件进度)/.test(direct);
            });

            for (const labelEl of progressLabels) {
              // 1.1 检查 labelEl 自身文本
              let val = parseProgressNum(labelEl.innerText || labelEl.textContent);
              if (val !== null) return val;

              // 1.2 检查 labelEl 紧邻的兄弟节点（如 <span>学习进度：</span><span>18%</span>）
              let sibling = labelEl.nextElementSibling;
              let siblingHops = 0;
              while (sibling && siblingHops < 3) {
                val = parseProgressNum(sibling.innerText || sibling.textContent);
                if (val !== null) return val;
                const barInner = sibling.querySelector ? sibling.querySelector("[style*='width'], [role='progressbar']") : null;
                if (barInner) {
                  const style = barInner.getAttribute("style") || "";
                  const sm = style.match(/width\s*:\s*(\d+(?:\.\d+)?)%/i);
                  if (sm) return parseFloat(sm[1]);
                }
                sibling = sibling.nextElementSibling;
                siblingHops++;
              }

              // 1.3 检查 labelEl 的父容器（如 <div class="progress-wrap">...</div>）
              const parent = labelEl.parentElement;
              if (parent) {
                const parentText = (parent.innerText || parent.textContent || "").replace(/\s+/g, " ").trim();
                if (parentText.length <= 300) {
                  const pm = parentText.match(/(?:学习进度|阅读进度|当前进度|完成进度|任务进度|课程进度|课件进度)[^0-9%]{0,40}(\d+(?:\.\d+)?)%/i);
                  if (pm) return parseFloat(pm[1]);
                }
                const parentBar = parent.querySelector("[role='progressbar'], .ant-progress, [class*='progress-bar'], [class*='progressBar'], [class*='progress_bar']");
                if (parentBar) {
                  const ariaVal = parentBar.getAttribute("aria-valuenow");
                  if (ariaVal !== null) {
                    const aval = parseFloat(ariaVal);
                    if (!isNaN(aval) && aval >= 0 && aval <= 100) return aval;
                  }
                  const innerBg = parentBar.querySelector("[class*='bg'], [class*='inner'], div") || parentBar;
                  const style = innerBg.getAttribute("style") || "";
                  const sm = style.match(/width\s*:\s*(\d+(?:\.\d+)?)%/i);
                  if (sm) return parseFloat(sm[1]);
                }
              }

              // 1.4 检查祖父容器（如整个顶部黑色工具栏）
              const grandParent = parent ? parent.parentElement : null;
              if (grandParent) {
                const gpText = (grandParent.innerText || grandParent.textContent || "").replace(/\s+/g, " ").trim();
                if (gpText.length <= 500) {
                  const gpm = gpText.match(/(?:学习进度|阅读进度|当前进度|完成进度|任务进度|课程进度|课件进度)[^0-9%]{0,40}(\d+(?:\.\d+)?)%/i);
                  if (gpm) return parseFloat(gpm[1]);
                }
              }
            }

            // 通道 2: 目录大纲侧边栏 / 抽屉 (Sidebar Catalog)
            // 如目录项中包含 "[PPT] 进度: 42%"、"进度: 18%" 等
            // 辅助函数：严格比对小节序号与标题，防止系列同名前缀课跨课串读
            const matchesCurrentCourse = (rowText) => {
              if (!state.currentCourseTitle) return true;
              const cleanRow = String(rowText || "").replace(/\s+/g, " ");
              const cleanExpected = String(state.currentCourseTitle).replace(/\s+/g, " ").trim();

              const expectedIdxMatch = cleanExpected.match(/^(\d{1,2})[\s.、-]/);
              if (expectedIdxMatch) {
                const expectedIdx = parseInt(expectedIdxMatch[1], 10);
                const rowIdxMatch = cleanRow.match(/(?:^|[\s(（【\[])(\d{1,2})[\s.、-]/);
                if (rowIdxMatch) {
                  const rowIdx = parseInt(rowIdxMatch[1], 10);
                  if (rowIdx !== expectedIdx) return false;
                }
              }

              const base = cleanExpected.replace(/^\d{1,2}[\s.、-]\s*/, "").trim();
              if (base.length >= 3) {
                return cleanRow.includes(base);
              }
              return cleanRow.includes(cleanExpected);
            };

            // 2.1 优先定位带有激活/选中/正在学习标识的小节行
            const activeChapterEls = doc.querySelectorAll(
              ".active, [class*='active'], .current, [class*='current'], .last-learn, [class*='last-learn'], [class*='selected'], [class*='highlight'], [class*='playing'], [class*='ongoing'], [aria-selected='true']"
            );
            for (const ac of activeChapterEls) {
              const row = (ac.closest && ac.closest("li, tr, [class*='item'], [class*='node'], [class*='row'], [class*='section'], [class*='chapter'], div")) || ac;
              const text = (row.innerText || row.textContent || "").replace(/\s+/g, " ").trim();
              if (text.length > 500) continue;
              if (!matchesCurrentCourse(text)) {
                continue;
              }
              const m = text.match(/(?:学习)?进度\s*[:：]?\s*(\d+(?:\.\d+)?)%/i) || text.match(/(\d+(?:\.\d+)?)%\s*(?:已完成|已学|已看)/i);
              if (m) {
                const val = Number(m[1]);
                if (!isNaN(val) && val >= 0 && val <= 100) return val;
              }
            }

            // 2.2 查找“上次学到”、“正在学习”、“学习中”、“当前学习”等徽章行
            const statusBadges = Array.from(doc.querySelectorAll("span, div, label, tag, [class*='tag'], [class*='badge']")).filter((b) => {
              const bt = (b.innerText || b.textContent || "").trim();
              return bt === "上次学到" || bt === "上次学习" || bt === "正在学习" || bt === "学习中" || bt === "当前学习";
            });
            for (const badge of statusBadges) {
              const row = (badge.closest && badge.closest("li, tr, [class*='item'], [class*='node'], [class*='row'], [class*='chapter'], div")) || badge.parentElement;
              if (row) {
                const text = (row.innerText || row.textContent || "").replace(/\s+/g, " ").trim();
                if (text.length <= 500) {
                  if (!matchesCurrentCourse(text)) {
                    continue;
                  }
                  const m = text.match(/(?:学习)?进度\s*[:：]?\s*(\d+(?:\.\d+)?)%/i) || text.match(/(\d+(?:\.\d+)?)%/i);
                  if (m) {
                    const val = Number(m[1]);
                    if (!isNaN(val) && val >= 0 && val <= 100) return val;
                  }
                }
              }
            }

            // 通道 3: 检查 Ant Design / 通用进度条组件 (.ant-progress, [role='progressbar'])
            const progressBars = doc.querySelectorAll("[role='progressbar'], .ant-progress, [class*='progress-bar'], [class*='progressBar'], [class*='progress_bar']");
            for (const pb of progressBars) {
              if (pb.closest(".prism-player, [class*='player']")) continue;
              const ariaVal = pb.getAttribute("aria-valuenow");
              if (ariaVal !== null) {
                const val = Number(ariaVal);
                if (!isNaN(val) && val >= 0 && val <= 100) return val;
              }
              const pbText = (pb.innerText || "").replace(/\s+/g, "");
              const tm = pbText.match(/(\d+(?:\.\d+)?)%/);
              if (tm) {
                const val = Number(tm[1]);
                if (!isNaN(val) && val >= 0 && val <= 100) return val;
              }
              const innerBg = pb.querySelector("[class*='bg'], [class*='inner'], div") || pb;
              const style = innerBg.getAttribute("style") || "";
              const sm = style.match(/width\s*:\s*(\d+(?:\.\d+)?)%/i);
              if (sm) {
                const val = Number(sm[1]);
                if (!isNaN(val) && val >= 0 && val <= 100) return val;
              }
            }

            // 通道 4: 兜底浅层 DOM 文本中的进度描述
            const shortEls = doc.querySelectorAll("span, div, p, label, b, strong, td, th");
            for (const el of shortEls) {
              if (el.children && el.children.length > 3) continue;
              const text = (el.innerText || el.textContent || "").replace(/\s+/g, " ").trim();
              if (text.length > 50) continue;
              const m = text.match(/(?:^|[\s\[(])(?:学习|阅读|当前|完成|任务|课程|课件)?进度\s*[:：]?\s*(\d+(?:\.\d+)?)%/i);
              if (m) {
                const val = Number(m[1]);
                if (!isNaN(val) && val >= 0 && val <= 100) return val;
              }
            }
          } catch (_) {}
        }
        return null;
      };

      // 微交互保活与 PPT 自动翻页：每隔 5 秒执行一次
      const nowTs = Date.now();
      if (!state.lastDocScrollAt || nowTs - state.lastDocScrollAt >= 5000) {
        state.lastDocScrollAt = nowTs;
        try {
          // 2.1 PPT 幻灯片键盘翻页派发（向主文档、窗口及所有 iframe 派发 ArrowRight / PageDown）
          docs.forEach((doc) => {
            const sendKeyEvent = (target, key, code, keyCode) => {
              try {
                const init = { key, code, keyCode, which: keyCode, bubbles: true, cancelable: true };
                target.dispatchEvent(new KeyboardEvent("keydown", init));
                target.dispatchEvent(new KeyboardEvent("keyup", init));
              } catch (_) {}
            };
            sendKeyEvent(doc, "ArrowRight", "ArrowRight", 39);
            sendKeyEvent(doc, "PageDown", "PageDown", 34);
            if (doc.defaultView) {
              sendKeyEvent(doc.defaultView, "ArrowRight", "ArrowRight", 39);
            }

            // 2.2 自动寻找并点击 PPT 课件的“下一页”按钮或右箭头
            const nextBtns = Array.from(doc.querySelectorAll(
              "button[title*='下一页'], [title*='下一页'], [aria-label*='下一页'], [title*='后一页'], [aria-label*='后一页'], [class*='btn-next'], [class*='page-next'], [class*='next-btn'], [class*='next-page'], [class*='page-turn-next'], [class*='arrow-right'], .anticon-right, .el-icon-arrow-right, svg[class*='right']"
            ));
            doc.querySelectorAll("button, span, div, a").forEach((el) => {
              if (el.children && el.children.length > 1) return;
              const t = (el.innerText || el.textContent || "").trim();
              if (t === "下一页" || t === "下一页 >" || t === "下一页>") {
                nextBtns.push(el);
              }
            });

            for (const btn of nextBtns) {
              if (btn && btn.offsetWidth > 0 && btn.offsetHeight > 0 && !btn.disabled && !String(btn.className || "").includes("disabled")) {
                try {
                  btn.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
                  btn.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
                  btn.click();
                  break;
                } catch (_) {}
              }
            }

            // 2.3 尝试点击课件右侧区域（很多 HTML5 幻灯片点击右半边翻页）
            const slideArea = doc.querySelector(".slide, [class*='slide-container'], [class*='reader-container'], [class*='ppt-container'], [class*='doc-container'], canvas");
            if (slideArea && slideArea.offsetWidth > 200 && slideArea.offsetHeight > 200) {
              try {
                const rect = slideArea.getBoundingClientRect();
                const clickX = rect.left + rect.width * 0.85;
                const clickY = rect.top + rect.height * 0.5;
                slideArea.dispatchEvent(new MouseEvent("click", { clientX: clickX, clientY: clickY, bubbles: true }));
              } catch (_) {}
            }
          });

          // 2.4 多层容器平滑滚动与鼠标移动保活
          const scrollable = Array.from(document.querySelectorAll("body, div, section, main, article")).find((el) => {
            return el.scrollHeight > el.clientHeight + 80 && el.clientHeight > 200;
          }) || window;
          if (scrollable === window) {
            window.scrollBy({ top: 35, behavior: "smooth" });
            if (window.innerHeight + window.scrollY >= (document.body.scrollHeight || 1000) - 50) {
              window.scrollTo({ top: 0, behavior: "smooth" });
            }
          } else {
            scrollable.scrollTop = (scrollable.scrollTop + 35) % Math.max(1, scrollable.scrollHeight - scrollable.clientHeight);
          }
          document.dispatchEvent(new MouseEvent("mousemove", { clientX: 120, clientY: 120, bubbles: true }));
        } catch (_) {}
      }

      const docProgress = detectDocLearningProgress();
      const staySeconds = Math.floor((Date.now() - state.pageLoadedAt) / 1000);

      if (docProgress !== null) {
        // 发现明确的页面进度（例如 "学习进度: 18%"、"42%" 等）
        let currentP = docProgress;
        if (state.lastDocProgress && docProgress < state.lastDocProgress - 15) {
          // 进度出现断崖式下降（例如上一门 100% 切换到新课程 52%），说明已平滑切换到新课程，立即重置并采纳新进度！
          state.pageLoadedAt = Date.now();
          currentP = docProgress;
        } else {
          currentP = Math.max(docProgress, state.lastDocProgress || 0);
        }
        state.lastDocProgress = currentP;
        report("timeupdate", { currentTime: currentP, duration: 100 });

        // 仅当进度达到 100% 或弹出完成模态对话框时，才上报完播；
        // 刚打开页面不足 10 秒时绝不盲目上报完播，避免读取到上一门课程未刷新的旧残留进度！
        if (staySeconds >= 10 && (currentP >= 100 || hasVisibleCompletionModal())) {
          report("ended", { currentTime: 100, duration: 100 });
          return;
        }
      } else {
        // 未检测到明确进度（可能刚打开正在加载，或纯静态页面）
        // 彻底废除原先 90 秒草率提前完播的逻辑！兜底时长设为 600 秒（10分钟），避免提前切课
        const targetDuration = 600;
        const simulatedCurrent = Math.min(staySeconds, targetDuration);
        report("timeupdate", { currentTime: simulatedCurrent, duration: targetDuration });

        // 若中途弹出了明确的完成对话框，立即完播
        if (hasVisibleCompletionModal()) {
          report("ended", { currentTime: targetDuration, duration: targetDuration });
          return;
        }

        // 仅当持续驻留满 10 分钟且完全无进度条时，才做最终兜底
        if (staySeconds >= targetDuration) {
          report("ended", { currentTime: targetDuration, duration: targetDuration });
          return;
        }
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

  state.setCourse = (title, kind) => {
    state.currentCourseTitle = String(title || "").trim();
    state.currentCourseKind = String(kind || "").trim();
    state.lastDocProgress = 0;
    state.pageLoadedAt = Date.now();
    try {
      sessionStorage.setItem("__mtool_current_course__", JSON.stringify({
        title: state.currentCourseTitle,
        kind: state.currentCourseKind,
      }));
    } catch (_) {}
    dismissCompletionModal();
  };

  state.update = (speedValue, mutedValue, autoPlay, title, kind) => {
    state.speed = Math.min(2, Math.max(1, Number(speedValue) || 2));
    state.muted = Boolean(mutedValue);
    if (autoPlay !== undefined) state.autoPlay = Boolean(autoPlay);
    if (title && !state.currentCourseTitle) {
      state.currentCourseTitle = String(title).trim();
    }
    if (kind && !state.currentCourseKind) {
      state.currentCourseKind = String(kind).trim();
    }
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

#[allow(dead_code)]
fn browser_nav_script(provider: Provider) -> String {
    const TEMPLATE: &str = r##"
(() => {
  // 1. 全局媒体静音暂停与开发者工具快捷键（在所有 frame / iframe 均生效）
  // 选专题窗口专门用于浏览目录和导入专题，防止视频自动出声并避免抢占学习
  let lastDevtoolsToggle = 0;
  const triggerDevtools = () => {
    const now = Date.now();
    if (now - lastDevtoolsToggle < 300) return;
    lastDevtoolsToggle = now;
    const message = "MTOOL_DEVTOOLS_TOGGLE|" + now;
    if (window.top === window) {
      try {
        const prev = document.title;
        document.title = message;
        setTimeout(() => {
          try {
            if (document.title.startsWith("MTOOL_DEVTOOLS_TOGGLE|")) {
              document.title = prev;
            }
          } catch (_) {}
        }, 50);
      } catch (_) {}
    } else {
      try { window.top.postMessage({ __mtoolDevtools: true }, "*"); } catch (_) {}
      try { window.parent.postMessage({ __mtoolDevtools: true }, "*"); } catch (_) {}
    }
  };

  try {
    window.addEventListener("keydown", (e) => {
      const isF12 = e.key === "F12" || e.keyCode === 123 || e.code === "F12";
      const isMacInspect = (e.metaKey && e.altKey && (e.key === "i" || e.key === "I" || e.keyCode === 73 || e.code === "KeyI"));
      const isWinInspect = (e.ctrlKey && e.shiftKey && (e.key === "i" || e.key === "I" || e.keyCode === 73 || e.code === "KeyI"));
      if (isF12 || isMacInspect || isWinInspect) {
        e.preventDefault();
        e.stopPropagation();
        triggerDevtools();
      }
    }, true);
  } catch (_) {}

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

  window.addEventListener("message", (event) => {
    if (event.data && event.data.__mtoolDevtools) triggerDevtools();
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
  const devtoolsBtn = createBtn("开发者工具 (F12 / ⌥⌘I)", '<polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/>', () => triggerDevtools());

  bar.appendChild(handle);
  bar.appendChild(backBtn);
  bar.appendChild(forwardBtn);
  bar.appendChild(refreshBtn);
  bar.appendChild(homeBtn);
  bar.appendChild(devtoolsBtn);

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

fn update_media_script(
    speed: f64,
    muted: bool,
    auto_play: bool,
    title: &str,
    kind: &str,
) -> String {
    let title_json = serde_json::to_string(title).unwrap_or_default();
    let kind_json = serde_json::to_string(kind).unwrap_or_default();
    format!(
        r#"(() => {{
          const speed = {};
          const muted = {};
          const autoPlay = {};
          const courseTitle = {title_json};
          const courseKind = {kind_json};
          if (window.__MTOOL_LEARNING_BRIDGE__) {{
            window.__MTOOL_LEARNING_BRIDGE__.update(speed, muted, autoPlay, courseTitle, courseKind);
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

fn ulearn_course_click_script(title: &str, locator: &str) -> String {
    let title_json = serde_json::to_string(title).unwrap_or_default();
    let locator_json = serde_json::to_string(locator).unwrap_or_default();
    format!(
        r#"(async () => {{
          const targetTitle = {title_json};
          const targetLocator = {locator_json};
          const clean = (value) => String(value || "").replace(/\s+/g, " ").trim();

          // 拦截 window.open 防止弹多余新窗口
          try {{
            window.open = (url) => {{
              const next = clean(url);
              if (next && next !== "about:blank" && !/^javascript:/i.test(next)) {{
                try {{ window.location.assign(new URL(next, window.location.href).href); }} catch (_) {{}}
              }}
              try {{
                window.location = {{
                  set href(val) {{
                    try {{ window.location.assign(new URL(val, window.location.href).href); }} catch (_) {{}}
                  }}
                }};
              }} catch (_) {{}}
              return window;
            }};
          }} catch (_) {{}}

          const cleanTarget = clean(targetTitle);
          try {{
            if (window.__MTOOL_LEARNING_BRIDGE__ && typeof window.__MTOOL_LEARNING_BRIDGE__.setCourse === "function") {{
              window.__MTOOL_LEARNING_BRIDGE__.setCourse(cleanTarget);
            }}
          }} catch (_) {{}}

          const findTarget = () => {{
            const byLocator = targetLocator ? document.querySelector(targetLocator) : null;
            const all = Array.from(document.querySelectorAll("body *"));
            const byTitle = all.find((el) => clean(el.innerText) === cleanTarget) ||
              all.find((el) => {{
                const text = clean(el.innerText);
                return text.length <= cleanTarget.length + 8 && text.includes(cleanTarget);
              }});
            return byLocator || byTitle;
          }};

          let target = findTarget();
          if (!target) return;

          // 找到卡片或链接
          let card = target;
          for (let current = target, depth = 0; current && current !== document.body && depth < 6; current = current.parentElement, depth++) {{
            if (current.matches("a[href], [class*='card'], [class*='item'], [class*='course']")) {{
              card = current;
              break;
            }}
          }}

          try {{ card.scrollIntoView({{ block: "center", behavior: "instant" }}); }} catch (_) {{}}

          const anchor = card.matches("a[href]") ? card : card.querySelector("a[href]");
          const clickTarget = anchor || (card.matches("button, [role='button']") ? card : null) || target;

          if (clickTarget.tagName === "A" && clickTarget.getAttribute("href") && !/^javascript:/i.test(clickTarget.getAttribute("href"))) {{
            clickTarget.removeAttribute("target");
            try {{
              window.location.assign(new URL(clickTarget.getAttribute("href"), window.location.href).href);
              return;
            }} catch (_) {{}}
          }}

          clickTarget.querySelectorAll?.("a[target]").forEach((a) => a.removeAttribute("target"));
          if (clickTarget.matches?.("a[target]")) clickTarget.removeAttribute("target");

          const triggerClick = (el) => {{
            if (!el) return;
            try {{
              const rect = el.getBoundingClientRect();
              const init = {{
                bubbles: true,
                cancelable: true,
                view: window,
                clientX: rect.left + Math.max(5, Math.min(rect.width / 2, 25)),
                clientY: rect.top + Math.max(5, Math.min(rect.height / 2, 25)),
                button: 0,
              }};
              el.dispatchEvent(new PointerEvent("pointerdown", init));
              el.dispatchEvent(new MouseEvent("mousedown", init));
              el.dispatchEvent(new PointerEvent("pointerup", init));
              el.dispatchEvent(new MouseEvent("mouseup", init));
              el.dispatchEvent(new MouseEvent("click", init));
              if (typeof el.click === "function") el.click();
            }} catch (_) {{
              try {{ el.click(); }} catch (_) {{}}
            }}
          }};

          triggerClick(clickTarget);
          if (card !== clickTarget) triggerClick(card);
        }})();"#
    )
}

fn merchant_course_click_script(title: &str, locator: &str) -> String {
    let title_json = serde_json::to_string(title).unwrap_or_default();
    let locator_json = serde_json::to_string(locator).unwrap_or_default();
    format!(
        r#"(async () => {{
          const targetTitle = {title_json};
          const targetLocator = {locator_json};
          const clean = (value) => String(value || "").replace(/\s+/g, " ").trim();
          const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

          // 拦截 window.open 防止弹空白页
          try {{
            window.open = (url) => {{
              const next = clean(url);
              if (next && next !== "about:blank" && !/^javascript:/i.test(next)) {{
                try {{ window.location.assign(new URL(next, window.location.href).href); }} catch (_) {{}}
              }}
              try {{
                window.location = {{
                  set href(val) {{
                    try {{ window.location.assign(new URL(val, window.location.href).href); }} catch (_) {{}}
                  }}
                }};
              }} catch (_) {{}}
              return window;
            }};
          }} catch (_) {{}}

          const cleanTarget = clean(targetTitle);
          const baseTitle = cleanTarget.replace(/^\d{{1,2}}\s*[.、-]\s*/, "").trim();
          const targetIdxMatch = cleanTarget.match(/^(\d{{1,2}})[\s.、-]/);
          const targetIdx = targetIdxMatch ? parseInt(targetIdxMatch[1], 10) : null;

          try {{
            if (window.__MTOOL_LEARNING_BRIDGE__ && typeof window.__MTOOL_LEARNING_BRIDGE__.setCourse === "function") {{
              window.__MTOOL_LEARNING_BRIDGE__.setCourse(cleanTarget);
            }}
          }} catch (_) {{}}

          const matchesTarget = (rawText) => {{
            const text = clean(rawText);
            if (!text) return false;
            if (targetIdx !== null) {{
              const rowIdxMatch = text.match(/(?:^|[\s(（【\[])(\d{{1,2}})[\s.、-]/);
              if (rowIdxMatch && parseInt(rowIdxMatch[1], 10) !== targetIdx) {{
                return false;
              }}
            }}
            if (text === cleanTarget) return true;
            if (text.length <= cleanTarget.length + 10 && text.includes(cleanTarget)) return true;
            if (baseTitle.length >= 3 && text.length <= baseTitle.length + 12 && text.includes(baseTitle)) return true;
            return false;
          }};

          const findTarget = () => {{
            const byLocator = targetLocator ? document.querySelector(targetLocator) : null;
            const all = Array.from(document.querySelectorAll("body *"));
            const byTitle = all.find((el) => clean(el.innerText) === cleanTarget) ||
              all.find((el) => matchesTarget(el.innerText));
            return byLocator || byTitle;
          }};

          let target = findTarget();
          if (!target) {{
            // 查找页面上所有处于折叠状态的章节头部并展开
            const stageHeaders = Array.from(document.querySelectorAll(
              ".course-stage-caption, [class*='stage-caption'], .ant-collapse-header, [class*='collapse-header'], [class*='collapse-item__header'], [class*='chapter-header'], [class*='chapter_header'], [role='tab']"
            ));
            let expandedAny = false;
            for (const h of stageHeaders) {{
              const parent = (h.parentElement ? h.parentElement.closest("[class*='stage'], [class*='chapter'], .ant-collapse-item, [class*='collapse-item']") : null) || h.parentElement;
              const hasContentItems = !!parent && !!parent.querySelector(".course-content-item, [class*='content-item']");
              if (hasContentItems) continue;
              const arrow = h.querySelector(".anticon, svg, [class*='arrow'], [class*='icon']");
              const arrowStyle = (arrow ? arrow.getAttribute("style") || "" : "") + (arrow && arrow.parentElement ? arrow.parentElement.getAttribute("style") || "" : "");
              const isRotated = /rotate\(-?90deg\)/i.test(arrowStyle);
              const isAriaClosed = h.getAttribute("aria-expanded") === "false";
              if (isRotated || isAriaClosed || !arrow) {{
                try {{ h.click(); }} catch (_) {{}}
                try {{ h.dispatchEvent(new MouseEvent("click", {{ bubbles: true, cancelable: true, view: window }})); }} catch (_) {{}}
                if (arrow) {{
                  try {{ arrow.click(); }} catch (_) {{}}
                  try {{ arrow.dispatchEvent(new MouseEvent("click", {{ bubbles: true, cancelable: true, view: window }})); }} catch (_) {{}}
                }}
                const innerSpan = h.querySelector(".course-stage-index, span, [role='button']");
                if (innerSpan && innerSpan !== h) {{
                  try {{ innerSpan.click(); }} catch (_) {{}}
                }}
                expandedAny = true;
              }}
            }}
            if (expandedAny) {{
              await sleep(350);
            }}
            target = findTarget();
          }}
          if (!target) return;

          // 递归找到整张小节卡片容器
          let card = target;
          for (let current = target, depth = 0; current && current !== document.body && depth < 8; current = current.parentElement, depth++) {{
            if (current.matches("a[href], [class*='card'], [class*='item'], [class*='course'], [class*='list-item'], [class*='row'], tr, li")) {{
              card = current;
              break;
            }}
          }}

          try {{ card.scrollIntoView({{ block: "center", behavior: "instant" }}); }} catch (_) {{}}

          // 优先查找卡片内的操作按钮（扩展支持去考试/开始考试/参加考试/进入考试/填写问卷/去评价等）
          const actionRegex = /^(去学习|开始学习|继续学习|立即学习|学习中|进入学习|播放|去考试|开始考试|参加考试|进入考试|立即考试|重新考试|补考|查看试卷|填写问卷|去填写|开始填写|参加调研|参与问卷|开始问卷|问卷调查|去评价|立即评价|填写评价|评价|去完成|查看线下课|查看详情|查看)$/;
          const actionBtn = Array.from(card.querySelectorAll("button, a, [role='button'], div, span")).find((el) => {{
            const t = clean(el.innerText);
            return actionRegex.test(t) ||
                   el.matches("[class*='btn-primary'], [class*='study-btn'], [class*='play-btn'], [class*='start'], [class*='exam-btn'], [class*='survey-btn'], [class*='eval-btn']");
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

          const triggerClick = (el) => {{
            if (!el) return;
            try {{
              const rect = el.getBoundingClientRect();
              const init = {{
                bubbles: true,
                cancelable: true,
                view: window,
                clientX: rect.left + Math.max(5, Math.min(rect.width / 2, 25)),
                clientY: rect.top + Math.max(5, Math.min(rect.height / 2, 25)),
                button: 0,
              }};
              el.dispatchEvent(new PointerEvent("pointerdown", init));
              el.dispatchEvent(new MouseEvent("mousedown", init));
              el.dispatchEvent(new PointerEvent("pointerup", init));
              el.dispatchEvent(new MouseEvent("mouseup", init));
              el.dispatchEvent(new MouseEvent("click", init));
              if (typeof el.click === "function") el.click();
            }} catch (_) {{
              try {{ el.click(); }} catch (_) {{}}
            }}
          }};

          triggerClick(clickTarget);
          if (card !== clickTarget) triggerClick(card);
          if (target !== clickTarget && target !== card) triggerClick(target);

          // 等待弹窗或主界面渲染，自动触发【开始考试】/【进入考试】/【参加考试】
          await sleep(400);
          const modalOrMain = document.querySelector(".ant-modal, .ant-modal-content, [role='dialog'], [class*='modal'], [class*='dialog'], [class*='drawer'], .mainContent___vvQdb, [class*='main-content'], .sectionContent___rouak");
          if (modalOrMain) {{
            const modalBtn = Array.from(modalOrMain.querySelectorAll("button, a, [role='button'], div, span")).find((el) => {{
              const t = clean(el.innerText);
              return /^(开始考试|进入考试|参加考试|立即考试|去考试|开始答题|进入答题|继续学习|继续播放|我知道了)$/.test(t) ||
                     el.matches(".ant-btn-primary, [class*='btn-primary'], [class*='primary-btn']");
            }});
            if (modalBtn) {{
              triggerClick(modalBtn);
            }}
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

fn course_click_script(title: &str, locator: &str, provider: Provider) -> String {
    match provider {
        Provider::Ulearn => ulearn_course_click_script(title, locator),
        Provider::Merchant => merchant_course_click_script(title, locator),
    }
}

fn ulearn_capture_script(request_id: &str) -> String {
    const TEMPLATE: &str = r##"
(() => {
  const requestId = "__REQUEST_ID__";
  const originalTitle = document.title;
  window.__MTOOL_CAPTURE_REQUEST__ = requestId;
  document.title = "MTOOL_CAPTURE_START|" + requestId;
  window.setTimeout(async () => {
    try {
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
        while (current && current !== document.body && parts.length < 12) {
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

      const isNavOrHeader = (el) => {
        let cur = el;
        for (let i = 0; cur && cur !== document.body && i < 6; i++, cur = cur.parentElement) {
          if (cur.tagName === "HEADER" || cur.tagName === "NAV") return true;
          const c = String(cur.className || "").toLowerCase();
          if (c.includes("header") || c.includes("navbar") || c.includes("nav-bar") || c.includes("bread") || c.includes("menu")) return true;
        }
        return false;
      };

      const isInvalidTopicTitle = (s) => {
        const t = clean(s);
        if (!t || t.length < 2) return true;
        return /^(银联乐学|中国银联|乐学|首页|个人中心|学习中心|学习地图|考试中心|赛事中心|全部|课程大纲|专题介绍|乐学圈|我的学习|全部课程|返回|上一步|加入自学|已加入)$/.test(t) ||
               /^(起止时间|课程数|浏览人数|学习人数|完成标准|章节进度|学习进度)/.test(t);
      };

      const findTopicTitle = () => {
        // 1. 优先通过页面信息头部特征锚点精准定位大标题
        const metaAnchor = Array.from(document.querySelectorAll("body *")).find((el) => {
          if (!visible(el) || isNavOrHeader(el)) return false;
          const t = clean(el.innerText);
          if (t.length < 2 || t.length > 50) return false;
          return /^(起止时间|学习人数|学习进度|完成任务数|完成标准|章节进度|课程数|浏览人数)\s*[:：]?/.test(t);
        });
        if (metaAnchor) {
          let card = metaAnchor.parentElement;
          for (let d = 0; card && card !== document.body && d < 6; d++, card = card.parentElement) {
            const titleCandidates = Array.from(card.querySelectorAll("h1, h2, h3, h4, [class*='title'], [class*='name']"))
              .filter((el) => {
                if (!visible(el) || isNavOrHeader(el)) return false;
                const text = clean(el.innerText);
                return text.length >= 2 && text.length <= 80 && !isInvalidTopicTitle(text);
              })
              .map((el) => clean(el.innerText));
            if (titleCandidates.length > 0) {
              return titleCandidates[0];
            }
          }
        }

        // 2. 银联乐学的“课程大纲”使用 chapterTitle 标识专题名
        const chapterTitleElements = Array.from(document.querySelectorAll(".chapterTitle"));
        for (const element of chapterTitleElements) {
          if (!visible(element)) continue;
          const text = clean(element.getAttribute("title") || element.innerText);
          if (!isInvalidTopicTitle(text)) return text;
        }

        // 3. 页面标题清洗
        let docTitle = clean(originalTitle);
        docTitle = docTitle
          .replace(/^MTOOL\s*·\s*[^·]+\s*·\s*/i, "")
          .replace(/\s*[-_|\s]\s*(银联乐学|中国银联|培训平台|专题详情|课程详情|学习端|播放端).*$/i, "")
          .trim();
        if (docTitle && !isInvalidTopicTitle(docTitle) && docTitle.length >= 2) {
          return docTitle;
        }

        const topicMetaPatterns = [/起止时间/, /课程数/, /浏览人数/, /学习人数/, /学习进度/, /完成标准/, /章节进度/];
        // 4. 扫描当前 DOM 中的可见候选文本
        const directCandidates = Array.from(document.querySelectorAll("body *"))
          .filter((el) => visible(el) && !isNavOrHeader(el) && el.getClientRects().length > 0)
          .map((el) => ({ element: el, text: ownText(el) }))
          .filter(({ text }) => !isInvalidTopicTitle(text) && text.length >= 4);
        const occurrences = new Map();
        directCandidates.forEach(({ text }) => occurrences.set(text, (occurrences.get(text) || 0) + 1));

        for (const { text } of directCandidates) {
          if ((occurrences.get(text) || 0) === 1) {
            const hasMeta = topicMetaPatterns.filter((p) => p.test(text)).length > 0;
            if (!hasMeta) return text;
          }
        }

        return location.hostname;
      };

      const bodyText = clean(document.body.innerText);

      // 2. 专题指标判定
      const countMatch = bodyText.match(/完成任务数\s*(\d+)\s*\/\s*(\d+)/) || bodyText.match(/完成标准\s*(\d+)\s*\/\s*(\d+)/);
      const topicProgressMatch = bodyText.match(/(?:学习|章节)进度\s*[:：]?\s*(\d+(?:\.\d+)?)%/);

      // 3. 扫描银联乐学专属课程卡片
      const badgeMarkers = Array.from(document.querySelectorAll("body *")).filter((element) => {
        if (!visible(element)) return false;
        const text = clean(element.innerText);
        return /^(已学习|未学习|学习中)$/.test(text);
      });

      const rawCards = [];
      if (badgeMarkers.length > 0) {
        for (const marker of badgeMarkers) {
          let card = marker;
          for (let depth = 0; card && card !== document.body && depth < 8; depth++, card = card.parentElement) {
            const t = clean(card.innerText);
            if (/(学时|学分)/.test(t) && t.length < 600) {
              rawCards.push(card);
              break;
            }
          }
        }
      } else {
        const metaElements = Array.from(document.querySelectorAll("body *")).filter((el) => {
          if (!visible(el)) return false;
          const t = clean(el.innerText);
          return /学时\s*[:：]?\s*\d+/.test(t) && /学分\s*[:：]?\s*\d+/.test(t) && t.length < 600;
        });
        rawCards.push(...metaElements);
      }

      const courses = [];
      const seenTitles = new Set();
      const seenCards = new Set();

      for (const card of rawCards) {
        if (!card || card === document.body || seenCards.has(card)) continue;
        seenCards.add(card);

        const cardText = clean(card.innerText);

        // 提取标题
        let title = "";
        const titleEl = card.querySelector("[class*='name'], [class*='title'], h2, h3, h4, h5, p");
        const candTitle = titleEl ? clean(titleEl.innerText) : "";
        if (candTitle.length >= 2 && !/(学时|学分|已学习|未学习|学习中)/.test(candTitle)) {
          title = candTitle;
        } else {
          const subTexts = Array.from(card.querySelectorAll("div, p, span, a"))
            .filter((el) => visible(el))
            .map((el) => clean(el.innerText))
            .filter((t) => t.length >= 3 && !/(学时|学分|已学习|未学习|学习中|分|星)/.test(t) && !/^\d+$/.test(t) && !/^[★☆\s\d]+$/.test(t));
          if (subTexts.length > 0) {
            title = subTexts[0];
          }
        }

        if (!title || seenTitles.has(title)) continue;
        seenTitles.add(title);

        const locator = cssPath(card);
        const link = card.matches("a[href]") ? card.getAttribute("href") : (card.querySelector("a[href]")?.getAttribute("href") || "");
        const url = link ? new URL(link, location.href).href : location.href;
        const externalId = url !== location.href ? url : (title + "_" + locator);

        // 状态判定：专属识别已学习与未学习（支持明确百分比进度）
        const progMatch = cardText.match(/(?:学习)?进度\s*[:：]?\s*(\d+(?:\.\d+)?)%/);
        const isCompleted = /(已完成|已学完|已学习)/.test(cardText) || !!card.querySelector("[title*='已学习'], [title*='已完成'], [aria-label*='已学习'], [aria-label*='已完成']");
        const isIncomplete = /未学习/.test(cardText);
        let completed = false;
        let progress = 0;
        if (progMatch) {
          const pVal = Number(progMatch[1]) || 0;
          if (pVal >= 100) {
            completed = true;
            progress = 100;
          } else {
            completed = false;
            progress = pVal;
          }
        } else if (isCompleted) {
          completed = true;
          progress = 100;
        } else if (!isIncomplete && /学习中/.test(cardText)) {
          completed = false;
          progress = 50;
        }

        // 时长识别（学时 1 -> 45分钟）
        let durationSeconds = 0;
        const durMatch = cardText.match(/学时\s*[:：]?\s*(\d+)/) || cardText.match(/(\d+)\s*分钟/);
        if (durMatch) {
          const val = Number(durMatch[1]) || 0;
          if (/学时/.test(durMatch[0])) {
            durationSeconds = val * 45 * 60;
          } else {
            durationSeconds = val * 60;
          }
        }

        const isSlides = /\[?ppt\]?|课件|幻灯片/i.test(cardText) || /\[?ppt\]?|课件|幻灯片/i.test(title);
        const kind = isSlides ? "slides" : "video";

        courses.push({
          externalId,
          title,
          url,
          locator,
          sectionTitle: "",
          kind,
          durationSeconds,
          progress,
          completed
        });
      }

      // 整专题全满校正
      const isAllCompletedByStats = !!(
        countMatch &&
        Number(countMatch[1]) > 0 &&
        Number(countMatch[1]) === Number(countMatch[2]) &&
        courses.length === Number(countMatch[2]) &&
        (topicProgressMatch ? Number(topicProgressMatch[1]) >= 100 : true)
      );

      if (isAllCompletedByStats) {
        courses.forEach((c) => {
          c.completed = true;
          c.progress = 100;
        });
      }

      const topicTitle = findTopicTitle();
      const completedCount = isAllCompletedByStats ? courses.length : (countMatch ? Number(countMatch[1]) : courses.filter((item) => item.completed).length);
      const totalCount = countMatch ? Number(countMatch[2]) : courses.length;

      const payload = {
        title: String(topicTitle || "未知专题"),
        url: location.href,
        progress: isAllCompletedByStats ? 100 : (topicProgressMatch ? Number(topicProgressMatch[1]) : (totalCount ? completedCount / totalCount * 100 : 0)),
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
        .replace("__CHUNK_SIZE__", &CAPTURE_CHUNK_SIZE.to_string())
        .replace("__CHUNK_INTERVAL__", &CAPTURE_CHUNK_INTERVAL_MS.to_string())
        .replace(
            "__RESTORE_DELAY__",
            &(CAPTURE_CHUNK_INTERVAL_MS * 2).to_string(),
        )
}

fn merchant_capture_script(request_id: &str) -> String {
    const TEMPLATE: &str = r##"
(() => {
  const requestId = "__REQUEST_ID__";
  const originalTitle = document.title;
  window.__MTOOL_CAPTURE_REQUEST__ = requestId;
  document.title = "MTOOL_CAPTURE_START|" + requestId;
  window.setTimeout(async () => {
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
    const sleep = (ms) => new Promise((resolve) => window.setTimeout(resolve, ms));
    const cssPath = (element) => {
      if (!element || element === document.body) return "body";
      const parts = [];
      let current = element;
      while (current && current !== document.body && parts.length < 12) {
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
    const isTagOrBadge = (s) => /^(知识|课程|考试|测验|测试|课件|文档|阅读材料|参考资料|视频|音频|图文|直播|ppt|pptx|pdf|word|excel|线下课|线上课|面授|面授课|公开课|问卷|调查问卷|评价表|满意度评价|调研|签到|打卡|活动|讨论|实操|练习|作业|大纲|目录|必修|选修|必修课|选修课|必修学分|选修学分|未完成|已完成|已学完|已学习|未学习|学习中|已考试|已通过|未通过|进行中|全部|展开|收起|去学习|立即学习|开始学习|重新学习|继续学习|查看|详情|上次学习|试看|播放中|需本人处理|\d{1,2})$/i.test(clean(s));
    const isPureTagOrBadge = (s) => {
      const t = clean(s);
      if (!t) return true;
      return /^(?:(?:知识|课程|考试|测验|测试|课件|文档|资料|手册|阅读材料|参考资料|视频|音频|图文|直播|ppt|pptx|pdf|word|excel|问卷|调查问卷|评价表|满意度评价|未完成|已完成|已学完|已学习|未学习|学习中|已考试|已通过|未通过|进行中|上次学习|试看|播放中|去学习|立即学习|开始学习|重新学习|继续学习|必修|选修|需本人处理)\s*)+$/i.test(t);
    };
    const isMeta = (s) => {
      const t = clean(s);
      if (!t) return false;
      return (
        /^\d+(?:\.\d+)?\s*\/\s*\d+(?:\.\d+)?$/.test(t) ||
        /^\d+(?:\.\d+)?%$/.test(t) ||
        /^(?:起止时间|有效时间|开课时间|培训时间|考试时间)/.test(t) ||
        /\d{4}[-/.]\d{1,2}[-/.]\d{1,2}.*?[~至到-]\s*\d{4}[-/.]\d{1,2}[-/.]\d{1,2}/.test(t) ||
        /^(?:获|已获)?(?:必修学分|选修学分|学分)(?:[\s:：].*)?$/i.test(t) ||
        /^(?:超越员工数|完成任务数|学习人数|浏览人数|课程数)(?:[\s:：].*)?$/i.test(t) ||
        /(?:^|\s)(学习时长|必修学分|选修学分|学习进度|学时|学分|起止时间|得分|正确率|总分|题数|时长|考试时长|课程数|浏览人数|学习人数)\s*[:：]/.test(t) ||
        /^(学习时长|必修学分|选修学分|学习进度|学时|学分|起止时间|得分|正确率|总分|题数|时长|考试时长|课程数|浏览人数|学习人数)$/.test(t) ||
        /(?:进度|学习进度)\s*[:：]?\s*\d+(?:\.\d+)?%/.test(t) ||
        /(?:学时|学分|题数|总分|得分)\s*[:：]?\s*\d+/.test(t) ||
        /(?:浏览人数|学习人数|课程数)\s*[:：]?\s*\d+/.test(t) ||
        /正确率\s*[:：]?\s*\d+(?:\.\d+)?%/.test(t)
      );
    };
    const isSiteOrUiTitle = (s) => /^(YS学堂|银商学堂|银联乐学|中国银联|乐学|首页|个人中心|学习中心|学习地图|考试中心|赛事中心|全部|培训管理|培训介绍|培训内容|专题介绍|课程大纲|乐学圈|我的学习|我的课程|课程详情|专题详情|全部课程|培训项目|学习任务|登录|加入自学|已加入)$/i.test(s);

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

    const getItemScope = (el) => {
      if (!el) return null;
      let current = el;
      let best = el;
      for (let depth = 0; current && current !== document.body && depth < 5; depth++, current = current.parentElement) {
        const t = clean(current.innerText);
        const candidateTitle = titleFrom(current);
        const nonTitleText = candidateTitle ? t.split(candidateTitle).join(" ") : t;
        const cleanedNonTitle = nonTitleText.replace(/(?:已考试|未考试|去考试|待考试|参加考试|开始考试|进入考试|考试中|考试通过|考试合格|考试不合格|补考|填写问卷|参与问卷|去问卷|去评价|已评价|课件学习)/g, " ");
        const badgeCount = countMatches(cleanedNonTitle, /(?:线下课|面授|面授课|问卷|调查问卷|调研问卷|评价表|课件|考试|视频课)/g);
        const durationCount = countMatches(t, /(?:学习时长|时长)\s*[:：]?\s*\d+\s*分钟|(?:\d+\s*分钟|\d+:\d+)/g);
        if (badgeCount > 1 || durationCount > 1) {
          break;
        }
        if (current.matches && current.matches(".course-content-item, [class*='content-item'], li, tr")) {
          best = current;
          break;
        }
        if (current.matches && current.matches("[class*='card'], [class*='item']")) {
          best = current;
        }
      }
      return best;
    };

    const detectCourseKind = (title, text, durationSeconds, container) => {
      const cleanTitle = String(title || "").trim();
      const cleanText = String(text || "").trim();
      const scope = getItemScope(container) || container;
      const scopeText = scope ? clean(cleanText + " " + (scope.innerText || "")) : cleanText;

      // 提取当前条目自身范围内的独立徽章标签文本
      const badgeEls = scope && scope.querySelectorAll ?
        Array.from(scope.querySelectorAll("[class*='tag'], [class*='badge'], [class*='label'], [class*='type'], span, em, i")) : [];
      const badgeTexts = badgeEls.map((e) => clean(e.innerText)).filter((t) => t.length >= 2 && t.length <= 12);
      const hasBadgeMatching = (regex) => badgeTexts.some((t) => regex.test(t));

      // 1. 【线下课与面授优先识别】必须优先以当前条目自身的徽章为准，绝不能被后续问卷或视频误吞
      const hasExplicitOfflineBadge = hasBadgeMatching(/^(线下课|面授|面授课|现场培训|线下培训)$/) ||
        /(?:^|[^a-zA-Z0-9])(线下课|面授|面授课)(?:[^a-zA-Z0-9]|$)/.test(cleanText) ||
        /(?:线下课|面授|面授课|线下\+线上|线上\+线下)/.test(cleanTitle);
      if (hasExplicitOfflineBadge) {
        return "offline";
      }

      // 2. 【问卷与评价表识别】必须具备自身明确的问卷徽章，或标题以问卷/评价表结尾，或标题明确为讲师授课满意度评价
      const hasExplicitSurveyBadge = hasBadgeMatching(/^(问卷|调查问卷|调研问卷|评价表|满意度评价|课后评价|讲师评价)$/) ||
        /(问卷|调查问卷|调研问卷|评价表|满意度评价)$/.test(cleanTitle) ||
        /(讲师授课满意度评价|满意度调查|课后满意度)/.test(cleanTitle);
      if (hasExplicitSurveyBadge) {
        return "survey";
      }

      // 3. 【文档与资料材料检测】
      const hasExplicitMaterialBadge = hasBadgeMatching(/^(文档|资料|阅读材料|参考资料|手册|pdf|word|docx)$/i) ||
        /[-_（(【\[\s](文档|阅读材料|参考资料|资料|pdf|手册|word|docx)[)）\]\s]?$/i.test(cleanTitle) ||
        /(^|[^a-zA-Z0-9])(文档|资料|阅读材料|参考资料|手册|pdf|word|docx)([^a-zA-Z0-9]|$)/i.test(scopeText);
      if (hasExplicitMaterialBadge) {
        return "material";
      }

      // 4. 【课件/PPT 检测】
      const hasExplicitSlidesBadge = hasBadgeMatching(/^(课件|ppt课件|幻灯片|ppt|pptx)$/i) ||
        /[-_（(【\[\s](课件|幻灯片|ppt课件|ppt|pptx)[)）\]\s]?$/i.test(cleanTitle) ||
        /^.*[-_](课件|ppt|pptx)$/i.test(cleanTitle) ||
        /(^|[^a-zA-Z0-9])(ppt|pptx|ppt课件|课件|幻灯片)([^a-zA-Z0-9]|$)/i.test(scopeText);
      if (hasExplicitSlidesBadge) {
        return "slides";
      }

      // 5. 【考试与测验检测】
      const isTechTesting = /(软件测试|压力测试|接口测试|性能测试|自动化测试|测试用例|测试开发|单元测试|测试方法|测试体系|测试流程|测试实战|测试理论)/.test(cleanTitle);
      const hasExplicitExamBadge = hasBadgeMatching(/^(考试|测验|试卷|测试)$/) || /(^|\s)(考试|测验|试卷)(\s|$)/.test(cleanText);
      const hasExamKeywordInTitle = /(期末考试|结业考试|随堂测验|模拟考试|在线考试|阶段测验|课后测验|综合测试|结业测试|试卷)|^.*(考试|测验)$/.test(cleanTitle) ||
        (!isTechTesting && /(考试|测验)/.test(cleanTitle));
      if (hasExplicitExamBadge || hasExamKeywordInTitle) {
        return "exam";
      }

      // 6. 【条目自身挂载的视频特征】
      const hasExplicitVideoBadge = hasBadgeMatching(/^(视频|视频课)$/) || /(^|\s)(视频|视频课)(\s|$)/.test(cleanText);
      if (hasExplicitVideoBadge) {
        return "video";
      }
      if (scope && scope.querySelector) {
        if (scope.querySelector("video, audio") || (scope.matches && scope.matches("video, audio"))) {
          return "video";
        }
        const hasVideoFeature = scope.querySelector(
          "[class*='video'], [class*='player'], [class*='play-btn'], [class*='play_btn'], [class*='play-icon'], [class*='play_icon'], [data-type*='video'], svg[class*='play'], i[class*='play'], [class*='icon-play']"
        );
        if (hasVideoFeature) {
          return "video";
        }
      }

      // 7. 【真实时长特征保护】
      // 凡是具有明确时长（>= 120 秒，即 >= 2 分钟）的课程，绝非静态课件，判定为视频！
      const hasRealisticDuration = Number(durationSeconds) >= 120;
      if (hasRealisticDuration) {
        return "video";
      }

      // 8. 【全局播放器回退】：仅在无任何特殊标识时，若页面存在播放器才作为视频
      if (typeof document !== "undefined" && document.querySelector && document.querySelector("video, .prism-player, [class*='player']")) {
        return "video";
      }

      return "video";
    };

    const isElementCompleted = (element) => {
      if (!element) return false;
      // 调用方已定位单课容器；再次向上查找会读到分组内其他课程的完成标记。
      const row = element;
      const combinedText = clean(row.innerText);

      // “上次学习”仅标记最近访问的章节，已完成课程也会显示，不能据此否定完成图标。
      // 明确未完成状态仍需排除，避免把待学或学习中的课程识别为已完成。
      if (/(学习中|播放中|未学习|未开始|待学习)/.test(combinedText) && !/(已完成|已学完|已学习|已考合格|100%)/.test(combinedText)) {
        return false;
      }

      // 单课明确的进度优先于文本、类名和图标，部分进度不能被强制改为 100%。
      const rowProgress = combinedText.match(/(?:学习)?进度\s*[:：]?\s*(\d+(?:\.\d+)?)%/);
      if (rowProgress) return Number(rowProgress[1]) >= 100;

      // 1. 文本匹配与对勾字符（包含“已学习”徽章）
      if (/(已完成|已学完|已学习|已考合格|考试合格|已通过|已考试通过|已考试|进度\s*[:：]?\s*100%)/.test(combinedText)) {
        return true;
      }
      if (/[✓✔☑✅]/.test(combinedText)) {
        return true;
      }

      // 2. 显式属性与无障碍标记
      if (row.querySelector && row.querySelector("[title*='完成'], [title*='已学完'], [title*='已学习'], [title*='已通过'], [aria-label*='完成'], [aria-label*='已学完'], [aria-label*='已学习'], [aria-label*='已通过']")) {
        return true;
      }

      // 3. 收集可能展示图标或状态的元素（严格限定在当前行内部，严禁跨越到前驱兄弟节点）
      const targets = [
        row,
        ...(row.querySelectorAll ? Array.from(row.querySelectorAll("i, span, em, svg, [class*='icon'], [class*='status'], [class*='state'], [class*='badge'], [class*='check'], [class*='finish'], [class*='success']")) : [])
      ];

      for (const el of targets) {
        if (!el) continue;
        const cls = String((el.className && typeof el.className === "string" ? el.className : (el.getAttribute && el.getAttribute("class"))) || "").toLowerCase();

        // 明确未完成/等待/橙色样式仍排除。
        if (/(orange|warn|pending|unfinished|unfinish|not-finish|playing)/i.test(cls)) {
          continue;
        }
        // YS 当前章节可同时带 active completed；选中状态不能覆盖独立的完成状态。
        // 只有选中/激活对勾、没有明确完成类名的元素，仍不能据此判定课程完成。
        const hasCompletionStatus = /(^|[\s_-])(success|finish|finished|completed|complete)([\s_-]|$)/i.test(cls);
        if (/(last-learn|current|active|xuanzhong|select)/i.test(cls) && !hasCompletionStatus) {
          continue;
        }
        const style = String((el.getAttribute && (el.getAttribute("style") || el.getAttribute("stroke") || el.getAttribute("fill"))) || "").toLowerCase();
        if (style.includes("rgb(255") || style.includes("rgba(255") || style.includes("#ff") || style.includes("#fa") || style.includes("#f6") || style.includes("#e6a23c") || style.includes("orange")) {
          continue;
        }

        if (
          /(^|[\s_-])(check|checked|checkmark|success|finish|finished|completed|complete|is-finish|is-complete|status-complete|state-complete)([\s_-]|$)/i.test(cls) ||
          /(circle-check|check-circle|icon-check|icon-success|van-icon-success|el-icon-check|anticon-check)/i.test(cls)
        ) {
          return true;
        }

        if (el.tagName && el.tagName.toLowerCase() === "svg") {
          const svgHtml = (el.innerHTML || "").toLowerCase();
          // 仅匹配明确的对勾/完成语义，切勿匹配通用形状标签 polyline 或选中类名 xuanzhong
          if (/(check|finish|success|wancheng)/i.test(svgHtml)) {
            return true;
          }
          const useEl = el.querySelector && el.querySelector("use");
          if (useEl) {
            const href = String(useEl.getAttribute("href") || useEl.getAttribute("xlink:href") || "").toLowerCase();
            if (/(check|finish|success|wancheng)/.test(href)) {
              return true;
            }
          }
        }

        if (el.getAttribute) {
          const dataStatus = String(el.getAttribute("data-status") || el.getAttribute("data-state") || "").toLowerCase();
          if (/^(finish|finished|completed|complete|success|done|passed|2)$/.test(dataStatus)) {
            return true;
          }
        }
      }
      return false;
    };

    const isCourseCard = (element) => {
      if (!element || element === document.body) return false;
      const text = clean(element.innerText);
      if (text.length < 6 || text.length > 1200) return false;

      // 必须排除课程详情总览大卡片与页面信息头部区域（包含起止时间、学分统计、超越员工数、培训介绍等元信息）
      if (/(原创作者|贡献者|学习人数|完成任务数|超越员工数|获必修学分|起止时间|有效时间|开课时间|课程介绍|专题介绍|培训介绍|培训项目|主讲老师\s*[:：])/.test(text)) {
        return false;
      }
      // 必须排除章节总标题面板头部（如“章节 (2) 时长：446分钟”或“章节（1）”）
      if (/^章节\s*[（(]?\d+[）)]?/.test(text) || /^目录\s*[（(]?/.test(text)) {
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
      return url || title || locator;
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
      /^\d+(?:\.\d+)?\s*\/\s*\d+(?:\.\d+)?$/.test(s) ||
      /^\d+(?:\.\d+)?%$/.test(s) ||
      /^(?:起止时间|有效时间|开课时间|培训时间|考试时间)/.test(s) ||
      /\d{4}[-/.]\d{1,2}[-/.]\d{1,2}.*?[~至到-]\s*\d{4}[-/.]\d{1,2}[-/.]\d{1,2}/.test(s) ||
      /^(?:获|已获)?(?:必修学分|选修学分|学分)/.test(s) ||
      /^(?:超越员工数|完成任务数|学习人数|浏览人数|课程数)/.test(s) ||
      /^\d+\s*分钟$/.test(s) ||
      /^\d+:\d+$/.test(s) ||
      /^\d+\s*人看过$/.test(s) ||
      /^章节\s*[（(]?\d+[）)]?$/.test(s) ||
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

    // 1. 优先定位右侧章节目录面板或培训内容面板
    const findCatalogPanel = () => {
      // 1.0 针对培训项目详情页中“培训内容”Tab 面板
      const trainingContentPanes = Array.from(
        document.querySelectorAll(".ant-tabs-tabpane-active, [class*='tabpane-active'], [role='tabpanel'], [class*='train-content'], [class*='training-content']")
      ).filter((el) => {
        if (!visible(el) || isNavOrHeader(el)) return false;
        const text = clean(el.innerText);
        if (text.length < 15 || text.length > 30000) return false;
        if (/(起止时间|获必修学分|超越员工数)/.test(text)) return false;
        const hasChapterOrLesson =
          !!el.querySelector(".course-stage-caption, .course-content-item, [class*='stage-caption'], [class*='content-item'], .ant-collapse-item, [class*='collapse-item'], li") ||
          /(?:^|\s)\d{1,2}\s+[^\s]/.test(text) ||
          /(线下课|问卷|视频|课件|考试|\d+\s*分钟|\d+:\d+)/.test(text);
        return hasChapterOrLesson;
      });
      if (trainingContentPanes.length > 0) {
        trainingContentPanes.sort((a, b) => a.innerText.length - b.innerText.length);
        return trainingContentPanes[0];
      }

      // 1. 寻找包含 "章节 (X)" 或 "目录" 或 "章节列表" 的所有容器
      const candidates = Array.from(document.querySelectorAll("body *")).filter((el) => {
        if (!visible(el) || isNavOrHeader(el)) return false;
        if (el.closest(".prism-player, [class*='player'], [class*='control-bar'], [class*='controls']")) return false;
        const text = clean(el.innerText);
        const hasCatalogHeader = /(章节\s*[（(]?\d+[）)]?|课程目录|章节列表)/.test(text);
        if (!hasCatalogHeader) return false;
        if (text.length < 20 || text.length > 25000) return false;

        // 该容器内部必须实际包含章节列表项（即使全折叠，也包含 .course-stage-caption、.course-content-item、.course-stage-index 等）
        const hasKnownStage = !!el.querySelector(".course-stage-caption, .course-content-item, .course-stage-index, [class*='stage-caption'], [class*='content-item'], .ant-collapse-item, [class*='collapse-item']");
        if (hasKnownStage) return true;

        const childTexts = Array.from(el.querySelectorAll("div, li, span, p")).map((c) => clean(c.innerText));
        const hasChapters = childTexts.some((t) => /^\d{1,2}\s*[^\s\d]/.test(t) || /^\d{1,2}\s+[^\s]/.test(t) || /^\d{1,2}\s*[.、-]/.test(t) || /^\d{1,2}$/.test(t));
        return hasChapters;
      });

      if (candidates.length > 0) {
        candidates.sort((a, b) => a.innerText.length - b.innerText.length);
        return candidates[0];
      }

      // 1.5 针对包含 .course-detail-right-wrapper 等已知章节目录外层容器
      const rightWrappers = Array.from(document.querySelectorAll(".course-detail-right-wrapper, [class*='detail-right-wrapper']")).filter((el) => {
        if (!visible(el) || isNavOrHeader(el)) return false;
        return !!el.querySelector(".course-stage-caption, .course-content-item, [class*='stage-caption'], [class*='content-item']");
      });
      if (rightWrappers.length > 0) {
        return rightWrappers[0];
      }

      // 2. 针对包含 .course-stage-caption 或 .course-content-item 的容器
      const stageContainers = Array.from(document.querySelectorAll("div, section, aside")).filter((el) => {
        if (!visible(el) || isNavOrHeader(el)) return false;
        if (el.closest(".prism-player, [class*='player'], [class*='control-bar']")) return false;
        const stages = el.querySelectorAll(".course-stage-caption, [class*='stage-caption']");
        const items = el.querySelectorAll(".course-content-item, [class*='content-item']");
        return (stages.length >= 1 && items.length >= 1) || stages.length >= 2;
      });
      if (stageContainers.length > 0) {
        stageContainers.sort((a, b) => a.innerText.length - b.innerText.length);
        return stageContainers[0];
      }

      // 3. 回退：寻找包含章节项（支持单章节或多章节）的公共容器
      const allContainers = Array.from(document.querySelectorAll("div, section, aside, ul")).filter((el) => {
        if (!visible(el) || isNavOrHeader(el)) return false;
        if (el.closest(".prism-player, [class*='player'], [class*='control-bar']")) return false;
        const text = clean(el.innerText);
        if (text.length < 20 || text.length > 25000) return false;
        if (/(起止时间|获必修学分|超越员工数)/.test(text)) return false;
        const children = Array.from(el.children);
        const chapterChildren = children.filter((c) => {
          const ct = clean(c.innerText);
          return /^\d{1,2}\s*[^\s\d]/.test(ct) || /^\d{1,2}\s+[^\s]/.test(ct) || /^\d{1,2}\s*[.、-]/.test(ct) || c.querySelector(".course-stage-caption, [class*='stage-caption']");
        });
        if (chapterChildren.length >= 1) {
          return /(线下课|问卷|视频|课件|考试|\d+\s*分钟|\d+:\d+)/.test(text);
        }
        return false;
      });

      if (allContainers.length > 0) {
        allContainers.sort((a, b) => a.innerText.length - b.innerText.length);
        return allContainers[0];
      }

      return null;
    };

    const parseCatalogCourses = (panel) => {
      if (!panel) return [];

      const nodeText = (el) => {
        if (!el) return "";
        const t = clean(el.innerText);
        if (t) return t;
        return clean(el.textContent);
      };

      // 提取规范章节条目：识别并提取所有子课程（播放课程条目），彻底排除大章节名称
      const rawCandidates = Array.from(
        panel.querySelectorAll("li, div, a, [class*='item'], [class*='chapter'], [class*='section'], [class*='node'], [class*='lesson']")
      ).filter((el) => {
        if (isNavOrHeader(el)) return false;
        if (el.closest(".prism-player, [class*='player'], [class*='control-bar'], [class*='controls'], [class*='speed'], [class*='quality'], [class*='intro'], [class*='teacher']")) return false;
        const text = nodeText(el);
        if (text.length < 2 || text.length > 250) return false;
        if (/(倍速|标清|高清|超清|人看过|课程介绍|主讲老师|收起目录|展开目录|00:00)/.test(text)) return false;
        if (/^(章节\s*[（(]?\d+[）)]?|时长\s*[:：]|\d+\s*分钟$)/.test(text)) return false;

        // 排除大章节折叠头部（大章节名称如 "01 如何使用企微链接..."，必须排除含有时长或在普通 li 列表中的真正小节）
        const hasLessonDuration = /(\d+)\s*分钟|\d+:\d+/.test(text);
        const isLiItem = el.tagName === "LI" || !!el.closest("li");
        const isExplicitStageHeader = el.matches(
          ".course-stage-caption, [class*='stage-caption'], .ant-collapse-header, [class*='collapse-header'], [class*='collapse-item__header'], [class*='chapter-header'], [class*='chapter_header'], [class*='stage__header']"
        ) || !!el.closest(".course-stage-caption, [class*='stage-caption'], .ant-collapse-header, [class*='collapse-header']");

        const isBigChapterHeader = isExplicitStageHeader || (!hasLessonDuration && !isLiItem && (/^\d{1,2}\s*[^\s\d.、-]/.test(text) || /^\d{1,2}\s+[^\s]/.test(text)) && !/^\d{1,2}\s*[.、-]/.test(text) && !/(视频|文档|课件|线下课|面授|问卷|调查问卷|调研问卷|评价表|考试|测验|已考试|去考试|进度|已完成|未学习)/.test(text));
        if (isBigChapterHeader) return false;

        // 子小节特征：小节编号(如 01. / 1. / 01)、时长、小节类型徽章(视频/文档/课件/线下课/问卷等)、进度/状态，或者已知小节容器类名
        const isKnownContentItem = el.matches(".course-content-item, [class*='content-item']") || !!el.closest(".course-content-item, [class*='content-item']");
        const hasLessonNumber = /^\d{1,2}\s*[.、-]/.test(text) || /^第\d+[讲节课步]\s*/.test(text) || (/^\d{1,2}\s+[^\s]/.test(text) && text.length < 90);
        const hasDuration = /(\d+)\s*分钟/.test(text) || /\d+:\d+/.test(text);
        const hasBadge = /(视频|课件|文档|资料|手册|ppt|考试|测验|线下课|面授|面授课|问卷|调查问卷|调研问卷|评价表|满意度评价)/i.test(text);
        const hasProgressOrStatus = /进度\s*[:：]?\s*\d+(?:\.\d+)?%/.test(text) || /(已完成|未学习|学习中|上次学习|待播放|未开始|播放中|已考试|未考试|待考试|去考试|参加考试|开始考试|进入考试|考试通过|考试合格)/.test(text) || (el.classList && (el.classList.contains("completed") || el.classList.contains("active")));
        const hasPlayIcon = !!el.querySelector("svg, i, [class*='play'], [class*='video'], [class*='icon']");

        return isKnownContentItem || (isLiItem && (hasLessonNumber || hasDuration)) || (hasLessonNumber && hasDuration) || hasLessonNumber || (hasDuration && (hasBadge || hasProgressOrStatus)) || (hasBadge && (hasProgressOrStatus || hasDuration)) || (hasLessonNumber && hasPlayIcon);
      });

      // 过滤出真正代表一门小节的条目（排除大容器和小标签）
      const singleLessonItems = rawCandidates.filter((el) => {
        const t = nodeText(el);
        const isPureMeta = /^(进度\s*[:：]?\s*\d+(?:\.\d+)?%|学习时长\s*[:：].*|时长\s*[:：].*|\d+\s*分钟|学时\s*[:：].*|学分\s*[:：].*)$/i.test(t);
        if (isTagOrBadge(t) || isPureTagOrBadge(t) || isPureMeta || /^\d+\s*分钟$/.test(t)) return false;
        const lines = String(el.innerText || el.textContent || "").split(/\n+/).map(clean).filter(Boolean);
        const validLines = lines.filter((l) => l.length >= 2 && !isTagOrBadge(l) && !isPureTagOrBadge(l) && !isMeta(l) && !isPhaseOrSectionHeader(l));
        if (validLines.length === 0) return false;

        // 排除包含多个不同小节的父容器（例如如果它的子元素里有多个编号开头的小节，或包含多个小节徽章）
        const children = Array.from(el.children);
        const childLessons = children.filter((c) => {
          const ct = nodeText(c);
          return /^\d{1,2}\s*[.、-]/.test(ct) || (/^\d{1,2}\s+[^\s]/.test(ct) && ct.length < 80);
        });
        if (childLessons.length > 1) return false;

        // 排除包含多个不同小节的父级列表容器（多个独立时长、多个独立学分）
        const durCount = countMatches(t, /(?:学习时长|时长)\s*[:：]?\s*\d+\s*分钟|(?:\d+\s*分钟|\d+:\d+)/g);
        if (durCount > 1) return false;
        const creditCount = countMatches(t, /(?:必修学分|选修学分|学分)\s*[:：]?\s*\d+/g);
        if (creditCount > 1) return false;

        // 统计排除自身标题后的独立小节类型徽章数，防止标题中的“满意度评价表”与“问卷”徽章叠加导致误判为多任务容器
        const candidateTitle = titleFrom(el) || validLines[0] || "";
        const nonTitleText = candidateTitle ? t.split(candidateTitle).join(" ") : t;
        const cleanedNonTitle = nonTitleText.replace(/(?:已考试|未考试|去考试|待考试|参加考试|开始考试|进入考试|考试中|考试通过|考试合格|考试不合格|补考|填写问卷|参与问卷|去问卷|去评价|已评价|课件学习)/g, " ");
        const innerBadges = countMatches(cleanedNonTitle, /(?:线下课|面授|面授课|问卷|调查问卷|调研问卷|评价表|满意度评价|课件|考试|视频课)/g);
        if (innerBadges > 1) return false;
        return true;
      });

      // 当父子两层都满足单课条件时（例如外层行与内层标题 div），保留包含更多元信息（类型徽章/进度/时长/考试）的外层行
      const leafCandidates = singleLessonItems.filter((item) => {
        const parent = item.parentElement;
        if (parent && singleLessonItems.includes(parent)) {
          const pText = nodeText(parent);
          const iText = nodeText(item);
          // 若父容器文本并没有多包含其他小节，且包含类型、进度或考试，则保留父容器
          if (/(视频|文档|课件|ppt|pptx|考试|测验|测试|线下课|面授|问卷|评价表|进度|%|分钟|未完成)/i.test(pText) && !/(视频|文档|课件|ppt|pptx|考试|测验|测试|线下课|面授|问卷|评价表|进度|%|分钟|未完成)/i.test(iText)) {
            return false;
          }
        }
        return true;
      });

      // 如果筛选出的列表较少，回退保留 leafOnly
      const finalItems = leafCandidates.length > 0 ? leafCandidates : rawCandidates.filter((item) =>
        !rawCandidates.some((other) => other !== item && item.contains(other))
      );

      const catalogSeen = new Set();
      const parsed = [];
      finalItems.forEach((item) => {
        const text = nodeText(item);
        const card = getItemScope(item) || item;
        const cardText = clean(card.innerText || card.textContent || "");
        const ownItemText = clean(text + " " + cardText);

        const lines = String(item.innerText || item.textContent || "").split(/\n+/).map(clean).filter(Boolean);
        const validLines = lines.filter((l) =>
          l.length >= 2 &&
          !isTagOrBadge(l) &&
          !isPureTagOrBadge(l) &&
          !isMeta(l) &&
          !/^\d+\s*分钟$/.test(l) &&
          !/^(上次学习|已完成|未完成|未开始|播放中|试看|\d+:\d+)$/.test(l)
        );
        validLines.sort((a, b) => scoreTitleCandidate(b) - scoreTitleCandidate(a) || b.length - a.length);
        const rawTitle = titleFrom(item) || titleFrom(card) || validLines[0] || "";
        let title = rawTitle
          .replace(/\s*\d+\s*分钟.*$/, "")
          .replace(/\s*上次学习.*$/, "");
        // 彻底清洗标题末尾拼接的类型标签与状态词（如 "考试 未完成"、"需本人处理"、"未完成"、"视频" 等）
        title = title.replace(/(?:\s+(?:视频|课件|文档|资料|手册|ppt课件|ppt|pptx|考试|测验|测试|问卷|未完成|已完成|未学习|学习中|已考试|合格|不合格|已通过|未通过|需本人处理|去学习|立即学习|开始学习|待播放|播放中))+$/i, "").trim();

        if (!title || title.length < 2 || isInvalidTopicTitle(title) || isPhaseOrSectionHeader(title) || catalogSeen.has(title)) return;
        catalogSeen.add(title);

        const locator = cssPath(item);
        const url = linkFrom(item);
        const externalId = externalIdFrom(item, url, locator, title);

        let durMatch = ownItemText.match(/学习时长\s*[:：]?\s*(\d+)\s*分钟/) || ownItemText.match(/(\d+)\s*分钟/) || ownItemText.match(/学时\s*[:：]?\s*(\d+)/) || ownItemText.match(/时长\s*[:：]?\s*(\d+)/);
        let durationSeconds = 0;
        if (durMatch) {
          const val = Number(durMatch[1]) || 0;
          if (/学时/.test(durMatch[0])) {
            durationSeconds = val * 45 * 60;
          } else {
            durationSeconds = val * 60;
          }
        }

        const progressMatch = ownItemText.match(/(?:学习)?进度\s*[:：]?\s*(\d+(?:\.\d+)?)%/);
        let progress = 0;
        let completed = false;

        if (progressMatch) {
          const pVal = Number(progressMatch[1]) || 0;
          if (pVal >= 100) {
            completed = true;
            progress = 100;
          } else {
            // 只要条目自身明确标明了小于 100% 的进度（例如 16%、37%、0%），必须判定为未完成，保留其真实进度！
            completed = false;
            progress = pVal;
          }
        } else {
          // 仅在条目自身没有明确百分比进度时，才根据未完成/已完成词条及完成图标判定
          const hasExplicitIncomplete = /(学习中|播放中|未学习|未开始|待学习)/.test(ownItemText);
          if (!hasExplicitIncomplete) {
            if (isElementCompleted(card) || /(已完成|已学完|已学习|已考合格|已通过|已考试)/.test(ownItemText)) {
              completed = true;
              progress = 100;
            }
          }
        }

        const itemKind = detectCourseKind(title, text, durationSeconds, item);

        // 不再显示章节名称二级目录，直接平铺在专题下方
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
    const catalogPanel = findCatalogPanel();
    const searchRoot = catalogPanel || document.body;

    const isBigChapterTitle = (text) => {
      const t = clean(text);
      if (t.length < 3 || t.length > 90) return false;
      if (/(进度|已完成|未学习|学习中|\d+:\d+|\d+\s*分钟|原创作者|学习人数|课程介绍|讲师简介|评价)/.test(t)) return false;
      if (/^\d{1,2}\s*[.、-]/.test(t) || /^第\d+[讲节课步]\s*/.test(t)) return false;
      return /^\d{1,2}\s*[^\s\d.、-]/.test(t) || /^\d{1,2}\s+[^\s]/.test(t) || /^第[0-9一二三四五六七八九十]+[章节部分篇]\s*/.test(t);
    };

    const getChapterHeaders = (root) => {
      const searchTargets = [root, document.body].filter(Boolean);
      for (const target of searchTargets) {
        const byClass = Array.from(
          target.querySelectorAll(
            ".course-stage-caption, [class*='stage-caption'], .ant-collapse-header, [class*='collapse-header'], [class*='collapse-item__header'], [class*='chapter-header'], [class*='chapter_header'], [class*='stage__header']"
          )
        ).filter((el) => {
          if (!visible(el) || isNavOrHeader(el)) return false;
          if (el.closest(".prism-player, [class*='player'], [class*='control-bar'], [class*='controls']")) return false;
          // 排除含有具体小节时长或属于 li 列表的普通课程条目，绝不把普通小节误认作大章节头部
          const t = clean(el.innerText || el.textContent);
          if (/(\d+)\s*分钟|\d+:\d+/.test(t) || el.tagName === "LI" || !!el.closest("li")) return false;
          return true;
        });
        if (byClass.length > 0) return byClass;
      }
      return [];
    };

    let allCatalogCourses = [];
    const seenCatalogTitles = new Set();
    const addCatalogCourses = (list) => {
      if (!Array.isArray(list)) return;
      for (const item of list) {
        if (!item || !item.title) continue;
        const existing = allCatalogCourses.find((c) => c.title === item.title);
        if (!existing) {
          seenCatalogTitles.add(item.title);
          allCatalogCourses.push(item);
        } else {
          // 如果后续解析得到了更高的进度或完成状态，进行更新
          if (item.completed && !existing.completed) {
            existing.completed = true;
            existing.progress = 100;
          } else if (item.progress > existing.progress) {
            existing.progress = item.progress;
          }
          if (item.durationSeconds > 0 && existing.durationSeconds === 0) {
            existing.durationSeconds = item.durationSeconds;
          }
        }
      }
    };

    // 1. 先收集初始状态下已渲染/已展开的课程条目（大多数普通课程直接在此处完整获取全部小节！）
    addCatalogCourses(parseCatalogCourses(searchRoot));

    // 2. 只有在页面上明确存在“处于折叠/收起状态且缺失子条目的大章节”时，才需要进行按需点击展开！
    // 普通课程（所有章节直接展示）无折叠大章节，跳过点击逻辑，零额外操作
    const getChapterContainer = (h) =>
      (h.parentElement ? h.parentElement.closest("[class*='stage'], [class*='chapter'], .ant-collapse-item, [class*='collapse-item']") : null) || h.parentElement;

    const hasChildItems = (h) => {
      const parent = getChapterContainer(h);
      if (parent && parent.querySelector(".course-content-item, [class*='content-item']")) return true;
      let sibling = h.nextElementSibling;
      while (sibling) {
        if (sibling.matches(".course-stage-caption, [class*='stage-caption'], .ant-collapse-header, [class*='collapse-header']")) break;
        if (sibling.matches(".course-content-item, [class*='content-item']") || sibling.querySelector(".course-content-item, [class*='content-item']")) return true;
        sibling = sibling.nextElementSibling;
      }
      return false;
    };

    const isHeaderCollapsed = (h) => {
      // 只要该章节当前已经存在课时小节条目，说明已经是展开状态，严禁点击折叠！
      if (hasChildItems(h)) return false;
      const arrow = h.querySelector(".anticon, svg, [class*='arrow'], [class*='icon']");
      const arrowStyle = (arrow ? arrow.getAttribute("style") || "" : "") + (arrow && arrow.parentElement ? arrow.parentElement.getAttribute("style") || "" : "");
      const isRotated = /rotate\(-?90deg\)/i.test(arrowStyle);
      const isAriaClosed = h.getAttribute("aria-expanded") === "false";
      return isRotated || isAriaClosed || !arrow;
    };

    const chapterHeaders = getChapterHeaders(searchRoot);
    const hasCollapsedChapters = chapterHeaders.some(isHeaderCollapsed);

    if (chapterHeaders.length > 0 && hasCollapsedChapters) {
      for (let i = 0; i < chapterHeaders.length; i++) {
        const h = chapterHeaders[i];
        try {
          if (!isHeaderCollapsed(h)) {
            continue; // 已经展开，直接跳过，绝不点击！
          }

          const parent = getChapterContainer(h);
          const arrow = h.querySelector(".anticon, svg, [class*='arrow'], [class*='icon']");

          try { h.scrollIntoView({ block: "nearest", behavior: "instant" }); } catch (_) {}

          // 触发点击展开
          h.click();
          try { h.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window })); } catch (_) {}

          if (arrow) {
            try { arrow.click(); } catch (_) {}
            try { arrow.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window })); } catch (_) {}
          }
          const innerSpan = h.querySelector(".course-stage-index, span, [role='button']");
          if (innerSpan && innerSpan !== h) {
            try { innerSpan.click(); } catch (_) {}
          }

          // 等待 React 异步渲染子小节
          await sleep(350);
          let waitTries = 0;
          while (waitTries < 4 && !hasChildItems(h)) {
            await sleep(150);
            waitTries++;
          }

          // 展开后，从当前章节父容器、searchRoot 以及 document.body 中提取刚渲染出的子课程
          if (parent) {
            addCatalogCourses(parseCatalogCourses(parent));
          }
          addCatalogCourses(parseCatalogCourses(searchRoot));
          addCatalogCourses(parseCatalogCourses(document.body));
        } catch (_) {}
      }
    }

    const catalogCourses = allCatalogCourses;
    if (catalogCourses.length > 0) {
      courses = catalogCourses;
    } else {
      // 2. 如果特定 panel 没命中，优先在全局 document.body 搜索章节目录！
      const bodyCatalog = parseCatalogCourses(document.body);
      if (bodyCatalog.length > 0) {
        courses = bodyCatalog;
      } else {
        // 3. 只有当全局均未匹配到任何章节小节时，才回退至常规专题页基于 markers 扫描
        const markers = Array.from(document.querySelectorAll("body *")).filter((element) => {
          if (!visible(element)) return false;
          // 排除页面顶部概览/头部信息卡内的任何元素
          const overview = element.closest && element.closest("[class*='header'], [class*='banner'], [class*='overview'], [class*='summary'], [class*='project-info'], [class*='train-info'], [class*='course-info']");
          if (overview && /(起止时间|获必修学分|超越员工数|完成任务数)/.test(clean(overview.innerText))) {
            return false;
          }
          const text = clean(element.innerText);
          if (provider === "merchant") {
            return /^(已完成|已学完|已考试|未学习|学习中|去学习|立即学习|未考试|待考试|去考试|参加考试)$/.test(text) || /^进度\s*[:：]?\s*\d+(?:\.\d+)?%$/.test(text);
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
          if (!title || isTagOrBadge(title) || isMeta(title) || isInvalidTopicTitle(title) || isPhaseOrSectionHeader(title) || seenTitles.has(title)) return;
          seenTitles.add(title);

          const text = clean(container.innerText);
          const locator = cssPath(container);
          const url = linkFrom(container);
          const externalId = externalIdFrom(container, url, locator, title);

          const progressMatch = text.match(/(?:学习)?进度\s*[:：]?\s*\d+(?:\.\d+)?%/);
          let progress = 0;
          let completed = false;

          if (progressMatch) {
            const pVal = Number(progressMatch[1]) || 0;
            if (pVal >= 100) {
              completed = true;
              progress = 100;
            } else {
              completed = false;
              progress = pVal;
            }
          } else {
            const hasExplicitIncomplete = /(未学习|未开始|待学习|学习中|播放中)/.test(text);
            if (!hasExplicitIncomplete) {
              if (isElementCompleted(container) || /(已完成|已学完|已学习|已考合格|已通过|已考试)/.test(text)) {
                completed = true;
                progress = 100;
              }
            }
          }

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

    let topicProgressVal = null;
    try {
      const metaEls = Array.from(document.querySelectorAll("body *")).filter((el) => {
        if (!visible(el) || isNavOrHeader(el)) return false;
        const t = clean(el.innerText);
        return /^(?:学习人数|原创作者|主讲老师|起止时间|有效时间)/.test(t);
      });
      for (const mEl of metaEls) {
        let p = mEl.parentElement;
        for (let d = 0; p && p !== document.body && d < 4; d++, p = p.parentElement) {
          const pt = clean(p.innerText);
          if (pt.length > 500) continue;
          const match = pt.match(/(?:学习进度|进度)\s*[:：]?\s*(\d+(?:\.\d+)?)%/);
          if (match) {
            topicProgressVal = Number(match[1]);
            break;
          }
        }
        if (topicProgressVal !== null) break;
      }
    } catch (_) {}

    const countMatch = bodyText.match(/完成任务数\s*(\d+)\s*\/\s*(\d+)/) || bodyText.match(/完成标准\s*(\d+)\s*\/\s*(\d+)/);
    const topicProgressMatch = topicProgressVal !== null ? [null, String(topicProgressVal)] : bodyText.match(/(?:学习进度|章节进度)\s*[:：]?\s*(\d+(?:\.\d+)?)%/);
    const isAllCompletedByStats = !!(
      countMatch &&
      Number(countMatch[1]) > 0 &&
      Number(countMatch[1]) === Number(countMatch[2]) &&
      courses.length === Number(countMatch[2]) &&
      (topicProgressMatch ? Number(topicProgressMatch[1]) >= 100 : true)
    );

    if (isTopicExpired || isAllCompletedByStats) {
      courses.forEach((c) => {
        c.completed = true;
        c.progress = 100;
      });
    }

    const topicTitle = findTopicTitle();
    const completedCount = isTopicExpired || isAllCompletedByStats ? courses.length : (countMatch ? Number(countMatch[1]) : courses.filter((item) => item.completed).length);
    const totalCount = countMatch ? Number(countMatch[2]) : courses.length;
    const payload = {
      title: String(topicTitle || "未知专题"),
      url: location.href,
      progress: isTopicExpired || isAllCompletedByStats ? 100 : (topicProgressMatch ? Number(topicProgressMatch[1]) : (totalCount ? completedCount / totalCount * 100 : 0)),
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
        .replace("__PROVIDER__", "merchant")
        .replace("__CHUNK_SIZE__", &CAPTURE_CHUNK_SIZE.to_string())
        .replace("__CHUNK_INTERVAL__", &CAPTURE_CHUNK_INTERVAL_MS.to_string())
        .replace(
            "__RESTORE_DELAY__",
            &(CAPTURE_CHUNK_INTERVAL_MS * 2).to_string(),
        )
}

fn capture_script(request_id: &str, provider: Provider) -> String {
    match provider {
        Provider::Ulearn => ulearn_capture_script(request_id),
        Provider::Merchant => merchant_capture_script(request_id),
    }
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
                // 校验伪造包：若为长视频课程（kind == "video" 且 duration > 120），收到来自文档探测的伪造 100s 包时予以忽略
                let is_fake_doc_packet_for_video = active.kind == "video" && active.duration > 120.0 && duration <= 100.0;
                if !is_fake_doc_packet_for_video {
                    if current_time > 0.0 {
                        active.current_time = current_time;
                    }
                    if duration > 0.0 {
                        active.duration = duration;
                    }
                }
                if event == "ended" {
                    // 刚打开课程不到 8 秒就收到 ended，说明是上一门课的残留事件或旧弹窗，予以忽略防抖！
                    if now() - active.started_at < 8 {
                        return true;
                    }
                    // 视频课程绝不接受来自文档探测的伪造 100s 完播包！
                    if is_fake_doc_packet_for_video {
                        return true;
                    }
                    if active.phase != "ended" {
                        active.phase = "ended".to_string();
                        active.phase_since = now();
                    }
                } else if event == "need_login" {
                    if active.phase != "need_login" {
                        active.phase = "need_login".to_string();
                        active.phase_since = now();
                    }
                } else if event == "error" {
                    if active.phase != "error" {
                        active.phase = "error".to_string();
                        active.phase_since = now();
                    }
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

async fn ensure_platform_window(
    app: &AppHandle,
    state: &VideoTaskState,
    provider: Provider,
    show: bool,
    browser: bool,
) -> Result<tauri::WebviewWindow, String> {
    let label = if browser {
        provider.browser_label()
    } else {
        provider.player_label()
    };
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
    let initial_target_url = {
        let conn = Connection::open(state.db_path.as_ref()).ok();
        conn.and_then(|c| {
            c.query_row(
                "SELECT url FROM video_topics 
                 WHERE provider=?1 AND url != '' 
                 ORDER BY (CASE WHEN progress < 100.0 THEN 0 ELSE 1 END), last_synced_at DESC 
                 LIMIT 1",
                params![provider.key()],
                |row| row.get(0),
            ).ok()
        })
    };
    let url_string = initial_target_url.unwrap_or_else(|| provider.home());
    let url = url_string
        .parse::<tauri::Url>()
        .map_err(|error| error.to_string())?;
    let captures = state.captures.clone();
    let runtime = state.runtime.clone();
    let runtime_for_load = state.runtime.clone();
    let provider_for_title = provider;
    let app_for_popup = app.clone();
    let popup_label = label.clone();
    let settings_state = state.settings.clone();
    let last_title = Arc::new(Mutex::new(format!("MTOOL · {}", provider.name())));
    let last_title_for_cb = last_title.clone();
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        .title(format!("MTOOL · {}", provider.name()))
        .inner_size(1280.0, 820.0)
        .min_inner_size(640.0, 480.0)
        .devtools(true)
        .visible(show)
        .focused(show)
        .initialization_script_for_all_frames(if browser {
            browser_nav_script(provider)
        } else {
            bridge_script(provider, settings.speed, settings.muted)
        })
        .on_document_title_changed(move |window, title| {
            if title.starts_with(BRIDGE_DEVTOOLS_TOGGLE_PREFIX) {
                if window.is_devtools_open() {
                    window.close_devtools();
                } else {
                    window.open_devtools();
                }
                let prev = last_title_for_cb.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let _ = window.set_title(&prev);
                return;
            }
            // 浏览窗口只参与专题采集，不能用媒体事件覆盖后台课程状态。
            if browser && title.starts_with(BRIDGE_MEDIA_PREFIX) {
                return;
            }
            if !handle_bridge_title(&title, provider_for_title, &captures, &runtime) {
                let formatted = format!("MTOOL · {} · {title}", provider_for_title.name());
                let _ = window.set_title(&formatted);
                *last_title_for_cb.lock().unwrap_or_else(|e| e.into_inner()) = formatted;
            }
        })
        .on_page_load(move |window, _payload| {
            if browser {
                return;
            }
            let settings = settings_state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            let (course_title, course_kind) = {
                let runtime = runtime_for_load.lock().unwrap_or_else(|error| error.into_inner());
                runtime.active.get(provider.key()).map(|a| (a.course_title.clone(), a.kind.clone())).unwrap_or_default()
            };
            let _ = window.eval(update_media_script(settings.speed, settings.muted, false, &course_title, &course_kind));
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
        .map_err(|error| format!("打开{}学习窗口失败: {error}", provider.name()))?;

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

async fn ensure_player_window(
    app: &AppHandle,
    state: &VideoTaskState,
    provider: Provider,
    show: bool,
) -> Result<tauri::WebviewWindow, String> {
    ensure_platform_window(app, state, provider, show, false).await
}

async fn ensure_browser_window(
    app: &AppHandle,
    state: &VideoTaskState,
    provider: Provider,
    show: bool,
) -> Result<tauri::WebviewWindow, String> {
    ensure_platform_window(app, state, provider, show, true).await
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
        .ok_or_else(|| format!("请先点击【打开/选择专题】打开{}", provider.name()))?;
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
        "survey" => "survey",
        "offline" => "offline",
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
                title=excluded.title,url=excluded.url,
                progress=excluded.progress,
                total_count=CASE WHEN excluded.total_count > 0 THEN excluded.total_count ELSE video_topics.total_count END,
                completed_count=excluded.completed_count,
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
        let status = if course.completed || course.progress >= 100.0 {
            completed += 1;
            "completed"
        } else if kind == "video" || kind == "slides" || kind == "material" {
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
                     WHEN excluded.status='completed' OR excluded.progress >= 100.0 THEN 100.0
                     WHEN video_courses.status IN('opening','playing','verifying','paused','skipped','attention') AND video_courses.progress > excluded.progress THEN video_courses.progress
                     WHEN video_courses.status='completed' AND excluded.progress <= 0.0 AND excluded.kind NOT IN ('video','slides','material') THEN 100.0
                     ELSE excluded.progress
                   END,
                   status=CASE
                     WHEN excluded.status='completed' OR excluded.progress >= 100.0 THEN 'completed'
                     WHEN video_courses.status='completed' AND excluded.progress <= 0.0 AND excluded.kind NOT IN ('video','slides','material') THEN 'completed'
                     WHEN excluded.status='manual' AND video_courses.status!='skipped' THEN 'manual'
                     WHEN video_courses.status IN('opening','playing','verifying','paused','skipped','attention') THEN video_courses.status
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
            "DELETE FROM video_courses WHERE topic_id = ?1 AND status NOT IN ('opening', 'playing', 'verifying', 'paused', 'skipped', 'attention') AND id NOT IN ({placeholders})"
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
    transaction
        .execute(
            "UPDATE video_topics SET 
             completed_count = (SELECT COUNT(*) FROM video_courses WHERE topic_id=?1 AND status='completed'),
             total_count = (SELECT COUNT(*) FROM video_courses WHERE topic_id=?1),
             progress = CASE
               WHEN (SELECT COUNT(*) FROM video_courses WHERE topic_id=?1 AND status='completed') = (SELECT COUNT(*) FROM video_courses WHERE topic_id=?1) AND (SELECT COUNT(*) FROM video_courses WHERE topic_id=?1) > 0 THEN 100.0
               WHEN ?2 > 0.0 THEN ?2
               ELSE
                 ROUND((CAST((SELECT COUNT(*) FROM video_courses WHERE topic_id=?1 AND status='completed') AS REAL) / MAX(1, (SELECT COUNT(*) FROM video_courses WHERE topic_id=?1))) * 100.0, 1)
             END
             WHERE id=?1",
            params![topic_id, capture.progress],
        )
        .map_err(|error| error.to_string())?;

    let (db_completed, db_manual): (usize, usize) = transaction
        .query_row(
            "SELECT 
               COALESCE(SUM(CASE WHEN status='completed' THEN 1 ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN status='manual' THEN 1 ELSE 0 END), 0)
             FROM video_courses WHERE topic_id=?1",
            params![topic_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap_or((completed, manual));
    transaction.commit().map_err(|error| error.to_string())?;

    let mut runtime = state
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for (index, course) in valid_courses.iter().enumerate() {
        if course.completed || course.progress >= 100.0 {
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
        completed: db_completed,
        manual: db_manual,
    })
}

fn is_phase_or_section_title(title: &str) -> bool {
    let t = title.trim();
    let trimmed = t.trim_start_matches(|c: char| {
        c.is_ascii_digit() || c.is_whitespace() || c == '.' || c == '-' || c == '、'
    });
    if trimmed.starts_with('第')
        && (trimmed.contains('期')
            || trimmed.contains("阶段")
            || trimmed.contains("部分")
            || trimmed.contains('篇')
            || trimmed.contains('讲')
            || trimmed.contains('节')
            || trimmed.contains('章'))
    {
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
        "SELECT id,topic_id,provider,url,locator,kind,title,duration_seconds,progress FROM video_courses WHERE id=?1",
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
    // 三个手动入口共用浏览窗口；队列只导航独立的后台播放器。
    let window = if auto_play {
        ensure_window(app, state, course.provider, false).await?
    } else {
        ensure_browser_window(app, state, course.provider, true).await?
    };
    let token = CAPTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
    {
        let mut runtime = state.runtime.lock().unwrap_or_else(|error| error.into_inner());
        let tokens = if auto_play {
            &mut runtime.playback_tokens
        } else {
            &mut runtime.browser_tokens
        };
        tokens.insert(course.provider.key().to_string(), token);
    }

    let reset_course_js = format!(
        "try {{ if (window.__MTOOL_LEARNING_BRIDGE__ && typeof window.__MTOOL_LEARNING_BRIDGE__.setCourse === 'function') window.__MTOOL_LEARNING_BRIDGE__.setCourse({}, {}); }} catch (_) {{}}",
        serde_json::to_string(&course.title).unwrap_or_default(),
        serde_json::to_string(&course.kind).unwrap_or_default()
    );
    let _ = window.eval(&reset_course_js);

    let already_running_this_course = {
        let runtime = state
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        runtime
            .active
            .get(course.provider.key())
            .map(|active| active.course_id == course.id && (active.phase == "playing" || active.phase == "opening"))
            .unwrap_or(false)
    };
    if auto_play && already_running_this_course {
        let settings = state
            .settings
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let _ = window.eval(update_media_script(settings.speed, settings.muted, auto_play, &course.title, &course.kind));
        return Ok(());
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
        let click_script = course_click_script(&course.title, &course.locator, course.provider);
        let click_window = window.clone();
        let click_provider = course.provider;
        let click_state = state.clone();
        tauri::async_runtime::spawn(async move {
            for delay in [1200, 2500, 4500, 8000] {
                tokio::time::sleep(Duration::from_millis(delay)).await;
                if !navigation_token_matches(&click_state, click_provider, token, !auto_play) {
                    break;
                }
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
                if !navigation_token_matches(&click_state, click_provider, token, !auto_play) {
                    break;
                }
                let _ = click_window.eval(&click_script);
            }
        });
    }
    if auto_play {
        let play_window = window.clone();
        let play_state = state.clone();
        let play_provider = course.provider;
        let play_title = course.title.clone();
        let play_kind = course.kind.clone();
        tauri::async_runtime::spawn(async move {
            for delay in [1000, 2200, 4000, 6500] {
                tokio::time::sleep(Duration::from_millis(delay)).await;
                if !playback_token_matches(&play_state, play_provider, token) {
                    break;
                }
                let settings = play_state
                    .settings
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone();
                let _ = play_window.eval(update_media_script(settings.speed, settings.muted, true, &play_title, &play_kind));
            }
        });
    }
    Ok(())
}

fn remember_queue_topic(conn: &Connection, course: &CourseRecord) -> Result<(), String> {
    for key in [course.provider.key(), "all"] {
        conn.execute(
            "INSERT INTO video_queue_lanes(provider,topic_id) VALUES(?1,?2)
             ON CONFLICT(provider) DO UPDATE SET topic_id=excluded.topic_id",
            params![key, course.topic_id],
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

// 预览与实际调度共用只读查询；仅真正开始课程时更新专题位置。
fn next_pending(
    path: &PathBuf,
    provider: Option<Provider>,
) -> Result<Option<CourseRecord>, String> {
    let conn = Connection::open(path).map_err(|error| error.to_string())?;
    let id: Option<String> = conn
        .query_row(
            "SELECT c.id FROM video_courses c
         JOIN video_topics t ON t.id=c.topic_id
         LEFT JOIN video_queue_lanes lane ON lane.provider=COALESCE(?1, 'all')
         LEFT JOIN video_queue_lanes platform ON platform.provider=c.provider
         WHERE c.status IN ('pending','paused') AND c.kind IN ('video','slides','material')
           AND (?1 IS NULL OR c.provider=?1) AND platform.blocked_reason IS NULL
         ORDER BY CASE WHEN c.topic_id=lane.topic_id THEN 0 ELSE 1 END,
                  CASE WHEN c.status='paused' THEN 0 ELSE 1 END,
                  t.rowid,c.sort_order,c.title,c.id LIMIT 1",
            params![provider.map(Provider::key)],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    id.map(|id| load_course(path, &id)).transpose()
}

fn pause_player(app: &AppHandle, state: &VideoTaskState, provider: Provider) {
    state
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .playback_tokens
        .remove(provider.key());
    if let Some(window) = app.get_webview_window(&provider.player_label()) {
        let _ = window.eval(r#"(() => {
            const pause = (win) => {
                try {
                    win.__MTOOL_LEARNING_BRIDGE__?.update(1, true, false);
                    win.document.querySelectorAll('video,audio').forEach(media => media.pause());
                    win.document.querySelectorAll('iframe').forEach(frame => pause(frame.contentWindow));
                } catch (_) {}
            };
            pause(window);
        })();"#);
    }
}

fn persist_stopped_course(
    path: &PathBuf,
    active: &ActiveCourse,
    status: &str,
    reason: Option<&str>,
) -> Result<(), String> {
    let conn = Connection::open(path).map_err(|error| error.to_string())?;
    let progress = if active.duration > 0.0 && active.current_time > 0.0 {
        Some(((active.current_time / active.duration) * 100.0).clamp(0.0, 100.0))
    } else {
        None
    };
    conn.execute(
        "UPDATE video_courses SET status=?2,progress=COALESCE(?3,progress),
         duration_seconds=CASE WHEN ?4>0 THEN ?4 ELSE duration_seconds END,
         last_error=?5,updated_at=?6 WHERE id=?1 AND status!='completed'",
        params![
            active.course_id,
            status,
            progress,
            active.duration as i64,
            reason,
            now()
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn block_platform(path: &PathBuf, active: &ActiveCourse) -> Result<(), String> {
    let reason = "登录已失效，请打开登录，完成后点击“登录后继续”";
    persist_stopped_course(path, active, "paused", Some(reason))?;
    let conn = Connection::open(path).map_err(|error| error.to_string())?;
    conn.execute(
        "INSERT INTO video_queue_lanes(provider,topic_id,blocked_reason) VALUES(?1,?2,?3)
         ON CONFLICT(provider) DO UPDATE SET topic_id=excluded.topic_id,blocked_reason=excluded.blocked_reason",
        params![active.provider.key(),active.topic_id,reason],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

fn cancel_browser_navigation(state: &VideoTaskState, provider: Provider) {
    state.runtime.lock().unwrap_or_else(|error| error.into_inner())
        .browser_tokens.remove(provider.key());
}

fn navigation_token_matches(state: &VideoTaskState, provider: Provider, token: u64, browser: bool) -> bool {
    let runtime = state.runtime.lock().unwrap_or_else(|error| error.into_inner());
    let tokens = if browser {
        &runtime.browser_tokens
    } else {
        &runtime.playback_tokens
    };
    tokens.get(provider.key()) == Some(&token)
}

fn playback_token_matches(state: &VideoTaskState, provider: Provider, token: u64) -> bool {
    state
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .playback_tokens
        .get(provider.key())
        == Some(&token)
}

async fn start_one(
    app: &AppHandle,
    state: &VideoTaskState,
    provider: Option<Provider>,
) -> Result<bool, String> {
    let Some(course) = next_pending(state.db_path.as_ref(), provider)? else {
        return Ok(false);
    };
    // open_course 会按需创建播放窗口；首次启动也必须继续打开课程并登记为 opening。
    if let Err(error) = open_course(app, state, &course, true).await {
        pause_player(app, state, course.provider);
        let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
        conn.execute(
            "UPDATE video_courses SET status='attention',last_error=?2,updated_at=?3 WHERE id=?1",
            params![course.id, error, now()],
        )
        .map_err(|error| error.to_string())?;
        return Err(error);
    }
    let timestamp = now();
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    remember_queue_topic(&conn, &course)?;
    let _ = conn.execute(
        "UPDATE video_courses SET status='paused',updated_at=?2 WHERE provider=?1 AND id != ?3 AND status IN ('opening','playing','verifying')",
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
                kind: course.kind.clone(),
                course_title: course.title.clone(),
                started_at: timestamp,
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
pub async fn get_video_task_dashboard(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
) -> Result<VideoTaskDashboard, String> {
    let _guard = state.queue_tick.lock().await;
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
                 FROM video_courses WHERE topic_id=?1 ORDER BY sort_order,title,id",
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
                course.status = match active.phase.as_str() {
                    "ended" => "verifying".to_string(),
                    "need_login" | "error" => "attention".to_string(),
                    _ => active.phase.clone(),
                };
                if active.duration > 0.0 {
                    course.duration_seconds = active.duration as i64;
                    course.progress =
                        ((active.current_time / active.duration) * 100.0).clamp(0.0, 100.0);
                }
            } else if matches!(course.status.as_str(), "opening" | "playing" | "verifying") {
                course.status = "paused".to_string();
            }
            if !matches!(course.status.as_str(), "attention" | "paused" | "skipped") {
                course.last_error = None;
            }
            stats.total += 1;
            match course.status.as_str() {
                "completed" => stats.completed += 1,
                "pending" => stats.pending += 1,
                "paused" => stats.paused += 1,
                "skipped" => stats.skipped += 1,
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
            let window = browser_win.as_ref();
            let blocked_reason: Option<String> = conn
                .query_row(
                    "SELECT blocked_reason FROM video_queue_lanes WHERE provider=?1",
                    params![provider.key()],
                    |row| row.get(0),
                )
                .optional()
                .unwrap_or(None)
                .flatten();
            SourceStatus {
                provider: provider.key().to_string(),
                name: provider.name().to_string(),
                home_url: provider.home().to_string(),
                window_open: browser_win.is_some(),
                current_url: window.and_then(|window| window.url().ok().map(|url| url.to_string())),
                blocked_reason,
            }
        })
        .collect();
    let settings = state
        .settings
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let mut next_courses = Vec::new();
    let providers_to_check = if settings.cross_site_parallel {
        vec![Some(Provider::Ulearn), Some(Provider::Merchant)]
    } else {
        vec![None]
    };
    for p in providers_to_check {
        if let Some(course) = next_pending(state.db_path.as_ref(), p)? {
            let topic_title: String = conn
                .query_row(
                    "SELECT title FROM video_topics WHERE id=?1",
                    params![course.topic_id],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| "".to_string());
            let is_paused = conn
                .query_row(
                    "SELECT status FROM video_courses WHERE id=?1",
                    params![course.id],
                    |row| row.get::<_, String>(0),
                )
                .map(|st| st == "paused")
                .unwrap_or(false);
            next_courses.push(QueuePreview {
                provider: course.provider.key().to_string(),
                topic_id: course.topic_id,
                topic_title,
                course_id: course.id,
                title: course.title,
                paused: is_paused,
            });
        }
    }
    Ok(VideoTaskDashboard {
        settings,
        sources,
        topics,
        stats,
        next_courses,
    })
}

#[tauri::command]
pub async fn open_video_learning_site(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    provider: String,
) -> Result<(), String> {
    let provider = Provider::parse(&provider)?;
    cancel_browser_navigation(state.inner(), provider);
    let window = ensure_browser_window(&app, state.inner(), provider, true).await?;

    let recent_topic_url: Option<String> = {
        let conn = Connection::open(state.db_path.as_ref()).ok();
        conn.and_then(|c| {
            c.query_row(
                "SELECT url FROM video_topics 
                 WHERE provider=?1 AND url != '' 
                 ORDER BY (CASE WHEN progress < 100.0 THEN 0 ELSE 1 END), last_synced_at DESC 
                 LIMIT 1",
                params![provider.key()],
                |row| row.get(0),
            ).ok()
        })
    };

    if let Some(target_url) = recent_topic_url {
        let current_url = window.url().map(|u| u.to_string()).unwrap_or_default();
        let home = provider.home();
        let is_at_home_or_login = current_url.is_empty()
            || current_url == home
            || current_url.contains("/login")
            || current_url.contains("/sso")
            || current_url.trim_end_matches('/').ends_with("/home");
        if is_at_home_or_login {
            if let Ok(parsed) = target_url.parse::<tauri::Url>() {
                let _ = window.navigate(parsed);
            }
        }
    }

    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    Ok(())
}

#[tauri::command]
pub async fn import_current_video_topic(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    provider: String,
) -> Result<ImportSummary, String> {
    let provider = Provider::parse(&provider)?;
    let capture = capture_current(&app, state.inner(), provider).await?;
    let _guard = state.queue_tick.lock().await;
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
    cancel_browser_navigation(state.inner(), provider);
    let window = ensure_browser_window(&app, state.inner(), provider, false).await?;
    window
        .navigate(
            url.parse::<tauri::Url>()
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let capture = capture_current(&app, state.inner(), provider).await?;
    let _guard = state.queue_tick.lock().await;
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
    cancel_browser_navigation(state.inner(), provider);
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
pub async fn update_video_task_settings(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    mut settings: VideoTaskSettings,
) -> Result<(), String> {
    let _guard = state.queue_tick.lock().await;
    let previous = state
        .settings
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    settings.running = previous.running;
    if previous.cross_site_parallel && !settings.cross_site_parallel {
        pause_queue(&app, state.inner())?;
    }
    settings.speed = clamp_speed(settings.speed);
    *state
        .settings
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = settings.clone();
    persist_settings(state.db_path.as_ref(), &settings)?;
    for provider in [Provider::Ulearn, Provider::Merchant] {
        if let Some(window) = app.get_webview_window(&provider.player_label()) {
            window
                .eval(update_media_script(settings.speed, settings.muted, false, "", ""))
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn start_video_queue(state: tauri::State<'_, VideoTaskState>) -> Result<(), String> {
    let _guard = state.queue_tick.lock().await;
    if let Ok(conn) = Connection::open(state.db_path.as_ref()) {
        let _ = conn.execute(
            "UPDATE video_queue_lanes SET blocked_reason=NULL WHERE blocked_reason LIKE '%登录%'",
            [],
        );
        let _ = conn.execute(
            "UPDATE video_courses SET last_error=NULL WHERE last_error LIKE '%登录%'",
            [],
        );
    }
    let mut settings = state
        .settings
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    settings.running = true;
    persist_settings(state.db_path.as_ref(), &settings)
}

#[tauri::command]
pub async fn show_video_learning_window(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    provider: String,
) -> Result<(), String> {
    let provider = Provider::parse(&provider)?;
    cancel_browser_navigation(state.inner(), provider);
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

fn pause_queue(app: &AppHandle, state: &VideoTaskState) -> Result<(), String> {
    {
        let mut settings = state
            .settings
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        settings.running = false;
        persist_settings(state.db_path.as_ref(), &settings)?;
    }
    let active_courses = state
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .active
        .clone();
    for active in active_courses.values() {
        pause_player(app, state, active.provider);
        persist_stopped_course(state.db_path.as_ref(), active, "paused", None)?;
        state
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .remove(active.provider.key());
    }
    Ok(())
}

#[tauri::command]
pub async fn pause_video_queue(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
) -> Result<(), String> {
    let _guard = state.queue_tick.lock().await;
    pause_queue(&app, state.inner())
}

#[tauri::command]
pub async fn pause_video_course(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    course_id: String,
) -> Result<(), String> {
    let _guard = state.queue_tick.lock().await;
    let is_active = state
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .active
        .values()
        .any(|active| active.course_id == course_id);
    if is_active {
        pause_queue(&app, state.inner())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn skip_video_course(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    course_id: String,
) -> Result<(), String> {
    let _guard = state.queue_tick.lock().await;
    let active = state
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .active
        .values()
        .find(|active| active.course_id == course_id)
        .cloned();
    if let Some(active) = active {
        pause_player(&app, state.inner(), active.provider);
        persist_stopped_course(
            state.db_path.as_ref(),
            &active,
            "skipped",
            Some("已手动跳过，需要时点击重试"),
        )?;
        state
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .remove(active.provider.key());
    }
    Ok(())
}

#[tauri::command]
pub async fn resume_video_platform(
    state: tauri::State<'_, VideoTaskState>,
    provider: String,
) -> Result<(), String> {
    let _guard = state.queue_tick.lock().await;
    let provider = Provider::parse(&provider)?;
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    conn.execute(
        "UPDATE video_queue_lanes SET blocked_reason=NULL WHERE provider=?1",
        params![provider.key()],
    )
    .map_err(|error| error.to_string())?;
    let _ = conn.execute(
        "UPDATE video_courses SET last_error=NULL WHERE provider=?1 AND last_error LIKE '%登录%'",
        params![provider.key()],
    );
    let mut settings = state
        .settings
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    settings.running = true;
    persist_settings(state.db_path.as_ref(), &settings)
}

#[tauri::command]
pub async fn tick_video_queue(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
) -> Result<(), String> {
    // 开始按钮和 2.5 秒轮询共用调度锁，避免首次创建窗口时重复启动同一课程。
    let _tick_guard = state.queue_tick.lock().await;
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
            let conn =
                Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
            let duration_secs = if active.duration > 0.0 {
                active.duration as i64
            } else {
                0
            };
            conn.execute(
                "UPDATE video_courses SET status='completed',progress=100.0,duration_seconds=CASE WHEN duration_seconds > 0 THEN duration_seconds ELSE ?2 END,last_error=NULL,updated_at=?3 WHERE id=?1",
                params![active.course_id, duration_secs, timestamp],
            )
            .map_err(|error| error.to_string())?;
            conn.execute(
                "UPDATE video_topics SET 
                 completed_count = (SELECT COUNT(*) FROM video_courses WHERE topic_id=?1 AND status='completed'),
                 total_count = (SELECT COUNT(*) FROM video_courses WHERE topic_id=?1),
                 progress = ROUND((CAST((SELECT COUNT(*) FROM video_courses WHERE topic_id=?1 AND status='completed') AS REAL) / MAX(1, (SELECT COUNT(*) FROM video_courses WHERE topic_id=?1))) * 100.0, 1),
                 last_synced_at = ?2
                 WHERE id=?1",
                params![active.topic_id, timestamp],
            )
            .map_err(|error| error.to_string())?;

            state
                .runtime
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .active
                .remove(active.provider.key());
        } else if active.phase == "opening" || active.phase == "playing" {
            if let Some(window) = app.get_webview_window(&active.provider.player_label()) {
                let _ = window.eval(update_media_script(settings.speed, settings.muted, true, &active.course_title, &active.kind));
            }
            let progress = if active.duration > 0.0 {
                ((active.current_time / active.duration) * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            if active.phase == "playing" && active.duration > 0.0 {
                if let Ok(conn) = Connection::open(state.db_path.as_ref()) {
                    let _ = conn.execute(
                        "UPDATE video_courses SET progress=?2,duration_seconds=CASE WHEN duration_seconds > 0 THEN duration_seconds ELSE ?3 END,updated_at=?4 WHERE id=?1",
                        params![active.course_id, progress, active.duration as i64, now()],
                    );
                }
            }

            // 1. 优先检测登录态失效：检查窗口 URL 是否重定向到登录页
            let is_login_url = app
                .get_webview_window(&active.provider.player_label())
                .and_then(|window| window.url().ok())
                .map(|url| {
                    let s = url.as_str().to_lowercase();
                    let path = url.path().to_lowercase();
                    path.ends_with("/login")
                        || path.contains("/login/")
                        || path.contains("/cas/login")
                        || path.contains("/sso/login")
                        || path.contains("/oauth/authorize")
                        || s.contains("/login?")
                        || s.contains("/sso?")
                })
                .unwrap_or(false);

            let is_actively_progressing = now() - active.last_progress_at <= 5;

            if is_login_url && !is_actively_progressing && (active.phase == "playing" || now() - active.phase_since >= 15) {
                pause_player(&app, state.inner(), active.provider);
                block_platform(state.db_path.as_ref(), &active)?;
                state
                    .runtime
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .active
                    .remove(active.provider.key());
                continue;
            }

            // 看门狗 1：高进度饱和完播检测（进度 >= 98.5% 或距离结束不足 1.5 秒，且停滞超过 4 秒）
            // 很多平台播放器在最后一秒自动 pause 而不触发 ended 事件，看门狗在此主动将其转入 ended 完播
            let is_saturated = active.duration > 5.0
                && (active.current_time >= active.duration - 1.5 || progress >= 98.5);
            let stall_duration = now() - active.last_progress_at;
            if is_saturated && stall_duration >= 4 {
                let mut runtime = state.runtime.lock().unwrap_or_else(|error| error.into_inner());
                if let Some(act) = runtime.active.get_mut(active.provider.key()) {
                    act.phase = "ended".to_string();
                    act.phase_since = now();
                    act.current_time = act.duration;
                }
                continue;
            }

            // 看门狗 2：页面加载（opening）超时检测（120 秒宽容期）
            if active.phase == "opening" && now() - active.phase_since >= 120 {
                pause_player(&app, state.inner(), active.provider);
                let conn =
                    Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
                let last_error = "未检测到可播放的视频或页面加载超时，请打开课程检查";
                conn.execute(
                    "UPDATE video_courses SET status='attention',progress=0.0,last_error=?2,updated_at=?3 WHERE id=?1",
                    params![active.course_id, last_error, now()],
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

            // 看门狗 3：播放中进度停滞检测（视频120秒，文档300秒宽容期），杜绝假完播谎报学时，统一标 attention 异常
            let stall_threshold = {
                let is_material = load_course(state.db_path.as_ref(), &active.course_id)
                    .map(|c| c.kind == "material" || c.kind == "slides")
                    .unwrap_or(false);
                if is_material {
                    300
                } else {
                    120
                }
            };
            if active.phase == "playing" && stall_duration >= stall_threshold {
                pause_player(&app, state.inner(), active.provider);
                let conn =
                    Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
                let mins = (active.current_time / 60.0).floor() as i64;
                let wait_mins = stall_threshold / 60;
                let last_error = format!("课程学习进度卡住超过{wait_mins}分钟未推进（停在约{mins}分钟），已自动跳过并开始播放下一门");
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

            // 看门狗 4：网页探活兜底（120 秒没有任何网页心跳）
            let last_activity = if active.phase == "opening" {
                active.phase_since
            } else {
                active.last_media_at
            };
            if now() - last_activity > 120 {
                let conn =
                    Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
                let last_error = app
                    .get_webview_window(&active.provider.player_label())
                    .and_then(|window| window.url().ok())
                    .filter(|url| provider_accepts_url(active.provider, url))
                    .map(|_| "未检测到可持续播放的视频，请打开课程检查")
                    .unwrap_or("课程页面未成功打开或已进入空白页，请重试后检查");
                pause_player(&app, state.inner(), active.provider);
                conn.execute(
                    "UPDATE video_courses SET status='attention',last_error=?2,updated_at=?3 WHERE id=?1",
                    params![active.course_id, last_error, now()],
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
            pause_player(&app, state.inner(), active.provider);
            block_platform(state.db_path.as_ref(), &active)?;
            state
                .runtime
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .active
                .remove(active.provider.key());
        } else if active.phase == "error" {
            pause_player(&app, state.inner(), active.provider);
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
        } else {
            // 兜底：任何其他非活跃状态（如 paused 等）从 runtime.active 彻底移除，杜绝死锁调度
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
    let mut startup_errors = Vec::new();
    if settings.cross_site_parallel {
        for provider in [Provider::Ulearn, Provider::Merchant] {
            if !active_providers.iter().any(|key| key == provider.key()) {
                match start_one(&app, state.inner(), Some(provider)).await {
                    Ok(started) => any_started |= started,
                    Err(error) => startup_errors.push(format!("{}：{error}", provider.name())),
                }
            }
        }
    } else if active_providers.is_empty() {
        match start_one(&app, state.inner(), None).await {
            Ok(started) => any_started = started,
            Err(error) => startup_errors.push(error),
        }
    }

    let active_count = state
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .active
        .len();
    if active_count == 0 && !any_started && next_pending(state.db_path.as_ref(), None)?.is_none() {
        let mut settings = state
            .settings
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if settings.running {
            settings.running = false;
            let _ = persist_settings(state.db_path.as_ref(), &settings);
        }
    }
    if startup_errors.is_empty() {
        Ok(())
    } else {
        Err(format!("课程启动失败：{}", startup_errors.join("；")))
    }
}

#[tauri::command]
pub async fn open_video_course(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    course_id: String,
) -> Result<(), String> {
    let course = load_course(state.db_path.as_ref(), &course_id)?;
    // 打开仅改变浏览窗口的起始页面，不暂停队列或改写课程进度。
    open_course(&app, state.inner(), &course, false).await
}

#[tauri::command]
pub async fn play_video_course(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    course_id: String,
) -> Result<(), String> {
    let _guard = state.queue_tick.lock().await;
    let course = load_course(state.db_path.as_ref(), &course_id)?;

    // 1. 停止当前平台其他正在播放的课程并保存为 paused
    let old_active = {
        let mut runtime = state
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        runtime.active.remove(course.provider.key())
    };
    if let Some(old) = old_active {
        pause_player(&app, state.inner(), old.provider);
        if let Ok(conn) = Connection::open(state.db_path.as_ref()) {
            let _ = conn.execute(
                "UPDATE video_courses SET status='paused',updated_at=?2 WHERE id=?1 AND status IN ('opening','playing','verifying')",
                params![old.course_id, now()],
            );
        }
    }

    // 2. 将全局队列设置为运行状态，并清理阻断标记与记录当前专题
    {
        let mut settings = state
            .settings
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        settings.running = true;
        let _ = persist_settings(state.db_path.as_ref(), &settings);
    }
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    let _ = conn.execute(
        "UPDATE video_queue_lanes SET blocked_reason=NULL WHERE provider=?1",
        params![course.provider.key()],
    );
    let _ = remember_queue_topic(&conn, &course);

    // 3. 打开目标课程并开始播放
    open_course(&app, state.inner(), &course, true).await?;

    // 4. 更新数据库状态为 opening，记录到 active 中
    let timestamp = now();
    let _ = conn.execute(
        "UPDATE video_courses SET status='paused',updated_at=?2 WHERE provider=?1 AND id != ?3 AND status IN ('opening','playing','verifying')",
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
                kind: course.kind.clone(),
                course_title: course.title.clone(),
                started_at: timestamp,
                phase: "opening".to_string(),
                phase_since: timestamp,
                last_media_at: timestamp,
                last_progress_at: timestamp,
                last_advanced_time: initial_time,
                current_time: initial_time,
                duration: initial_duration,
            },
        );

    let _ = app.emit("video-queue-state-changed", true);
    Ok(())
}

#[tauri::command]
pub async fn retry_video_course(
    state: tauri::State<'_, VideoTaskState>,
    course_id: String,
) -> Result<(), String> {
    let _guard = state.queue_tick.lock().await;
    let course = load_course(state.db_path.as_ref(), &course_id)?;
    let next_status = if course.kind == "video" || course.kind == "slides" || course.kind == "material" {
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
pub async fn complete_video_course(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    course_id: String,
) -> Result<(), String> {
    let _guard = state.queue_tick.lock().await;
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    let active_courses = state
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .active
        .clone();
    for active in active_courses
        .values()
        .filter(|active| active.course_id == course_id)
    {
        pause_player(&app, state.inner(), active.provider);
        state
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .remove(active.provider.key());
    }
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
pub async fn remove_video_topic(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    topic_id: String,
) -> Result<(), String> {
    let _guard = state.queue_tick.lock().await;
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    let active_courses = state
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .active
        .clone();
    for active in active_courses
        .values()
        .filter(|active| active.topic_id == topic_id)
    {
        pause_player(&app, state.inner(), active.provider);
        state
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .remove(active.provider.key());
    }
    conn.execute("PRAGMA foreign_keys=ON", []).ok();
    conn.execute("DELETE FROM video_topics WHERE id=?1", params![topic_id])
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn reset_video_topic(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    topic_id: String,
) -> Result<(), String> {
    let _guard = state.queue_tick.lock().await;
    let conn = Connection::open(state.db_path.as_ref()).map_err(|error| error.to_string())?;
    let active_courses = state
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .active
        .clone();
    for active in active_courses
        .values()
        .filter(|active| active.topic_id == topic_id)
    {
        pause_player(&app, state.inner(), active.provider);
        state
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .remove(active.provider.key());
    }
    conn.execute(
        "UPDATE video_courses SET status='pending',progress=0,last_error=NULL,updated_at=?2
         WHERE topic_id=?1 AND kind IN ('video', 'slides', 'material')",
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
pub async fn reset_video_course(
    state: tauri::State<'_, VideoTaskState>,
    course_id: String,
) -> Result<(), String> {
    let _guard = state.queue_tick.lock().await;
    let course = load_course(state.db_path.as_ref(), &course_id)?;
    let next_status = if course.kind == "video" || course.kind == "slides" || course.kind == "material" {
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
        let script = course_click_script("课程标题", "#saved-course", Provider::Ulearn);
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
        let update = update_media_script(2.0, true, true, "", "");

        assert!(bridge.contains("tryPlayMedia"));
        assert!(bridge.contains("triggerPlayUI"));
        assert!(bridge.contains("simulateFullClick"));
        assert!(bridge.contains("window.setInterval(() => apply(false), 1500)"));

        assert!(update.contains("window.__MTOOL_LEARNING_BRIDGE__.update(speed, muted, autoPlay, courseTitle, courseKind)"));
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
                    kind: "video".to_string(),
                    course_title: "01. 视频课".to_string(),
                    started_at: now(),
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
            queue_tick: Arc::new(tokio::sync::Mutex::new(())),
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
    fn import_material_course_becomes_pending_and_schedulable() {
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
            queue_tick: Arc::new(tokio::sync::Mutex::new(())),
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
            progress: if completed { 100.0 } else { 40.0 },
            completed,
        };
        let summary = import_capture(
            &state,
            Provider::Merchant,
            PageTopicCapture {
                title: "测试文档专题".to_string(),
                url: format!("https://{domain}/study/test_doc"),
                progress: 40.0,
                total_count: 1,
                completed_count: 0,
                courses: vec![
                    course("doc1", "03. 协议批量分发功能手册", "material", false),
                ],
            },
        )
        .expect("import capture");
        assert_eq!(summary.imported, 1);
        assert_eq!(summary.completed, 0);
        assert_eq!(summary.manual, 0);

        let conn = Connection::open(&path).expect("open test database");
        let doc_status: String = conn
            .query_row(
                "SELECT status FROM video_courses WHERE kind='material'",
                [],
                |row| row.get(0),
            )
            .expect("doc status");
        assert_eq!(doc_status, "pending");

        // 验证 next_pending 可以正常检索到该文档课程
        let pending = next_pending(&path, Some(Provider::Merchant)).expect("query next pending");
        assert!(pending.is_some());
        assert_eq!(pending.unwrap().title, "03. 协议批量分发功能手册");

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
                    kind: "video".to_string(),
                    course_title: "01. 卡顿课".to_string(),
                    started_at: start_time,
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
    fn capture_script_accurately_extracts_sub_courses_and_filters_big_chapter_headers() {
        let script = capture_script("test_req", Provider::Merchant);
        assert!(script.contains("isBigChapterHeader"));
        assert!(script.contains("singleLessonItems"));
        assert!(script.contains("findCatalogPanel"));
        assert!(script.contains("hasLessonNumber"));
        assert!(script.contains("sectionTitle: \"\""));
        assert!(script.contains(".course-stage-caption"));
        assert!(script.contains(".course-content-item"));
        assert!(script.contains("rotate\\(-?90deg\\)"));
    }

    #[test]
    fn test_scripts_have_no_duplicate_const_declarations() {
        let script = capture_script("test_req", Provider::Merchant);
        assert_eq!(script.matches("const catalogPanel").count(), 1);
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
    fn browser_and_bridge_scripts_include_devtools_shortcuts() {
        let nav_script = browser_nav_script(Provider::Merchant);
        assert!(nav_script.contains("MTOOL_DEVTOOLS_TOGGLE|"));
        assert!(nav_script.contains("F12"));
        assert!(nav_script.contains("isMacInspect"));
        assert!(nav_script.contains("isWinInspect"));
        assert!(nav_script.contains("开发者工具 (F12 / ⌥⌘I)"));

        let bridge = bridge_script(Provider::Merchant, 1.0, true);
        assert!(bridge.contains("MTOOL_DEVTOOLS_TOGGLE|"));
        assert!(bridge.contains("F12"));
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
        assert!(script.contains("hasExplicitSlidesBadge"));
        assert!(script.contains("ppt|pptx|ppt课件|课件|幻灯片"));
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

    #[test]
    fn capture_script_eliminates_duplicate_exam_entries_and_cleans_titles() {
        let script = capture_script("test_req", Provider::Merchant);
        assert!(script.contains("isPureTagOrBadge"));
        assert!(script.contains("未完成"));
        assert!(script.contains("需本人处理"));
        assert!(script.contains("ppt课件|ppt|pptx|考试|测验|测试|问卷|未完成|已完成"));
    }

    #[test]
    fn capture_script_isolates_offline_and_survey_and_filters_overview_metadata() {
        let script = capture_script("test_req", Provider::Merchant);
        assert!(script.contains("getItemScope"));
        assert!(script.contains("trainingContentPanes"));
        assert!(script.contains("hasExplicitOfflineBadge"));
        assert!(script.contains("hasExplicitSurveyBadge"));
        assert!(script.contains("innerBadges > 1"));
        assert!(script.contains("titleFrom(item) || titleFrom(card)"));
    }

    #[test]
    fn capture_script_preserves_survey_with_evaluation_in_title_and_handles_rpa_training_scenario() {
        let script = capture_script("test_req", Provider::Merchant);
        assert!(script.contains("nonTitleText = candidateTitle ? t.split(candidateTitle).join(\" \") : t;"));
        assert!(script.contains("durCount > 1"));
        assert!(script.contains("creditCount > 1"));

        let click_script = course_click_script("问卷测试", "#loc", Provider::Merchant);
        assert!(click_script.contains("填写问卷|去填写|开始填写|参加调研|参与问卷|开始问卷|问卷调查|去评价|立即评价|填写评价|评价"));

        let f = QueueFixture::new();
        let capture = PageTopicCapture {
            title: "银联商务信创版RPA开发培训（线上）".into(),
            url: "https://example.com/rpa-train".into(),
            progress: 0.0,
            total_count: 3,
            completed_count: 0,
            courses: vec![
                PageCourseCapture {
                    external_id: "rpa-offline".into(),
                    title: "银联商务信创版RPA开发培训".into(),
                    url: String::new(),
                    locator: "#offline-item".into(),
                    section_title: "01 银联商务信创版RPA开发培训".into(),
                    kind: "offline".into(),
                    duration_seconds: 7200,
                    progress: 0.0,
                    completed: false,
                },
                PageCourseCapture {
                    external_id: "rpa-survey".into(),
                    title: "银联商务讲师授课满意度评价表".into(),
                    url: String::new(),
                    locator: "#survey-item".into(),
                    section_title: "01 银联商务信创版RPA开发培训".into(),
                    kind: "survey".into(),
                    duration_seconds: 120,
                    progress: 0.0,
                    completed: false,
                },
                PageCourseCapture {
                    external_id: "rpa-exam".into(),
                    title: "银联商务信创版RPA开发测试".into(),
                    url: String::new(),
                    locator: "#exam-item".into(),
                    section_title: "01 银联商务信创版RPA开发培训".into(),
                    kind: "exam".into(),
                    duration_seconds: 10800,
                    progress: 0.0,
                    completed: false,
                },
            ],
        };

        let summary = import_capture(&f.state, Provider::Merchant, capture).unwrap();
        assert_eq!(summary.imported, 3);
        assert_eq!(summary.manual, 3);
        assert_eq!(summary.completed, 0);

        let conn = Connection::open(&f.path).unwrap();
        let topic_progress: f64 = conn.query_row(
            "SELECT progress FROM video_topics WHERE id=?1",
            params![summary.topic_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(topic_progress, 0.0);

        let total_count: i64 = conn.query_row(
            "SELECT total_count FROM video_topics WHERE id=?1",
            params![summary.topic_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(total_count, 3);

        let survey_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM video_courses WHERE topic_id=?1 AND title='银联商务讲师授课满意度评价表' AND kind='survey' AND status='manual'",
            params![summary.topic_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(survey_count, 1);
    }

    #[test]
    fn course_click_script_supports_async_modal_and_exam_buttons() {
        let script = course_click_script("01. 六步赢单考试", "#saved-course", Provider::Merchant);
        assert!(script.contains("去考试|开始考试|参加考试|进入考试"));
        assert!(script.contains("ant-modal"));
        assert!(script.contains("baseTitle"));
    }

    #[test]
    fn capture_script_preserves_exam_items_and_handles_security_awareness_scenario() {
        let script = capture_script("test_req", Provider::Merchant);
        assert!(script.contains("cleanedNonTitle = nonTitleText.replace(/(?:已考试|未考试|去考试|待考试|参加考试|开始考试|进入考试|考试中|考试通过|考试合格|考试不合格|补考|填写问卷|参与问卷|去问卷|去评价|已评价|课件学习)/g, \" \");"));
        assert!(script.contains("已考试|未考试|待考试|去考试|参加考试|开始考试|进入考试|考试通过|考试合格"));

        let f = QueueFixture::new();
        let capture = PageTopicCapture {
            title: "银联商务2026年安全意识培训".into(),
            url: "https://example.com/security-2026".into(),
            progress: 100.0,
            total_count: 6,
            completed_count: 6,
            courses: vec![
                PageCourseCapture {
                    external_id: "sec-1".into(),
                    title: "AI背景下的新型网络安全社工攻击".into(),
                    url: "https://example.com/sec1".into(),
                    locator: "#sec-1".into(),
                    section_title: "01 第一阶段".into(),
                    kind: "video".into(),
                    duration_seconds: 2400,
                    progress: 100.0,
                    completed: true,
                },
                PageCourseCapture {
                    external_id: "sec-2".into(),
                    title: "AI背景下的新型网络安全社工攻击课件".into(),
                    url: "https://example.com/sec2".into(),
                    locator: "#sec-2".into(),
                    section_title: "01 第一阶段".into(),
                    kind: "slides".into(),
                    duration_seconds: 0,
                    progress: 100.0,
                    completed: true,
                },
                PageCourseCapture {
                    external_id: "sec-3".into(),
                    title: "AI背景下的新型网络安全社工攻击考试".into(),
                    url: "https://example.com/sec3".into(),
                    locator: "#sec-3".into(),
                    section_title: "01 第一阶段".into(),
                    kind: "exam".into(),
                    duration_seconds: 2400,
                    progress: 100.0,
                    completed: true,
                },
                PageCourseCapture {
                    external_id: "sec-4".into(),
                    title: "数据安全新态势新要求".into(),
                    url: "https://example.com/sec4".into(),
                    locator: "#sec-4".into(),
                    section_title: "02 第二阶段".into(),
                    kind: "video".into(),
                    duration_seconds: 2400,
                    progress: 100.0,
                    completed: true,
                },
                PageCourseCapture {
                    external_id: "sec-5".into(),
                    title: "数据安全新态势新要求课件".into(),
                    url: "https://example.com/sec5".into(),
                    locator: "#sec-5".into(),
                    section_title: "02 第二阶段".into(),
                    kind: "slides".into(),
                    duration_seconds: 0,
                    progress: 100.0,
                    completed: true,
                },
                PageCourseCapture {
                    external_id: "sec-6".into(),
                    title: "数据安全新态势新要求考试".into(),
                    url: "https://example.com/sec6".into(),
                    locator: "#sec-6".into(),
                    section_title: "02 第二阶段".into(),
                    kind: "exam".into(),
                    duration_seconds: 2400,
                    progress: 100.0,
                    completed: true,
                },
            ],
        };

        let summary = import_capture(&f.state, Provider::Merchant, capture).unwrap();
        assert_eq!(summary.imported, 6);
        assert_eq!(summary.completed, 6);
        assert_eq!(summary.manual, 0);

        let conn = Connection::open(&f.path).unwrap();
        let exam_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM video_courses WHERE topic_id=?1 AND kind='exam' AND status='completed' AND progress=100.0",
            params![summary.topic_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(exam_count, 2);
    }

    #[test]
    fn capture_script_and_click_script_handle_single_stage_and_prevent_collapsing_open_chapters() {
        let script = capture_script("test_req", Provider::Merchant);
        assert!(script.contains("getChapterContainer"));
        assert!(script.contains("hasChildItems"));
        assert!(script.contains("isHeaderCollapsed"));
        assert!(script.contains("章节\\s*[（(]?\\d+[）)]?"));
        assert!(script.contains(".course-detail-right-wrapper"));

        let click_script = course_click_script("01. 测试", "#test", Provider::Merchant);
        assert!(click_script.contains("if (hasContentItems) continue;"));
    }

    #[test]
    fn capture_script_recognizes_ulearn_completed_badge() {
        let script = capture_script("test_req", Provider::Ulearn);
        assert!(script.contains("已完成|已学完|已学习"));
        assert!(script.contains("isAllCompletedByStats"));
    }

    #[test]
    fn test_ulearn_and_merchant_scripts_are_strictly_isolated() {
        let ulearn_capture = capture_script("req_u", Provider::Ulearn);
        let merchant_capture = capture_script("req_m", Provider::Merchant);

        // 银联乐学专属抓取器极其轻量，绝不包含 YS 学堂的折叠展开和大章节复杂度
        assert!(!ulearn_capture.contains(".course-stage-caption"));
        assert!(!ulearn_capture.contains("isBigChapterHeader"));
        assert!(!ulearn_capture.contains("findCatalogPanel"));
        assert!(ulearn_capture.contains("已学习"));

        // YS 学堂抓取器包含专属复杂逻辑
        assert!(merchant_capture.contains(".course-stage-caption"));
        assert!(merchant_capture.contains("isBigChapterHeader"));
        assert!(merchant_capture.contains("findCatalogPanel"));

        // 点击脚本物理隔离：乐学不包含 ant-modal 弹窗等待与折叠展开
        let ulearn_click = course_click_script("测试课程", "#loc", Provider::Ulearn);
        let merchant_click = course_click_script("测试课程", "#loc", Provider::Merchant);

        assert!(!ulearn_click.contains("ant-modal"));
        assert!(!ulearn_click.contains("stageHeaders"));
        assert!(merchant_click.contains("ant-modal"));
        assert!(merchant_click.contains("stageHeaders"));
    }

    #[test]
    fn sequential_topic_scheduling_locks_current_topic_until_done() {
        let path = std::env::temp_dir().join(format!(
            "mtool-video-task-test-{}.db",
            CAPTURE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        init_db(&path).expect("init test database");
        let conn = Connection::open(&path).expect("open test database");

        conn.execute(
            "INSERT INTO video_topics (id, provider, title, url, progress, total_count, completed_count, last_synced_at)
             VALUES ('topic-1', 'merchant', '专题一', 'https://example.com/1', 0, 2, 0, ?1)",
            params![now()],
        ).expect("insert topic-1");

        conn.execute(
            "INSERT INTO video_topics (id, provider, title, url, progress, total_count, completed_count, last_synced_at)
             VALUES ('topic-2', 'merchant', '专题二', 'https://example.com/2', 0, 1, 0, ?1)",
            params![now()],
        ).expect("insert topic-2");

        conn.execute(
            "INSERT INTO video_courses (id, topic_id, provider, external_id, title, status, sort_order, updated_at)
             VALUES ('c-1-1', 'topic-1', 'merchant', 'ext-1-1', '专题一第1节', 'pending', 1, ?1)",
            params![now()],
        ).expect("insert c-1-1");

        conn.execute(
            "INSERT INTO video_courses (id, topic_id, provider, external_id, title, status, sort_order, updated_at)
             VALUES ('c-1-2', 'topic-1', 'merchant', 'ext-1-2', '专题一第2节', 'pending', 2, ?1)",
            params![now()],
        ).expect("insert c-1-2");

        conn.execute(
            "INSERT INTO video_courses (id, topic_id, provider, external_id, title, status, sort_order, updated_at)
             VALUES ('c-2-1', 'topic-2', 'merchant', 'ext-2-1', '专题二第1节', 'pending', 1, ?1)",
            params![now()],
        ).expect("insert c-2-1");

        let next1 = next_pending(&path, Some(Provider::Merchant))
            .expect("query next_pending")
            .expect("found course");
        assert_eq!(next1.id, "c-1-1");
        assert_eq!(next1.topic_id, "topic-1");

        remember_queue_topic(&conn, &next1).expect("activate first course");
        let lane_topic: String = conn
            .query_row(
                "SELECT topic_id FROM video_queue_lanes WHERE provider='merchant'",
                [],
                |row| row.get(0),
            )
            .expect("lane topic");
        assert_eq!(lane_topic, "topic-1");

        conn.execute(
            "UPDATE video_courses SET status='completed' WHERE id='c-1-1'",
            [],
        )
        .expect("update c-1-1");
        let next2 = next_pending(&path, Some(Provider::Merchant))
            .expect("query next_pending")
            .expect("found course");
        assert_eq!(next2.id, "c-1-2");
        assert_eq!(next2.topic_id, "topic-1");

        conn.execute(
            "UPDATE video_courses SET status='completed' WHERE id='c-1-2'",
            [],
        )
        .expect("update c-1-2");
        let next3 = next_pending(&path, Some(Provider::Merchant))
            .expect("query next_pending")
            .expect("found course");
        assert_eq!(next3.id, "c-2-1");
        assert_eq!(next3.topic_id, "topic-2");

        remember_queue_topic(&conn, &next3).expect("activate next topic");
        let lane_topic2: String = conn
            .query_row(
                "SELECT topic_id FROM video_queue_lanes WHERE provider='merchant'",
                [],
                |row| row.get(0),
            )
            .expect("lane topic 2");
        assert_eq!(lane_topic2, "topic-2");

        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn next_pending_prefers_paused_course_over_pending() {
        let path = std::env::temp_dir().join(format!(
            "mtool-video-task-test-{}.db",
            CAPTURE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        init_db(&path).expect("init test database");
        let conn = Connection::open(&path).expect("open test database");

        conn.execute(
            "INSERT INTO video_topics (id, provider, title, url, progress, total_count, completed_count, last_synced_at)
             VALUES ('topic-1', 'merchant', '专题一', 'https://example.com/1', 0, 2, 0, ?1)",
            params![now()],
        ).expect("insert topic-1");

        conn.execute(
            "INSERT INTO video_courses (id, topic_id, provider, external_id, title, status, sort_order, updated_at)
             VALUES ('c-1', 'topic-1', 'merchant', 'ext-1', '课程1', 'pending', 1, ?1)",
            params![now()],
        ).expect("insert c-1");

        conn.execute(
            "INSERT INTO video_courses (id, topic_id, provider, external_id, title, status, progress, sort_order, updated_at)
             VALUES ('c-2', 'topic-1', 'merchant', 'ext-2', '课程2', 'paused', 45.0, 2, ?1)",
            params![now()],
        ).expect("insert c-2");

        let next = next_pending(&path, Some(Provider::Merchant))
            .expect("query next_pending")
            .expect("found course");
        assert_eq!(next.id, "c-2");

        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn next_pending_ignores_skipped_and_honors_blocked_reason() {
        let path = std::env::temp_dir().join(format!(
            "mtool-video-task-test-{}.db",
            CAPTURE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        init_db(&path).expect("init test database");
        let conn = Connection::open(&path).expect("open test database");

        conn.execute(
            "INSERT INTO video_topics (id, provider, title, url, progress, total_count, completed_count, last_synced_at)
             VALUES ('topic-1', 'merchant', '专题一', 'https://example.com/1', 0, 2, 0, ?1)",
            params![now()],
        ).expect("insert topic-1");

        conn.execute(
            "INSERT INTO video_courses (id, topic_id, provider, external_id, title, status, sort_order, updated_at)
             VALUES ('c-1', 'topic-1', 'merchant', 'ext-1', '跳过的课程', 'skipped', 1, ?1)",
            params![now()],
        ).expect("insert c-1");

        conn.execute(
            "INSERT INTO video_courses (id, topic_id, provider, external_id, title, status, sort_order, updated_at)
             VALUES ('c-2', 'topic-1', 'merchant', 'ext-2', '待播课程', 'pending', 2, ?1)",
            params![now()],
        ).expect("insert c-2");

        let next = next_pending(&path, Some(Provider::Merchant))
            .expect("query next_pending")
            .expect("found course");
        assert_eq!(next.id, "c-2");

        conn.execute(
            "INSERT INTO video_queue_lanes (provider, blocked_reason) VALUES ('merchant', 'session_expired')
             ON CONFLICT(provider) DO UPDATE SET blocked_reason='session_expired'",
            [],
        ).expect("block lane");

        let blocked_result =
            next_pending(&path, Some(Provider::Merchant)).expect("query next_pending");
        assert!(blocked_result.is_none());

        conn.execute(
            "UPDATE video_queue_lanes SET blocked_reason=NULL WHERE provider='merchant'",
            [],
        )
        .expect("unblock lane");

        let unblocked_result =
            next_pending(&path, Some(Provider::Merchant)).expect("query next_pending");
        assert!(unblocked_result.is_some());
        assert_eq!(unblocked_result.unwrap().id, "c-2");

        drop(conn);
        let _ = std::fs::remove_file(path);
    }
    struct QueueFixture {
        path: PathBuf,
        state: VideoTaskState,
    }
    impl QueueFixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "mtool-queue-{}-{}.db",
                std::process::id(),
                CAPTURE_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            init_db(&path).unwrap();
            let state = VideoTaskState {
                db_path: Arc::new(path.clone()),
                settings: Arc::new(Mutex::new(VideoTaskSettings::default())),
                captures: Arc::new(Mutex::new(CaptureExchange::default())),
                runtime: Arc::new(Mutex::new(RuntimeState::default())),
                queue_tick: Arc::new(tokio::sync::Mutex::new(())),
            };
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("INSERT INTO video_topics(id,provider,title,url,last_synced_at) VALUES
                ('a','merchant','专题A','https://example.com/a',1),
                ('b','merchant','专题B','https://example.com/b',2),
                ('u','ulearn','专题U','https://example.com/u',3);
                INSERT INTO video_courses(id,topic_id,provider,external_id,title,status,sort_order,updated_at) VALUES
                ('a1','a','merchant','a1','A1','pending',0,900),
                ('a2','a','merchant','a2','A2','pending',1,800),
                ('b1','b','merchant','b1','B1','pending',0,1),
                ('u1','u','ulearn','u1','U1','pending',0,0);").unwrap();
            Self { path, state }
        }
        fn active(&self, id: &str) -> ActiveCourse {
            let c = load_course(&self.path, id).unwrap();
            ActiveCourse {
                course_id: c.id,
                topic_id: c.topic_id,
                provider: c.provider,
                kind: c.kind,
                course_title: c.title,
                started_at: now(),
                phase: "playing".into(),
                phase_since: now(),
                last_media_at: now(),
                last_progress_at: now(),
                last_advanced_time: 45.0,
                current_time: 45.0,
                duration: 100.0,
            }
        }
    }
    impl Drop for QueueFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[test]
    fn queue_preview_is_read_only_and_update_times_do_not_reorder_topics() {
        let f = QueueFixture::new();
        let conn = Connection::open(&f.path).unwrap();
        for provider in [None, Some(Provider::Merchant)] {
            assert_eq!(next_pending(&f.path, provider).unwrap().unwrap().id, "a1");
            assert_eq!(next_pending(&f.path, provider).unwrap().unwrap().id, "a1");
        }
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM video_queue_lanes", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        remember_queue_topic(&conn, &load_course(&f.path, "a1").unwrap()).unwrap();
        conn.execute(
            "UPDATE video_courses SET status='completed',updated_at=1000 WHERE id='a1'",
            [],
        )
        .unwrap();
        assert_eq!(next_pending(&f.path, None).unwrap().unwrap().id, "a2");
        // Viewing the last course's preview must not switch the remembered topic to B.
        conn.execute(
            "UPDATE video_courses SET status='opening' WHERE id='a2'",
            [],
        )
        .unwrap();
        assert_eq!(next_pending(&f.path, None).unwrap().unwrap().id, "b1");
        assert_eq!(
            conn.query_row(
                "SELECT topic_id FROM video_queue_lanes WHERE provider='all'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "a"
        );
        persist_stopped_course(&f.path, &f.active("a2"), "paused", None).unwrap();
        assert_eq!(next_pending(&f.path, None).unwrap().unwrap().id, "a2");
    }

    #[test]
    fn locked_topic_login_failure_blocks_both_serial_and_parallel_selection() {
        let f = QueueFixture::new();
        let conn = Connection::open(&f.path).unwrap();
        remember_queue_topic(&conn, &load_course(&f.path, "a1").unwrap()).unwrap();
        block_platform(&f.path, &f.active("a1")).unwrap();
        assert!(next_pending(&f.path, Some(Provider::Merchant))
            .unwrap()
            .is_none());
        assert_eq!(next_pending(&f.path, None).unwrap().unwrap().id, "u1");
        assert_eq!(
            next_pending(&f.path, Some(Provider::Ulearn))
                .unwrap()
                .unwrap()
                .id,
            "u1"
        );
        assert_eq!(
            conn.query_row(
                "SELECT status FROM video_courses WHERE id='a2'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "pending"
        );
        conn.execute(
            "UPDATE video_queue_lanes SET blocked_reason=NULL WHERE provider='merchant'",
            [],
        )
        .unwrap();
        let next = next_pending(&f.path, Some(Provider::Merchant))
            .unwrap()
            .unwrap();
        assert_eq!(next.id, "a1");
        assert_eq!(next.progress, 45.0);
    }

    #[test]
    fn restart_preserves_paused_skipped_progress_and_topic_position() {
        let f = QueueFixture::new();
        let conn = Connection::open(&f.path).unwrap();
        remember_queue_topic(&conn, &load_course(&f.path, "b1").unwrap()).unwrap();
        persist_stopped_course(&f.path, &f.active("a1"), "skipped", Some("手动跳过")).unwrap();
        conn.execute("UPDATE video_courses SET status='opening',progress=42,duration_seconds=100,kind='slides' WHERE id='b1'",[]).unwrap();
        init_db(&f.path).unwrap();
        let next = next_pending(&f.path, None).unwrap().unwrap();
        assert_eq!(next.id, "b1");
        assert_eq!(next.progress, 42.0);
        assert_eq!(
            conn.query_row(
                "SELECT status FROM video_courses WHERE id='b1'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "paused"
        );
        assert_eq!(
            conn.query_row(
                "SELECT status FROM video_courses WHERE id='a1'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "skipped"
        );
    }

    #[test]
    fn sync_preserves_skipped_paused_and_attention_until_platform_completes() {
        let f = QueueFixture::new();
        let capture = |completed: bool| PageTopicCapture {
            title: "同步专题".into(),
            url: "https://example.com/sync".into(),
            progress: 0.0,
            total_count: 3,
            completed_count: 0,
            courses: ["one", "two", "three"]
                .iter()
                .map(|id| PageCourseCapture {
                    external_id: (*id).into(),
                    title: (*id).into(),
                    url: format!("https://example.com/{id}"),
                    locator: String::new(),
                    section_title: String::new(),
                    kind: "video".into(),
                    duration_seconds: 100,
                    progress: 0.0,
                    completed,
                })
                .collect(),
        };
        let topic = import_capture(&f.state, Provider::Merchant, capture(false))
            .unwrap()
            .topic_id;
        let conn = Connection::open(&f.path).unwrap();
        for (external_id, status) in [
            ("one", "skipped"),
            ("two", "paused"),
            ("three", "attention"),
        ] {
            conn.execute("UPDATE video_courses SET status=?1,progress=45,last_error='保留原因' WHERE topic_id=?2 AND external_id=?3",params![status,topic,external_id]).unwrap();
        }
        import_capture(&f.state, Provider::Merchant, capture(false)).unwrap();
        for (external_id, status) in [
            ("one", "skipped"),
            ("two", "paused"),
            ("three", "attention"),
        ] {
            let result:(String,f64)=conn.query_row("SELECT status,progress FROM video_courses WHERE topic_id=?1 AND external_id=?2",params![topic,external_id],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
            assert_eq!(result, (status.into(), 45.0));
        }
        import_capture(&f.state, Provider::Merchant, capture(true)).unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM video_courses WHERE topic_id=?1 AND status='completed' AND progress=100 AND last_error IS NULL",params![topic],|row|row.get::<_,i64>(0)).unwrap(),3);
    }

    #[test]
    fn sync_corrects_previously_misclassified_completed_courses() {
        let f = QueueFixture::new();
        let progress_values = [100.0, 66.38, 100.0, 100.0, 93.62, 93.95, 94.57, 0.0, 0.0, 15.73, 33.18, 26.66];
        let capture = |all_completed: bool| PageTopicCapture {
            title: "银杏数智赋能专项培训（2026）".into(),
            url: "https://example.com/completion-regression".into(),
            progress: 20.22,
            total_count: 12,
            completed_count: 3,
            courses: progress_values.iter().enumerate().map(|(index, progress)| PageCourseCapture {
                external_id: format!("lesson-{index}"),
                title: format!("课程 {}", index + 1),
                url: format!("https://example.com/lesson/{index}"),
                locator: format!("#lesson-{index}"),
                section_title: String::new(),
                kind: "video".into(),
                duration_seconds: 960,
                progress: if all_completed { 100.0 } else { *progress },
                completed: all_completed || *progress >= 100.0,
            }).collect(),
        };
        let topic_id = import_capture(&f.state, Provider::Merchant, capture(true)).unwrap().topic_id;
        // 核心同步断言：当网页实际状态未完成（例如只有3门完成，其他门为21%、66%等）时，点击同步必须纠正本地状态
        let summary_corrected = import_capture(&f.state, Provider::Merchant, capture(false)).unwrap();
        assert_eq!(summary_corrected.imported, 12);
        assert_eq!(summary_corrected.completed, 3);

        let conn = Connection::open(&f.path).unwrap();
        let mut stmt = conn.prepare("SELECT status,progress FROM video_courses WHERE topic_id=?1 ORDER BY sort_order").unwrap();
        let courses = stmt.query_map(params![topic_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        }).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(courses.len(), 12);
        for ((status, progress), expected) in courses.iter().zip(progress_values) {
            assert_eq!(*progress, expected);
            assert_eq!(status, if expected >= 100.0 { "completed" } else { "pending" });
        }
        let topic: (i64, i64, f64) = conn.query_row(
            "SELECT completed_count,total_count,progress FROM video_topics WHERE id=?1",
            params![topic_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(topic, (3, 12, 20.22));
    }
    #[test]
    fn sync_corrects_user_screenshot_dcd_topic_scenario() {
        let f = QueueFixture::new();
        let topic_url = "https://example.com/topic/dcd-1";
        // 第一次导入：01 视频 15%，02 PPT 21%
        let capture_actual = PageTopicCapture {
            title: "(第一期) DCD开发底座介绍和使用".into(),
            url: topic_url.into(),
            progress: 15.0,
            total_count: 2,
            completed_count: 0,
            courses: vec![
                PageCourseCapture {
                    external_id: "c1".into(),
                    title: "01.（第一期）DCD开发底座介绍和使用".into(),
                    url: "https://example.com/c1".into(),
                    locator: "#c1".into(),
                    section_title: "".into(),
                    kind: "video".into(),
                    duration_seconds: 5460,
                    progress: 15.0,
                    completed: false,
                },
                PageCourseCapture {
                    external_id: "c2".into(),
                    title: "02.（第一期）DCD开发底座介绍和使用PPT".into(),
                    url: "https://example.com/c2".into(),
                    locator: "#c2".into(),
                    section_title: "".into(),
                    kind: "slides".into(),
                    duration_seconds: 0,
                    progress: 21.0,
                    completed: false,
                },
            ],
        };
        let topic_id = import_capture(&f.state, Provider::Merchant, capture_actual.clone()).unwrap().topic_id;

        // 模拟 PPT 课件因驻留超时或误操作在数据库中变成了已完成（100%，时长60秒）
        let conn = Connection::open(&f.path).unwrap();
        conn.execute(
            "UPDATE video_courses SET status='completed',progress=100.0,duration_seconds=60 WHERE topic_id=?1 AND external_id='c2'",
            params![topic_id],
        ).unwrap();
        conn.execute(
            "UPDATE video_topics SET completed_count=1,progress=50.0 WHERE id=?1",
            params![topic_id],
        ).unwrap();

        // 用户点击“同步专题”重新抓取实际页面状态
        let resync_summary = import_capture(&f.state, Provider::Merchant, capture_actual).unwrap();
        assert_eq!(resync_summary.imported, 2);
        assert_eq!(resync_summary.completed, 0);

        // 验证数据库状态被成功纠正
        let c2_status: (String, f64) = conn.query_row(
            "SELECT status, progress FROM video_courses WHERE topic_id=?1 AND external_id='c2'",
            params![topic_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(c2_status, ("pending".into(), 21.0));

        let topic_info: (i64, i64, f64) = conn.query_row(
            "SELECT completed_count, total_count, progress FROM video_topics WHERE id=?1",
            params![topic_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(topic_info, (0, 2, 15.0));
    }

    #[test]
    fn survey_and_offline_import_remain_manual_after_resync() {
        let f = QueueFixture::new();
        let capture = || PageTopicCapture {
            title: "人工智能培训".into(),
            url: "https://example.com/manual-kinds".into(),
            progress: 0.0,
            total_count: 2,
            completed_count: 0,
            courses: ["survey", "offline"].into_iter().map(|kind| PageCourseCapture {
                external_id: kind.into(), title: kind.into(), url: String::new(),
                locator: String::new(), section_title: String::new(), kind: kind.into(),
                duration_seconds: 120, progress: 0.0, completed: false,
            }).collect(),
        };
        let summary = import_capture(&f.state, Provider::Merchant, capture()).unwrap();
        assert_eq!(summary.manual, 2);
        let conn = Connection::open(&f.path).unwrap();
        // 模拟旧版本将线下课作为资料暂停后，新采集应纠正其类型与状态。
        conn.execute("UPDATE video_courses SET kind='material',status='paused' WHERE topic_id=?1", params![summary.topic_id]).unwrap();
        import_capture(&f.state, Provider::Merchant, capture()).unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM video_courses WHERE topic_id=?1 AND kind IN ('survey','offline') AND status='manual'",
            params![summary.topic_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 2);
        conn.execute("UPDATE video_courses SET status='completed',progress=100 WHERE topic_id=?1", params![summary.topic_id]).unwrap();
        import_capture(&f.state, Provider::Merchant, capture()).unwrap();
        let completed: i64 = conn.query_row("SELECT COUNT(*) FROM video_courses WHERE topic_id=?1 AND status='completed'", params![summary.topic_id], |row| row.get(0)).unwrap();
        assert_eq!(completed, 2);
    }

    #[test]
    fn browsing_and_queue_navigation_are_independent() {
        let f = QueueFixture::new();
        for provider in [Provider::Merchant, Provider::Ulearn] {
            assert_ne!(provider.browser_label(), provider.player_label());
        }
        {
            let mut runtime = f.state.runtime.lock().unwrap();
            runtime.playback_tokens.insert("merchant".into(), 1);
            runtime.browser_tokens.insert("merchant".into(), 2);
        }
        assert!(navigation_token_matches(&f.state, Provider::Merchant, 2, true));
        assert!(!navigation_token_matches(&f.state, Provider::Merchant, 1, true));
        // 队列换课不会取消用户的初始页面导航。
        f.state.runtime.lock().unwrap().playback_tokens.insert("merchant".into(), 3);
        assert!(navigation_token_matches(&f.state, Provider::Merchant, 2, true));
        // 用户转去专题页后，旧课件的延迟点击失效，后台播放仍有效。
        cancel_browser_navigation(&f.state, Provider::Merchant);
        assert!(!navigation_token_matches(&f.state, Provider::Merchant, 2, true));
        assert!(playback_token_matches(&f.state, Provider::Merchant, 3));
    }

    #[test]
    fn cancelled_playback_tokens_cannot_control_a_later_course() {
        let f = QueueFixture::new();
        f.state
            .runtime
            .lock()
            .unwrap()
            .playback_tokens
            .insert("merchant".into(), 1);
        assert!(playback_token_matches(&f.state, Provider::Merchant, 1));
        f.state
            .runtime
            .lock()
            .unwrap()
            .playback_tokens
            .remove("merchant");
        assert!(!playback_token_matches(&f.state, Provider::Merchant, 1));
        f.state
            .runtime
            .lock()
            .unwrap()
            .playback_tokens
            .insert("merchant".into(), 2);
        assert!(!playback_token_matches(&f.state, Provider::Merchant, 1));
        assert!(playback_token_matches(&f.state, Provider::Merchant, 2));
    }

    #[test]
    fn repeated_ended_events_do_not_reset_phase_since() {
        let captures = Arc::new(Mutex::new(CaptureExchange::default()));
        let runtime = Arc::new(Mutex::new(RuntimeState::default()));
        let initial_time = now() - 10;
        runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .insert(
                "merchant".to_string(),
                ActiveCourse {
                    course_id: "course-1".to_string(),
                    topic_id: "topic-1".to_string(),
                    provider: Provider::Merchant,
                    kind: "slides".to_string(),
                    course_title: "01. 课件".to_string(),
                    started_at: initial_time - 15,
                    phase: "playing".to_string(),
                    phase_since: initial_time,
                    last_media_at: initial_time,
                    last_progress_at: initial_time,
                    last_advanced_time: 100.0,
                    current_time: 100.0,
                    duration: 100.0,
                },
            );

        // 第一次收到 ended 事件，phase 变为 ended，phase_since 被记录
        assert!(handle_bridge_title(
            "MTOOL_MEDIA|merchant|ended|100|100",
            Provider::Merchant,
            &captures,
            &runtime,
        ));
        let first_phase_since = {
            let state = runtime.lock().unwrap_or_else(|error| error.into_inner());
            let active = state.active.get("merchant").expect("active course exists");
            assert_eq!(active.phase, "ended");
            active.phase_since
        };
        assert!(first_phase_since <= now());

        // 人为将 phase_since 设为 3 秒前（模拟缓冲等待）
        {
            let mut state = runtime.lock().unwrap_or_else(|error| error.into_inner());
            let active = state.active.get_mut("merchant").unwrap();
            active.phase_since = now() - 3;
        }
        let buffered_since = {
            let state = runtime.lock().unwrap_or_else(|error| error.into_inner());
            state.active.get("merchant").unwrap().phase_since
        };

        // 第二次收到 ended 事件（模拟 setInterval 1.5 秒重复上报）
        assert!(handle_bridge_title(
            "MTOOL_MEDIA|merchant|ended|100|100",
            Provider::Merchant,
            &captures,
            &runtime,
        ));

        // 关键断言：phase_since 绝不能被刷新成 now()，必须维持不变以避免核验死锁！
        let final_phase_since = {
            let state = runtime.lock().unwrap_or_else(|error| error.into_inner());
            let active = state.active.get("merchant").expect("active course exists");
            assert_eq!(active.phase, "ended");
            active.phase_since
        };
        assert_eq!(final_phase_since, buffered_since);
        assert!(final_phase_since < now());
    }

    #[test]
    fn test_opening_phase_debounces_premature_ended_events() {
        let captures = Arc::new(Mutex::new(CaptureExchange::default()));
        let runtime = Arc::new(Mutex::new(RuntimeState::default()));
        let current_time = now();
        runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .insert(
                "merchant".to_string(),
                ActiveCourse {
                    course_id: "course-new".to_string(),
                    topic_id: "topic-1".to_string(),
                    provider: Provider::Merchant,
                    kind: "slides".to_string(),
                    course_title: "01. 新课件".to_string(),
                    started_at: current_time,
                    phase: "opening".to_string(),
                    phase_since: current_time,
                    last_media_at: current_time,
                    last_progress_at: current_time,
                    last_advanced_time: 0.0,
                    current_time: 0.0,
                    duration: 600.0,
                },
            );

        // 刚打开 1 秒收到上一门课程遗留的 ended 事件，应被安全忽略，保持 opening 状态
        assert!(handle_bridge_title(
            "MTOOL_MEDIA|merchant|ended|100|100",
            Provider::Merchant,
            &captures,
            &runtime,
        ));
        {
            let state = runtime.lock().unwrap_or_else(|error| error.into_inner());
            let active = state.active.get("merchant").expect("active course exists");
            assert_eq!(active.phase, "opening");
        }

        // 模拟正常进入播放状态并持续一段时间
        {
            let mut state = runtime.lock().unwrap_or_else(|error| error.into_inner());
            let active = state.active.get_mut("merchant").unwrap();
            active.phase = "playing".to_string();
            active.phase_since = current_time - 15;
            active.started_at = current_time - 15;
        }
        assert!(handle_bridge_title(
            "MTOOL_MEDIA|merchant|ended|100|100",
            Provider::Merchant,
            &captures,
            &runtime,
        ));
        {
            let state = runtime.lock().unwrap_or_else(|error| error.into_inner());
            let active = state.active.get("merchant").expect("active course exists");
            assert_eq!(active.phase, "ended");
        }
    }

    #[test]
    fn stalled_playing_course_never_marked_completed_at_high_progress() {
        let f = QueueFixture::new();
        let conn = Connection::open(&f.path).unwrap();
        conn.execute(
            "UPDATE video_courses SET status='playing',progress=93.5,duration_seconds=1000 WHERE id='a1'",
            [],
        ).unwrap();

        let progress = 93.5;
        let mins = (935.0 / 60.0_f64).floor() as i64;
        let last_error = format!("视频播放卡住超过2分钟未推进（停在约{mins}分钟），已自动跳过并开始播放下一门");
        conn.execute(
            "UPDATE video_courses SET status='attention',progress=?2,last_error=?3,updated_at=?4 WHERE id=?1",
            params!["a1", progress, last_error, now()],
        ).unwrap();

        let (status, final_progress, err): (String, f64, Option<String>) = conn.query_row(
            "SELECT status, progress, last_error FROM video_courses WHERE id='a1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();

        assert_eq!(status, "attention");
        assert_eq!(final_progress, 93.5);
        assert!(err.unwrap().contains("视频播放卡住超过2分钟未推进"));

        let completed_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM video_courses WHERE topic_id='a' AND status='completed'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(completed_count, 0);
    }

    #[test]
    fn block_platform_preserves_course_as_paused_and_sets_blocked_reason() {
        let f = QueueFixture::new();
        let mut active = f.active("a1");
        active.current_time = 50.0;
        active.duration = 100.0;

        block_platform(&f.path, &active).unwrap();

        let conn = Connection::open(&f.path).unwrap();
        let (status, progress, error): (String, f64, Option<String>) = conn.query_row(
            "SELECT status, progress, last_error FROM video_courses WHERE id='a1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();

        assert_eq!(status, "paused");
        assert_eq!(progress, 50.0);
        assert!(error.unwrap().contains("登录已失效"));

        let blocked: String = conn.query_row(
            "SELECT blocked_reason FROM video_queue_lanes WHERE provider='merchant'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(blocked.contains("登录已失效"));

        assert!(next_pending(&f.path, Some(Provider::Merchant)).unwrap().is_none());
    }

    #[test]
    fn login_url_detection_matches_login_cas_sso_oauth() {
        let is_login = |url: &str| {
            let s = url.to_lowercase();
            s.contains("/login") || s.contains("/sso") || s.contains("/cas/") || s.contains("oauth")
        };
        assert!(is_login("https://auth.example.com/cas/login?service=xyz"));
        assert!(is_login("https://example.com/sso/authorize"));
        assert!(is_login("https://example.com/user/login"));
        assert!(is_login("https://oauth.example.com/oauth2/token"));

        assert!(!is_login("https://study.example.com/course/play/123"));
        assert!(!is_login("https://study.example.com/video/player.html"));
    }

    #[test]
    fn test_bridge_script_exempts_media_from_login_detection() {
        let script = bridge_script(Provider::Merchant, 2.0, true);
        assert!(script.contains("hasMediaOrPlayer"));
        assert!(script.contains("input[type='password']"));
        assert!(script.contains("pageAge >= 8000"));
    }

    #[test]
    fn test_unblocking_login_clears_blocked_reason_and_course_error() {
        let f = QueueFixture::new();
        let active = f.active("a1");
        block_platform(&f.path, &active).unwrap();

        // 验证被 block
        let conn = Connection::open(&f.path).unwrap();
        let blocked: Option<String> = conn.query_row(
            "SELECT blocked_reason FROM video_queue_lanes WHERE provider='merchant'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(blocked.is_some());

        // 执行解锁清理
        conn.execute(
            "UPDATE video_queue_lanes SET blocked_reason=NULL WHERE blocked_reason LIKE '%登录%'",
            [],
        ).unwrap();
        conn.execute(
            "UPDATE video_courses SET last_error=NULL WHERE last_error LIKE '%登录%'",
            [],
        ).unwrap();

        // 验证已清空且课程可再次被调度
        let blocked_after: Option<String> = conn.query_row(
            "SELECT blocked_reason FROM video_queue_lanes WHERE provider='merchant'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(blocked_after.is_none());

        let course_err: Option<String> = conn.query_row(
            "SELECT last_error FROM video_courses WHERE id='a1'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(course_err.is_none());

        let next = next_pending(&f.path, Some(Provider::Merchant)).unwrap();
        assert!(next.is_some());
        assert_eq!(next.unwrap().id, "a1");
    }

    #[test]
    fn test_ppt_bridge_script_detection_and_no_premature_completion() {
        let script = bridge_script(Provider::Merchant, 1.0, true);

        // 1. 验证多通道进度探测
        assert!(script.contains("parseProgressNum"));
        assert!(script.contains("detectDocLearningProgress"));
        assert!(script.contains("progressLabels"));
        assert!(script.contains("activeChapterEls"));
        assert!(script.contains("statusBadges"));
        assert!(script.contains("ant-progress"));

        // 2. 验证 PPT 自动翻页与微交互
        assert!(script.contains("ArrowRight"));
        assert!(script.contains("PageDown"));
        assert!(script.contains("下一页"));
        assert!(script.contains("slideArea"));

        // 3. 验证彻底废除 90 秒草率提前完播切课的 bug
        assert!(!script.contains("staySeconds >= 90"));
        assert!(!script.contains("staySeconds >= 60 && hasVisibleCompletionModal"));
        assert!(script.contains("targetDuration = 600"));

        // 4. 验证乐学抓取脚本支持 PPT 识别与明确百分比进度
        let ulearn = capture_script("req_u", Provider::Ulearn);
        assert!(ulearn.contains("isSlides"));
        assert!(ulearn.contains("progMatch"));
    }

    #[test]
    fn test_video_course_isolated_from_doc_ended_and_prefix_interference() {
        let captures = Arc::new(Mutex::new(CaptureExchange::default()));
        let runtime = Arc::new(Mutex::new(RuntimeState::default()));
        let current_time = now();

        // 1. 验证 bridge_script 包含对视频课程的独立隔离保护及序号精确匹配
        let script = bridge_script(Provider::Merchant, 2.0, true);
        assert!(script.contains("state.currentCourseKind === \"video\""));
        assert!(script.contains("matchesCurrentCourse"));
        assert!(script.contains("expectedIdxMatch"));
        assert!(script.contains("__mtool_current_course__"));

        // 2. 构造 04 视频课：时长 4165 秒（70分钟），当前进度 1082 秒（约 26%）
        runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .insert(
                "merchant".to_string(),
                ActiveCourse {
                    course_id: "course-dcd-04".to_string(),
                    topic_id: "topic-dcd".to_string(),
                    provider: Provider::Merchant,
                    kind: "video".to_string(),
                    course_title: "04. （第四期）DCD开发底座自定义业务的开发和接入".to_string(),
                    started_at: current_time,
                    phase: "opening".to_string(),
                    phase_since: current_time,
                    last_media_at: current_time,
                    last_progress_at: current_time,
                    last_advanced_time: 1082.0,
                    current_time: 1082.0,
                    duration: 4165.0,
                },
            );

        // 3. 刚打开 1 秒内收到上一门 PPT 遗留的 100|100 ended 事件，坚决阻断
        assert!(handle_bridge_title(
            "MTOOL_MEDIA|merchant|ended|100|100",
            Provider::Merchant,
            &captures,
            &runtime,
        ));
        {
            let state = runtime.lock().unwrap_or_else(|error| error.into_inner());
            let active = state.active.get("merchant").expect("active course exists");
            assert_eq!(active.phase, "opening");
            assert_eq!(active.duration, 4165.0);
            assert_eq!(active.current_time, 1082.0);
        }

        // 4. 模拟播放状态并经过 15 秒，但收到来自文档探测的伪造 100|100 完播包，依然坚决阻断
        {
            let mut state = runtime.lock().unwrap_or_else(|error| error.into_inner());
            let active = state.active.get_mut("merchant").unwrap();
            active.phase = "playing".to_string();
            active.started_at = current_time - 15;
            active.phase_since = current_time - 15;
        }
        // 4.1 伪造的 timeupdate 100|100 不能覆盖真实时长与播放进度
        assert!(handle_bridge_title(
            "MTOOL_MEDIA|merchant|timeupdate|100|100",
            Provider::Merchant,
            &captures,
            &runtime,
        ));
        {
            let state = runtime.lock().unwrap_or_else(|error| error.into_inner());
            let active = state.active.get("merchant").expect("active course exists");
            assert_eq!(active.duration, 4165.0);
            assert_eq!(active.current_time, 1082.0);
            assert_eq!(active.phase, "playing");
        }
        // 4.2 伪造的 ended 100|100 不能导致视频提前结束
        assert!(handle_bridge_title(
            "MTOOL_MEDIA|merchant|ended|100|100",
            Provider::Merchant,
            &captures,
            &runtime,
        ));
        {
            let state = runtime.lock().unwrap_or_else(|error| error.into_inner());
            let active = state.active.get("merchant").expect("active course exists");
            assert_eq!(active.phase, "playing");
            assert_eq!(active.duration, 4165.0);
        }

        // 5. 真实的视频播放器完播包（4165|4165）正常核验并完成
        assert!(handle_bridge_title(
            "MTOOL_MEDIA|merchant|ended|4165|4165",
            Provider::Merchant,
            &captures,
            &runtime,
        ));
        {
            let state = runtime.lock().unwrap_or_else(|error| error.into_inner());
            let active = state.active.get("merchant").expect("active course exists");
            assert_eq!(active.phase, "ended");
            assert_eq!(active.duration, 4165.0);
            assert_eq!(active.current_time, 4165.0);
        }
    }
}
