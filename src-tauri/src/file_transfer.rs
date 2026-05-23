use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio::sync::oneshot;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub ip: String,
    pub port: u16,
    pub hostname: String,
    pub alias: String,
    pub added_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigData {
    pub save_dir: Option<String>,
    pub trusted_peers: Vec<PeerInfo>,
}

#[derive(Debug, Serialize)]
pub struct LocalTransferInfo {
    pub ips: Vec<String>,
    pub port: u16,
    pub hostname: String,
}

#[derive(Debug, Serialize)]
pub struct SelectedFileInfo {
    pub path: String,
    pub name: String,
    pub size: u64,
}

#[derive(Clone)]
pub struct TransferState {
    pub config: Arc<RwLock<ConfigData>>,
    pub config_path: std::path::PathBuf,
    pub pending_friends: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
    pub active_transfers: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
    pub local_port: Arc<Mutex<u16>>,
}

impl TransferState {
    pub fn new() -> Self {
        let config_path = get_config_path();
        let config = match std::fs::read_to_string(&config_path) {
            Ok(content) => serde_json::from_str::<ConfigData>(&content).unwrap_or_default(),
            Err(_) => ConfigData::default(),
        };

        Self {
            config: Arc::new(RwLock::new(config)),
            config_path,
            pending_friends: Arc::new(Mutex::new(HashMap::new())),
            active_transfers: Arc::new(Mutex::new(HashMap::new())),
            local_port: Arc::new(Mutex::new(0)),
        }
    }

    pub fn save_config(&self, data: &ConfigData) -> Result<(), String> {
        let content = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
        std::fs::write(&self.config_path, content).map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn get_config_path() -> std::path::PathBuf {
    let data_dir = dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let app_dir = data_dir.join("mtool");
    std::fs::create_dir_all(&app_dir).ok();
    app_dir.join("mtool_transfer_config.json")
}

pub fn get_local_ips() -> Vec<String> {
    let mut ips = Vec::new();
    if let Ok(interfaces) = get_if_addrs::get_if_addrs() {
        for interface in interfaces {
            if !interface.is_loopback() {
                if let std::net::IpAddr::V4(ipv4) = interface.ip() {
                    ips.push(ipv4.to_string());
                }
            }
        }
    }
    if ips.is_empty() {
        ips.push("127.0.0.1".to_string());
    }
    ips
}

pub fn get_system_hostname() -> String {
    #[cfg(target_os = "windows")]
    {
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Windows Device".to_string())
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Try getting hostname via libc::gethostname first
        let mut buf = vec![0u8; 256];
        let res = unsafe {
            libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len())
        };
        if res == 0 {
            if let Some(pos) = buf.iter().position(|&b| b == 0) {
                if let Ok(s) = String::from_utf8(buf[..pos].to_vec()) {
                    let s_trimmed = s.trim().to_string();
                    if !s_trimmed.is_empty() {
                        return s_trimmed;
                    }
                }
            }
        }

        // Fallback to env var or file
        std::env::var("HOSTNAME")
            .or_else(|_| std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()))
            .unwrap_or_else(|_| "macOS/Linux Device".to_string())
    }
}

pub fn is_lan_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ipv4) => {
            ipv4.is_private() || ipv4.is_loopback() || ipv4.is_link_local()
        }
        std::net::IpAddr::V6(ipv6) => {
            ipv6.is_loopback() ||
            (ipv6.segments()[0] & 0xfe00 == 0xfc00) || // ULA
            (ipv6.segments()[0] & 0xffc0 == 0xfe80)    // Link-local
        }
    }
}

async fn read_frame(stream: &mut TcpStream) -> Result<String, String> {
    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes).await.map_err(|e| e.to_string())?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    if len > 10 * 1024 * 1024 {
        return Err("Frame too large (>10MB)".to_string());
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
    String::from_utf8(buf).map_err(|e| e.to_string())
}

async fn write_frame(stream: &mut TcpStream, msg: &str) -> Result<(), String> {
    let bytes = msg.as_bytes();
    let len = bytes.len() as u32;
    stream.write_all(&len.to_be_bytes()).await.map_err(|e| e.to_string())?;
    stream.write_all(bytes).await.map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn start_file_transfer_server(app: AppHandle, state: TransferState) {
    let mut port = 52026;
    let listener = loop {
        match TcpListener::bind(format!("0.0.0.0:{}", port)).await {
            Ok(l) => break l,
            Err(e) => {
                port += 1;
                if port > 52035 {
                    eprintln!("[mtool server] Failed to bind to any port: {}", e);
                    return;
                }
            }
        }
    };

    {
        let mut p = state.local_port.lock().await;
        *p = port;
    }

    println!("[mtool server] listening on port {}", port);

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                if !is_lan_ip(addr.ip()) {
                    println!("[mtool server] Rejecting non-LAN IP: {}", addr.ip());
                    continue;
                }

                let app_clone = app.clone();
                let state_clone = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_incoming_connection(app_clone, state_clone, stream, addr.ip().to_string()).await {
                        eprintln!("[mtool server] handle connection err from {}: {}", addr.ip(), e);
                    }
                });
            }
            Err(e) => {
                eprintln!("[mtool server] accept error: {}", e);
            }
        }
    }
}

async fn handle_incoming_connection(
    app: AppHandle,
    state: TransferState,
    mut stream: TcpStream,
    peer_ip: String,
) -> Result<(), String> {
    let frame = read_frame(&mut stream).await?;
    let req: serde_json::Value = serde_json::from_str(&frame).map_err(|e| e.to_string())?;
    let req_type = req["type"].as_str().ok_or("Missing request type")?;

    match req_type {
        "FriendRequest" => {
            let sender_name = req["sender_name"].as_str().unwrap_or("Unknown Device").to_string();
            let sender_port = req["sender_port"].as_u64().unwrap_or(52026) as u16;
            let request_id = format!("req-{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis());
            let (tx, rx) = oneshot::channel();
            
            {
                let mut pending = state.pending_friends.lock().await;
                pending.insert(request_id.clone(), tx);
            }

            app.emit("friend-request", serde_json::json!({
                "request_id": request_id,
                "sender_ip": peer_ip,
                "sender_name": sender_name,
                "sender_port": sender_port,
            })).ok();

            let accepted = match rx.await {
                Ok(ans) => ans,
                Err(_) => false,
            };

            let my_hostname = get_system_hostname();
            let my_local_port = {
                let p = state.local_port.lock().await;
                *p
            };
            let resp = serde_json::json!({
                "type": "FriendResponse",
                "accepted": accepted,
                "sender_name": my_hostname,
                "sender_port": my_local_port,
            });

            write_frame(&mut stream, &resp.to_string()).await?;

            if accepted {
                let mut conf = state.config.write().await;
                conf.trusted_peers.retain(|p| !(p.ip == peer_ip && p.port == sender_port));
                conf.trusted_peers.push(PeerInfo {
                    ip: peer_ip.clone(),
                    port: sender_port,
                    hostname: sender_name,
                    alias: String::new(),
                    added_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                });
                state.save_config(&conf)?;
                app.emit("trusted-peers-updated", ()).ok();
            }
        }
        "TransferRequest" => {
            let sender_name = req["sender_name"].as_str().unwrap_or("Unknown Device").to_string();
            let transfer_id = req["transfer_id"].as_str().ok_or("Missing transfer_id")?.to_string();
            let filename = req["filename"].as_str().ok_or("Missing filename")?.to_string();
            let filesize = req["filesize"].as_u64().ok_or("Missing filesize")?;

            let is_trusted = {
                let conf = state.config.read().await;
                conf.trusted_peers.iter().any(|p| p.ip == peer_ip)
            };

            if !is_trusted {
                let resp = serde_json::json!({
                    "type": "TransferResponse",
                    "accepted": false,
                    "reason": "Not in trusted devices list",
                });
                write_frame(&mut stream, &resp.to_string()).await?;
                return Err("Untrusted peer attempted file transfer".to_string());
            }

            let resp = serde_json::json!({
                "type": "TransferResponse",
                "accepted": true,
            });
            write_frame(&mut stream, &resp.to_string()).await?;

            let app_clone = app.clone();
            let state_clone = state.clone();
            let transfer_id_clone = transfer_id.clone();
            
            let handle = tokio::spawn(async move {
                if let Err(e) = recv_file_task(app_clone, state_clone, transfer_id_clone, stream, filename, filesize, sender_name).await {
                    eprintln!("[mtool server] receive file err: {}", e);
                }
            });

            {
                let mut active = state.active_transfers.lock().await;
                active.insert(transfer_id, handle);
            }
        }
        _ => return Err(format!("Unknown request type: {}", req_type)),
    }
    Ok(())
}

struct FileCleanupGuard {
    path: std::path::PathBuf,
    active: bool,
}

impl Drop for FileCleanupGuard {
    fn drop(&mut self) {
        if self.active {
            let path_clone = self.path.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(100));
                std::fs::remove_file(&path_clone).ok();
            });
        }
    }
}

async fn recv_file_task(
    app: AppHandle,
    state: TransferState,
    transfer_id: String,
    mut stream: TcpStream,
    filename: String,
    filesize: u64,
    sender_name: String,
) -> Result<(), String> {
    let save_dir = {
        let conf = state.config.read().await;
        conf.save_dir.clone()
    };
    
    let save_dir_path = match save_dir {
        Some(path) => std::path::PathBuf::from(path),
        None => dirs::download_dir().ok_or_else(|| "Could not locate Downloads directory".to_string())?,
    };

    std::fs::create_dir_all(&save_dir_path).map_err(|e| format!("Failed to create save directory: {}", e))?;

    let mut target_path = save_dir_path.join(&filename);
    let mut file_idx = 1;
    let path_clone = target_path.clone();
    let stem = path_clone.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let extension = path_clone.extension().unwrap_or_default().to_string_lossy().to_string();
    while target_path.exists() {
        let new_name = if extension.is_empty() {
            format!("{} ({})", stem, file_idx)
        } else {
            format!("{} ({}).{}", stem, file_idx, extension)
        };
        target_path = save_dir_path.join(new_name);
        file_idx += 1;
    }

    let mut file = tokio::fs::File::create(&target_path).await.map_err(|e| format!("Failed to create destination file: {}", e))?;
    let mut cleanup_guard = FileCleanupGuard { path: target_path.clone(), active: true };

    app.emit("recv-started", serde_json::json!({
        "transfer_id": transfer_id,
        "filename": target_path.file_name().unwrap().to_string_lossy().to_string(),
        "filesize": filesize,
        "sender_name": sender_name,
        "save_path": target_path.to_string_lossy().to_string(),
    })).ok();

    let mut buf = vec![0u8; 64 * 1024];
    let mut bytes_received = 0u64;
    let start_time = std::time::Instant::now();
    let mut last_emit = std::time::Instant::now();

    loop {
        let n = stream.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).await.map_err(|e| format!("File write error: {}", e))?;
        bytes_received += n as u64;

        let now = std::time::Instant::now();
        if now.duration_since(last_emit) >= std::time::Duration::from_millis(200) || bytes_received == filesize {
            last_emit = now;
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 { bytes_received as f64 / elapsed } else { 0.0 };
            app.emit("recv-progress", serde_json::json!({
                "transfer_id": transfer_id,
                "bytes_received": bytes_received,
                "total_bytes": filesize,
                "speed": speed,
            })).ok();
        }

        if bytes_received >= filesize {
            break;
        }
    }

    file.flush().await.ok();
    
    if bytes_received < filesize {
        return Err(format!("Transfer interrupted: received {} of {} bytes", bytes_received, filesize));
    }

    cleanup_guard.active = false;

    app.emit("recv-success", serde_json::json!({
        "transfer_id": transfer_id,
        "save_path": target_path.to_string_lossy().to_string(),
    })).ok();

    {
        let mut active = state.active_transfers.lock().await;
        active.remove(&transfer_id);
    }

    Ok(())
}

async fn send_file_task(
    app: AppHandle,
    state: TransferState,
    transfer_id: String,
    receiver_addr: String,
    file_path: String,
    filename: String,
    filesize: u64,
) -> Result<(), String> {
    let mut stream = TcpStream::connect(&receiver_addr).await.map_err(|e| format!("Failed to connect to {}: {}", receiver_addr, e))?;

    let my_hostname = get_system_hostname();
    let req = serde_json::json!({
        "type": "TransferRequest",
        "transfer_id": transfer_id,
        "sender_name": my_hostname,
        "filename": filename,
        "filesize": filesize,
    });
    write_frame(&mut stream, &req.to_string()).await?;

    let frame = read_frame(&mut stream).await?;
    let resp: serde_json::Value = serde_json::from_str(&frame).map_err(|e| e.to_string())?;
    let accepted = resp["accepted"].as_bool().unwrap_or(false);

    if !accepted {
        let reason = resp["reason"].as_str().unwrap_or("Rejected by receiver").to_string();
        app.emit("send-rejected", serde_json::json!({
            "transfer_id": transfer_id,
            "reason": reason,
        })).ok();
        return Err(format!("Rejected: {}", reason));
    }

    app.emit("send-started", serde_json::json!({
        "transfer_id": transfer_id,
        "filename": filename,
        "filesize": filesize,
    })).ok();

    let mut file = tokio::fs::File::open(&file_path).await.map_err(|e| format!("Failed to open file: {}", e))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut bytes_sent = 0u64;
    let start_time = std::time::Instant::now();
    let mut last_emit = std::time::Instant::now();

    loop {
        let n = file.read(&mut buf).await.map_err(|e| format!("File read error: {}", e))?;
        if n == 0 {
            break;
        }
        stream.write_all(&buf[..n]).await.map_err(|e| format!("Network write error: {}", e))?;
        bytes_sent += n as u64;

        let now = std::time::Instant::now();
        if now.duration_since(last_emit) >= std::time::Duration::from_millis(200) || bytes_sent == filesize {
            last_emit = now;
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 { bytes_sent as f64 / elapsed } else { 0.0 };
            app.emit("send-progress", serde_json::json!({
                "transfer_id": transfer_id,
                "bytes_sent": bytes_sent,
                "total_bytes": filesize,
                "speed": speed,
            })).ok();
        }
    }

    stream.flush().await.ok();

    app.emit("send-success", serde_json::json!({
        "transfer_id": transfer_id,
    })).ok();

    {
        let mut active = state.active_transfers.lock().await;
        active.remove(&transfer_id);
    }

    Ok(())
}

// ── Tauri Commands ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_local_transfer_info(
    state: tauri::State<'_, TransferState>,
) -> Result<LocalTransferInfo, String> {
    let ips = get_local_ips();
    let port = *state.local_port.lock().await;
    let hostname = get_system_hostname();
    Ok(LocalTransferInfo { ips, port, hostname })
}

#[tauri::command]
pub async fn get_transfer_config(
    state: tauri::State<'_, TransferState>,
) -> Result<ConfigData, String> {
    let conf = state.config.read().await;
    Ok(conf.clone())
}

#[tauri::command]
pub async fn update_save_dir(
    state: tauri::State<'_, TransferState>,
    save_dir: Option<String>,
) -> Result<(), String> {
    let mut conf = state.config.write().await;
    conf.save_dir = save_dir;
    state.save_config(&conf)?;
    Ok(())
}

#[tauri::command]
pub async fn select_save_dir(
    state: tauri::State<'_, TransferState>,
) -> Result<Option<String>, String> {
    let save_path = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new().pick_folder()
    }).await.map_err(|e| e.to_string())?;

    if let Some(path) = save_path {
        let path_str = path.to_string_lossy().to_string();
        let mut conf = state.config.write().await;
        conf.save_dir = Some(path_str.clone());
        state.save_config(&conf)?;
        Ok(Some(path_str))
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub async fn remove_trusted_peer(
    app: tauri::AppHandle,
    state: tauri::State<'_, TransferState>,
    ip: String,
    port: u16,
) -> Result<(), String> {
    let mut conf = state.config.write().await;
    conf.trusted_peers.retain(|p| !(p.ip == ip && p.port == port));
    state.save_config(&conf)?;
    app.emit("trusted-peers-updated", ()).ok();
    Ok(())
}

#[tauri::command]
pub async fn update_peer_alias(
    app: tauri::AppHandle,
    state: tauri::State<'_, TransferState>,
    ip: String,
    port: u16,
    alias: String,
) -> Result<(), String> {
    let mut conf = state.config.write().await;
    if let Some(peer) = conf.trusted_peers.iter_mut().find(|p| p.ip == ip && p.port == port) {
        peer.alias = alias;
        state.save_config(&conf)?;
        app.emit("trusted-peers-updated", ()).ok();
        Ok(())
    } else {
        Err("Peer not found".to_string())
    }
}

#[tauri::command]
pub async fn send_friend_request(
    app: tauri::AppHandle,
    state: tauri::State<'_, TransferState>,
    ip: String,
    port: u16,
) -> Result<String, String> {
    let my_hostname = get_system_hostname();
    let my_local_port = {
        let p = state.local_port.lock().await;
        *p
    };
    let req = serde_json::json!({
        "type": "FriendRequest",
        "sender_name": my_hostname,
        "sender_port": my_local_port,
    });

    let addr = format!("{}:{}", ip, port);
    let mut stream = TcpStream::connect(&addr).await.map_err(|e| format!("Failed to connect to {}: {}", addr, e))?;

    write_frame(&mut stream, &req.to_string()).await?;

    let frame = read_frame(&mut stream).await?;
    let resp: serde_json::Value = serde_json::from_str(&frame).map_err(|e| e.to_string())?;
    let accepted = resp["accepted"].as_bool().unwrap_or(false);
    let peer_hostname = resp["sender_name"].as_str().unwrap_or("Unknown Device").to_string();
    let peer_port = resp["sender_port"].as_u64().unwrap_or(port as u64) as u16;

    if accepted {
        let mut conf = state.config.write().await;
        conf.trusted_peers.retain(|p| !(p.ip == ip && p.port == peer_port));
        conf.trusted_peers.push(PeerInfo {
            ip: ip.clone(),
            port: peer_port,
            hostname: peer_hostname.clone(),
            alias: String::new(),
            added_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        });
        state.save_config(&conf)?;
        app.emit("trusted-peers-updated", ()).ok();
        Ok(peer_hostname)
    } else {
        Err("Friend request rejected".to_string())
    }
}

#[tauri::command]
pub async fn respond_friend_request(
    state: tauri::State<'_, TransferState>,
    request_id: String,
    accept: bool,
) -> Result<(), String> {
    let mut pending = state.pending_friends.lock().await;
    if let Some(tx) = pending.remove(&request_id) {
        let _ = tx.send(accept);
        Ok(())
    } else {
        Err("Request not found or expired".to_string())
    }
}

#[tauri::command]
pub async fn send_file(
    app: tauri::AppHandle,
    state: tauri::State<'_, TransferState>,
    receiver_ip: String,
    receiver_port: u16,
    file_path: String,
) -> Result<String, String> {
    let path = std::path::Path::new(&file_path);
    if !path.is_file() {
        return Err("File not found".to_string());
    }
    let filename = path.file_name().unwrap().to_string_lossy().to_string();
    let filesize = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let transfer_id = format!("send-{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis());
    let transfer_id_task = transfer_id.clone();

    let state_clone = state.inner().clone();
    let app_clone = app.clone();
    let file_path_clone = file_path.clone();
    let receiver_addr = format!("{}:{}", receiver_ip, receiver_port);

    let handle = tokio::spawn(async move {
        if let Err(e) = send_file_task(app_clone.clone(), state_clone, transfer_id_task.clone(), receiver_addr, file_path_clone, filename, filesize).await {
            app_clone.emit("send-error", serde_json::json!({
                "transfer_id": transfer_id_task,
                "error_message": e,
            })).ok();
        }
    });

    {
        let mut active = state.active_transfers.lock().await;
        active.insert(transfer_id.clone(), handle);
    }

    Ok(transfer_id)
}

#[tauri::command]
pub async fn select_file_to_send() -> Result<Option<SelectedFileInfo>, String> {
    let picked = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new().pick_file()
    }).await.map_err(|e| e.to_string())?;

    if let Some(path) = picked {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Ok(Some(SelectedFileInfo {
            path: path.to_string_lossy().to_string(),
            name,
            size,
        }))
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub async fn cancel_transfer(
    state: tauri::State<'_, TransferState>,
    transfer_id: String,
) -> Result<(), String> {
    let mut active = state.active_transfers.lock().await;
    if let Some(handle) = active.remove(&transfer_id) {
        handle.abort();
        Ok(())
    } else {
        Err("Transfer task not found".to_string())
    }
}

#[tauri::command]
pub async fn delete_local_file(path: String) -> Result<(), String> {
    std::fs::remove_file(path).map_err(|e| e.to_string())
}
