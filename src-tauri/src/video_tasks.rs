use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};

const PLAYER_LABEL: &str = "video-task-player";
const POPUP_LABEL_PREFIX: &str = "video-task-popup-";
static POPUP_COUNTER: AtomicU64 = AtomicU64::new(1);

pub struct VideoTaskState {
    speed: Arc<Mutex<f64>>,
    muted: Arc<Mutex<bool>>,
}

impl Default for VideoTaskState {
    fn default() -> Self {
        Self {
            speed: Arc::new(Mutex::new(1.0)),
            muted: Arc::new(Mutex::new(false)),
        }
    }
}

#[derive(serde::Serialize)]
pub struct VideoTaskWindowStatus {
    open: bool,
    url: Option<String>,
}

fn parse_http_url(url: &str) -> Result<tauri::Url, String> {
    let parsed = url
        .trim()
        .parse::<tauri::Url>()
        .map_err(|_| "请输入有效的网址".to_string())?;

    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("仅支持 http 或 https 教学网址".to_string());
    }

    Ok(parsed)
}

fn clamp_speed(speed: f64) -> f64 {
    if speed.is_finite() {
        speed.clamp(0.25, 10.0)
    } else {
        1.0
    }
}

fn initialization_script(speed: f64, muted: bool) -> String {
    let speed = clamp_speed(speed);
    let muted = if muted { "true" } else { "false" };
    format!(
        r#"
(() => {{
  if (window.__MTOOL_VIDEO_HELPER__) {{
    window.__MTOOL_VIDEO_HELPER__.setSpeed({speed});
    window.__MTOOL_VIDEO_HELPER__.setMuted({muted});
    return;
  }}

  const originalAddEventListener = EventTarget.prototype.addEventListener;
  const originalPause = HTMLMediaElement.prototype.pause;
  const originalPlay = HTMLMediaElement.prototype.play;
  const state = {{
    speed: {speed},
    muted: {muted},
    blurred: false,
    keepPlaying: false,
    tracked: new WeakSet(),
    setSpeed(value) {{
      const numeric = Number(value);
      this.speed = Number.isFinite(numeric) ? Math.min(10, Math.max(0.25, numeric)) : 1;
      applyToMedia();
    }},
    setMuted(value) {{
      this.muted = Boolean(value);
      applyToMedia();
    }}
  }};

  Object.defineProperty(window, '__MTOOL_VIDEO_HELPER__', {{ value: state }});

  // Record the real focus state before filtering the listeners installed by the page.
  originalAddEventListener.call(window, 'blur', () => {{ state.blurred = true; }}, true);
  originalAddEventListener.call(window, 'focus', () => {{ state.blurred = false; }}, true);

  const blockedEvents = new Set(['visibilitychange', 'webkitvisibilitychange', 'blur', 'focusout', 'pagehide', 'freeze']);
  EventTarget.prototype.addEventListener = function(type, listener, options) {{
    if ((this === window || this === document) && blockedEvents.has(String(type))) return;
    return originalAddEventListener.call(this, type, listener, options);
  }};

  const forceVisible = (target, key, value) => {{
    try {{ Object.defineProperty(target, key, {{ configurable: true, get: () => value }}); }} catch (_) {{}}
  }};
  forceVisible(document, 'hidden', false);
  forceVisible(document, 'webkitHidden', false);
  forceVisible(document, 'visibilityState', 'visible');
  forceVisible(document, 'webkitVisibilityState', 'visible');
  try {{ document.hasFocus = () => true; }} catch (_) {{}}
  try {{ Object.defineProperty(window, 'onblur', {{ configurable: true, get: () => null, set: () => {{}} }}); }} catch (_) {{}}
  try {{ Object.defineProperty(document, 'onvisibilitychange', {{ configurable: true, get: () => null, set: () => {{}} }}); }} catch (_) {{}}

  HTMLMediaElement.prototype.pause = function() {{
    if (state.blurred && state.keepPlaying && !this.ended) return;
    return originalPause.call(this);
  }};

  function trackMedia(media) {{
    if (state.tracked.has(media)) return;
    state.tracked.add(media);
    originalAddEventListener.call(media, 'play', () => {{ state.keepPlaying = true; }}, true);
    originalAddEventListener.call(media, 'pause', () => {{
      if (!state.blurred) state.keepPlaying = false;
    }}, true);
    originalAddEventListener.call(media, 'ended', () => {{ state.keepPlaying = false; }}, true);
  }}

  function applyToMedia() {{
    document.querySelectorAll('video, audio').forEach((media) => {{
      trackMedia(media);
      try {{ media.defaultPlaybackRate = state.speed; }} catch (_) {{}}
      try {{ media.playbackRate = state.speed; }} catch (_) {{}}
      try {{ media.muted = state.muted; }} catch (_) {{}}
      if (state.blurred && state.keepPlaying && media.paused && !media.ended && media.readyState >= 2) {{
        try {{ const result = originalPlay.call(media); if (result && result.catch) result.catch(() => {{}}); }} catch (_) {{}}
      }}
    }});
  }}

  originalAddEventListener.call(window, 'message', (event) => {{
    if (!event.data) return;
    if (event.data.type === 'mtool-video-speed') state.setSpeed(event.data.speed);
    else if (event.data.type === 'mtool-video-muted') state.setMuted(event.data.muted);
    else return;
    document.querySelectorAll('iframe').forEach((frame) => {{
      try {{ frame.contentWindow.postMessage(event.data, '*'); }} catch (_) {{}}
    }});
  }}, true);

  const start = () => {{
    applyToMedia();
    try {{
      new MutationObserver(applyToMedia).observe(document.documentElement || document, {{ childList: true, subtree: true }});
    }} catch (_) {{}}
    setInterval(applyToMedia, 500);
  }};

  if (document.readyState === 'loading') originalAddEventListener.call(document, 'DOMContentLoaded', start, {{ once: true }});
  else start();
}})();
"#
    )
}

fn speed_update_script(speed: f64) -> String {
    let speed = clamp_speed(speed);
    format!(
        r#"
(() => {{
  if (window.__MTOOL_VIDEO_HELPER__) window.__MTOOL_VIDEO_HELPER__.setSpeed({speed});
  const message = {{ type: 'mtool-video-speed', speed: {speed} }};
  document.querySelectorAll('iframe').forEach((frame) => {{
    try {{ frame.contentWindow.postMessage(message, '*'); }} catch (_) {{}}
  }});
}})();
"#
    )
}

fn mute_update_script(muted: bool) -> String {
    let muted = if muted { "true" } else { "false" };
    format!(
        r#"
(() => {{
  if (window.__MTOOL_VIDEO_HELPER__) window.__MTOOL_VIDEO_HELPER__.setMuted({muted});
  const message = {{ type: 'mtool-video-muted', muted: {muted} }};
  document.querySelectorAll('iframe').forEach((frame) => {{
    try {{ frame.contentWindow.postMessage(message, '*'); }} catch (_) {{}}
  }});
}})();
"#
    )
}

fn is_video_task_window(label: &str) -> bool {
    label == PLAYER_LABEL || label.starts_with(POPUP_LABEL_PREFIX)
}

#[cfg(desktop)]
fn build_video_task_popup(
    app: &AppHandle,
    requested_url: tauri::Url,
    features: tauri::webview::NewWindowFeatures,
    speed_state: Arc<Mutex<f64>>,
    muted_state: Arc<Mutex<bool>>,
) -> Result<tauri::WebviewWindow, String> {
    let label = format!(
        "{POPUP_LABEL_PREFIX}{}",
        POPUP_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let speed = *speed_state.lock().unwrap_or_else(|e| e.into_inner());
    let muted = *muted_state.lock().unwrap_or_else(|e| e.into_inner());
    let nested_app = app.clone();
    let nested_speed = speed_state.clone();
    let nested_muted = muted_state.clone();
    let load_speed = speed_state.clone();
    let load_muted = muted_state.clone();

    WebviewWindowBuilder::new(
        app,
        label,
        WebviewUrl::External("about:blank".parse().expect("about:blank is a valid URL")),
    )
    .title(format!("MTOOL · {}", requested_url.as_str()))
    .inner_size(1280.0, 820.0)
    .min_inner_size(320.0, 180.0)
    .resizable(true)
    .minimizable(true)
    .window_features(features)
    .initialization_script_for_all_frames(initialization_script(speed, muted))
    .on_document_title_changed(|window, title| {
        let _ = window.set_title(&format!("MTOOL · {title}"));
    })
    .on_page_load(move |window, _payload| {
        let current_speed = *load_speed.lock().unwrap_or_else(|e| e.into_inner());
        let current_muted = *load_muted.lock().unwrap_or_else(|e| e.into_inner());
        let _ = window.eval(speed_update_script(current_speed));
        let _ = window.eval(mute_update_script(current_muted));
    })
    .on_new_window(move |url, popup_features| {
        match build_video_task_popup(
            &nested_app,
            url,
            popup_features,
            nested_speed.clone(),
            nested_muted.clone(),
        ) {
            Ok(window) => tauri::webview::NewWindowResponse::Create { window },
            Err(error) => {
                eprintln!("[mtool video task] failed to create popup: {error}");
                tauri::webview::NewWindowResponse::Deny
            }
        }
    })
    .build()
    .map_err(|e| format!("创建课程新窗口失败: {e}"))
}

fn apply_compact_mode(window: &tauri::WebviewWindow, compact: bool) -> Result<(), String> {
    window
        .unminimize()
        .map_err(|e| format!("恢复播放窗口失败: {e}"))?;
    window
        .set_always_on_top(compact)
        .map_err(|e| format!("设置置顶状态失败: {e}"))?;

    if compact {
        window
            .set_size(LogicalSize::new(360.0, 220.0))
            .map_err(|e| format!("调整小窗尺寸失败: {e}"))?;
        window
            .set_position(LogicalPosition::new(20.0, 20.0))
            .map_err(|e| format!("移动小窗失败: {e}"))?;
    } else {
        window
            .set_size(LogicalSize::new(1280.0, 820.0))
            .map_err(|e| format!("恢复窗口尺寸失败: {e}"))?;
        window
            .center()
            .map_err(|e| format!("居中播放窗口失败: {e}"))?;
    }

    window
        .show()
        .map_err(|e| format!("显示播放窗口失败: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn open_video_task_window(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    url: String,
    speed: f64,
    muted: bool,
    compact: bool,
) -> Result<VideoTaskWindowStatus, String> {
    let parsed = parse_http_url(&url)?;
    let speed = clamp_speed(speed);
    *state.speed.lock().unwrap_or_else(|e| e.into_inner()) = speed;
    *state.muted.lock().unwrap_or_else(|e| e.into_inner()) = muted;

    if let Some(window) = app.get_webview_window(PLAYER_LABEL) {
        let current = window.url().ok();
        if current.as_ref() != Some(&parsed) {
            window
                .navigate(parsed.clone())
                .map_err(|e| format!("打开教学网址失败: {e}"))?;
        }
        window
            .eval(speed_update_script(speed))
            .map_err(|e| format!("更新播放倍速失败: {e}"))?;
        window
            .eval(mute_update_script(muted))
            .map_err(|e| format!("更新静音状态失败: {e}"))?;
        apply_compact_mode(&window, compact)?;
        window
            .set_focus()
            .map_err(|e| format!("聚焦播放窗口失败: {e}"))?;
    } else {
        let speed_state = state.speed.clone();
        let muted_state = state.muted.clone();
        let load_speed = speed_state.clone();
        let load_muted = muted_state.clone();
        let mut builder =
            WebviewWindowBuilder::new(&app, PLAYER_LABEL, WebviewUrl::External(parsed.clone()))
                .title("MTOOL · 视频任务")
                .inner_size(
                    if compact { 360.0 } else { 1280.0 },
                    if compact { 220.0 } else { 820.0 },
                )
                .min_inner_size(320.0, 180.0)
                .resizable(true)
                .minimizable(true)
                .always_on_top(compact)
                .initialization_script_for_all_frames(initialization_script(speed, muted))
                .on_page_load(move |window, _payload| {
                    let current_speed = *load_speed.lock().unwrap_or_else(|e| e.into_inner());
                    let current_muted = *load_muted.lock().unwrap_or_else(|e| e.into_inner());
                    let _ = window.eval(speed_update_script(current_speed));
                    let _ = window.eval(mute_update_script(current_muted));
                });

        #[cfg(desktop)]
        {
            let popup_app = app.clone();
            let popup_speed = speed_state.clone();
            let popup_muted = muted_state.clone();
            builder = builder.on_new_window(move |url, features| {
                match build_video_task_popup(
                    &popup_app,
                    url,
                    features,
                    popup_speed.clone(),
                    popup_muted.clone(),
                ) {
                    Ok(window) => tauri::webview::NewWindowResponse::Create { window },
                    Err(error) => {
                        eprintln!("[mtool video task] failed to create popup: {error}");
                        tauri::webview::NewWindowResponse::Deny
                    }
                }
            });
        }

        let window = builder
            .build()
            .map_err(|e| format!("创建内置浏览器失败: {e}"))?;

        if compact {
            window
                .set_position(LogicalPosition::new(20.0, 20.0))
                .map_err(|e| format!("移动小窗失败: {e}"))?;
        } else {
            let _ = window.center();
        }
    }

    Ok(VideoTaskWindowStatus {
        open: true,
        url: Some(parsed.to_string()),
    })
}

#[tauri::command]
pub fn set_video_task_speed(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    speed: f64,
) -> Result<(), String> {
    let speed = clamp_speed(speed);
    *state.speed.lock().unwrap_or_else(|e| e.into_inner()) = speed;
    for (label, window) in app.webview_windows() {
        if !is_video_task_window(&label) {
            continue;
        }
        window
            .eval(speed_update_script(speed))
            .map_err(|e| format!("更新播放倍速失败: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub fn set_video_task_muted(
    app: AppHandle,
    state: tauri::State<'_, VideoTaskState>,
    muted: bool,
) -> Result<(), String> {
    *state.muted.lock().unwrap_or_else(|e| e.into_inner()) = muted;
    for (label, window) in app.webview_windows() {
        if !is_video_task_window(&label) {
            continue;
        }
        window
            .eval(mute_update_script(muted))
            .map_err(|e| format!("更新静音状态失败: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub fn set_video_task_compact(app: AppHandle, compact: bool) -> Result<(), String> {
    let window = app
        .get_webview_window(PLAYER_LABEL)
        .ok_or_else(|| "请先打开一个教学网址".to_string())?;
    apply_compact_mode(&window, compact)
}

#[tauri::command]
pub fn get_video_task_window_status(app: AppHandle) -> VideoTaskWindowStatus {
    if let Some(window) = app.get_webview_window(PLAYER_LABEL) {
        return VideoTaskWindowStatus {
            open: true,
            url: window.url().ok().map(|url| url.to_string()),
        };
    }

    if let Some((_label, window)) = app
        .webview_windows()
        .into_iter()
        .find(|(label, _window)| label.starts_with(POPUP_LABEL_PREFIX))
    {
        VideoTaskWindowStatus {
            open: true,
            url: window.url().ok().map(|url| url.to_string()),
        }
    } else {
        VideoTaskWindowStatus {
            open: false,
            url: None,
        }
    }
}

#[tauri::command]
pub fn restore_video_task_window(app: AppHandle) -> Result<(), String> {
    let windows: Vec<_> = app
        .webview_windows()
        .into_iter()
        .filter(|(label, _window)| is_video_task_window(label))
        .map(|(_label, window)| window)
        .collect();
    if windows.is_empty() {
        return Err("请先打开一个教学网址".to_string());
    }

    for window in &windows {
        window
            .unminimize()
            .map_err(|e| format!("恢复播放窗口失败: {e}"))?;
        window
            .show()
            .map_err(|e| format!("显示播放窗口失败: {e}"))?;
    }

    let focus_window = app
        .get_webview_window(PLAYER_LABEL)
        .unwrap_or_else(|| windows[0].clone());
    focus_window
        .set_focus()
        .map_err(|e| format!("聚焦播放窗口失败: {e}"))?;
    Ok(())
}

#[tauri::command]
pub fn close_video_task_window(app: AppHandle) -> Result<(), String> {
    for (label, window) in app.webview_windows() {
        if !is_video_task_window(&label) {
            continue;
        }
        window
            .close()
            .map_err(|e| format!("关闭播放窗口失败: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_speed_is_limited_to_supported_range() {
        assert_eq!(clamp_speed(0.1), 0.25);
        assert_eq!(clamp_speed(3.0), 3.0);
        assert_eq!(clamp_speed(20.0), 10.0);
        assert_eq!(clamp_speed(f64::NAN), 1.0);
    }

    #[test]
    fn only_http_teaching_urls_are_accepted() {
        assert!(parse_http_url("https://example.com/course").is_ok());
        assert!(parse_http_url("http://example.com/course").is_ok());
        assert!(parse_http_url("file:///tmp/course.html").is_err());
        assert!(parse_http_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn initialization_script_contains_requested_speed() {
        let script = initialization_script(7.5, true);
        assert!(script.contains("speed: 7.5"));
        assert!(script.contains("muted: true"));
        assert!(script.contains("visibilitychange"));
        assert!(script.contains("HTMLMediaElement.prototype.pause"));
    }

    #[test]
    fn popup_windows_are_part_of_video_task_controls() {
        assert!(is_video_task_window(PLAYER_LABEL));
        assert!(is_video_task_window("video-task-popup-42"));
        assert!(!is_video_task_window("main"));
    }
}
