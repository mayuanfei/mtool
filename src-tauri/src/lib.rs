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

use base64::prelude::*;
use std::io::Cursor;
use qrcode::{QrCode, EcLevel};
use image::Rgb;
use arboard::{Clipboard, ImageData};
use std::fs;

#[tauri::command]
fn generate_qr(payload: &str, redundancy: &str, resolution: u32, color: &str) -> Result<String, String> {
    if payload.is_empty() {
        return Err("Payload is empty".to_string());
    }

    let ec_level = match redundancy {
        "L" => EcLevel::L,
        "M" => EcLevel::M,
        "Q" => EcLevel::Q,
        "H" => EcLevel::H,
        _ => EcLevel::M,
    };

    let code = QrCode::with_error_correction_level(payload, ec_level)
        .map_err(|e| format!("QR Code generation failed: {}", e))?;

    let hex = color.trim_start_matches('#');
    let r = u8::from_str_radix(hex.get(0..2).unwrap_or("00"), 16).unwrap_or(0);
    let g = u8::from_str_radix(hex.get(2..4).unwrap_or("00"), 16).unwrap_or(0);
    let b = u8::from_str_radix(hex.get(4..6).unwrap_or("00"), 16).unwrap_or(0);

    let image = code.render::<Rgb<u8>>()
        .min_dimensions(resolution, resolution)
        .dark_color(Rgb([r, g, b]))
        .light_color(Rgb([255, 255, 255]))
        .build();

    let mut buf = Vec::new();
    let mut cursor = Cursor::new(&mut buf);
    image.write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| format!("Failed to write PNG: {}", e))?;

    Ok(BASE64_STANDARD.encode(&buf))
}

#[tauri::command]
fn read_text_from_clipboard() -> Result<String, String> {
    let mut clipboard = Clipboard::new().map_err(|e| format!("Failed to init clipboard: {}", e))?;
    clipboard.get_text().map_err(|e| format!("Clipboard read error: {}", e))
}

#[tauri::command]
fn copy_qr_to_clipboard(base64_str: &str) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|e| format!("Failed to init clipboard: {}", e))?;
    let img_bytes = BASE64_STANDARD.decode(base64_str).map_err(|e| format!("Base64 decode error: {}", e))?;
    let img = image::load_from_memory(&img_bytes).map_err(|e| format!("Image load error: {}", e))?;
    let img_rgba = img.to_rgba8();
    
    let img_data = ImageData {
        width: img_rgba.width() as usize,
        height: img_rgba.height() as usize,
        bytes: img_rgba.into_raw().into(),
    };
    
    clipboard.set_image(img_data).map_err(|e| format!("Clipboard write error: {}", e))?;
    Ok(())
}

#[tauri::command]
fn download_qr(base64_str: &str) -> Result<(), String> {
    let img_bytes = BASE64_STANDARD.decode(base64_str).map_err(|e| format!("Base64 decode error: {}", e))?;
    
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("PNG Image", &["png"])
        .set_file_name("qrcode.png")
        .save_file() 
    {
        fs::write(path, img_bytes).map_err(|e| format!("Failed to save file: {}", e))?;
    }
    
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet, format_json, minify_json, generate_qr, read_text_from_clipboard, copy_qr_to_clipboard, download_qr])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
