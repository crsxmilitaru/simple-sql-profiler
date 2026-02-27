mod db;
mod profiler;
mod settings;

use db::ConnectionConfig;
use profiler::{ProfilerCommand, spawn_profiler_task};
use tauri::Manager;
use tokio::sync::{mpsc, oneshot};

struct AppState {
    tx: mpsc::Sender<ProfilerCommand>,
}

#[tauri::command]
async fn connect_to_server(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    config: ConnectionConfig,
    remember_password: bool,
) -> Result<(), String> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ProfilerCommand::Connect {
            config: config.clone(),
            reply: reply_tx,
        })
        .await
        .map_err(|e| format!("Internal error: {e}"))?;

    reply_rx
        .await
        .map_err(|e| format!("Internal error: {e}"))??;

    let saved = settings::SavedConnection {
        server_name: config.server_name,
        authentication: config.authentication,
        username: config.username,
        database: config.database,
        encrypt: config.encrypt,
        trust_cert: config.trust_cert,
        remember_password,
    };
    settings::save(&app, &saved, &config.password)?;

    Ok(())
}

#[tauri::command]
async fn disconnect_from_server(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ProfilerCommand::Disconnect { reply: reply_tx })
        .await
        .map_err(|e| format!("Internal error: {e}"))?;

    reply_rx
        .await
        .map_err(|e| format!("Internal error: {e}"))?
}

#[tauri::command]
async fn start_capture(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ProfilerCommand::StartCapture { reply: reply_tx })
        .await
        .map_err(|e| format!("Internal error: {e}"))?;

    reply_rx
        .await
        .map_err(|e| format!("Internal error: {e}"))?
}

#[tauri::command]
async fn stop_capture(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ProfilerCommand::StopCapture { reply: reply_tx })
        .await
        .map_err(|e| format!("Internal error: {e}"))?;

    reply_rx
        .await
        .map_err(|e| format!("Internal error: {e}"))?
}

#[tauri::command]
async fn load_connection(
    app: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    let (conn, password) = settings::load(&app)?;
    let mut val = serde_json::to_value(&conn)
        .map_err(|e| format!("Serialization error: {e}"))?;
    val.as_object_mut().unwrap().insert("password".into(), password.into());
    Ok(val)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let tx = spawn_profiler_task(app.handle().clone());
            app.manage(AppState { tx });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            connect_to_server,
            disconnect_from_server,
            start_capture,
            stop_capture,
            load_connection,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
