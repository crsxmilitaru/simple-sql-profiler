use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

const KEYRING_SERVICE: &str = "simple-sql-profiler";
const KEYRING_USER: &str = "connection-password";
const SETTINGS_FILE: &str = "connection.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedConnection {
    pub server_name: String,
    pub authentication: String,
    pub username: String,
    pub database: String,
    pub encrypt: String,
    pub trust_cert: bool,
    pub remember_password: bool,
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("Failed to resolve config dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create config dir: {e}"))?;
    Ok(dir.join(SETTINGS_FILE))
}

pub fn save(app: &tauri::AppHandle, conn: &SavedConnection, password: &str) -> Result<(), String> {
    let path = settings_path(app)?;
    let json = serde_json::to_string_pretty(conn)
        .map_err(|e| format!("Failed to serialize settings: {e}"))?;
    fs::write(&path, json).map_err(|e| format!("Failed to write settings: {e}"))?;

    if conn.remember_password {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|e| format!("Keyring error: {e}"))?;
        entry
            .set_password(password)
            .map_err(|e| format!("Failed to save password: {e}"))?;
    } else {
        let _ = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .and_then(|e| e.delete_credential());
    }

    Ok(())
}

pub fn load(app: &tauri::AppHandle) -> Result<(SavedConnection, String), String> {
    let path = settings_path(app)?;
    let json = fs::read_to_string(&path).map_err(|e| format!("No saved connection: {e}"))?;
    let conn: SavedConnection =
        serde_json::from_str(&json).map_err(|e| format!("Invalid settings file: {e}"))?;

    let password = if conn.remember_password {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .and_then(|e| e.get_password())
            .unwrap_or_default()
    } else {
        String::new()
    };

    Ok((conn, password))
}
