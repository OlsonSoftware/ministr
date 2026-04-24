// Tauri command exports — server-side Rust
#[tauri::command]
fn greet(name: String) -> String {
    format!("Hello, {name}!")
}

#[tauri::command]
fn get_settings() -> Settings {
    Settings::default()
}

#[tauri::command]
fn save_file(path: String, content: String) -> Result<(), String> {
    Ok(())
}
