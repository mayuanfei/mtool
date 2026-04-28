#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn format_json(input: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(input)
        .map_err(|e| format!("Invalid JSON: {}", e))?;
    serde_json::to_string_pretty(&value)
        .map_err(|e| format!("Format error: {}", e))
}

#[tauri::command]
fn minify_json(input: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(input)
        .map_err(|e| format!("Invalid JSON: {}", e))?;
    serde_json::to_string(&value)
        .map_err(|e| format!("Minify error: {}", e))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet, format_json, minify_json])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
