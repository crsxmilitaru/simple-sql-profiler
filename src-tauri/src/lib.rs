mod db;
mod profiler;
mod settings;

use db::ConnectionConfig;
use profiler::{spawn_profiler_task, CaptureOptions, ProfilerCommand, QueryResultData};
use std::backtrace::Backtrace;
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
async fn disconnect_from_server(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ProfilerCommand::Disconnect { reply: reply_tx })
        .await
        .map_err(|e| format!("Internal error: {e}"))?;

    reply_rx.await.map_err(|e| format!("Internal error: {e}"))?
}

#[tauri::command]
async fn start_capture(
    state: tauri::State<'_, AppState>,
    options: Option<CaptureOptions>,
) -> Result<(), String> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ProfilerCommand::StartCapture {
            options: options.unwrap_or_default(),
            reply: reply_tx,
        })
        .await
        .map_err(|e| format!("Internal error: {e}"))?;

    reply_rx.await.map_err(|e| format!("Internal error: {e}"))?
}

#[tauri::command]
async fn stop_capture(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ProfilerCommand::StopCapture { reply: reply_tx })
        .await
        .map_err(|e| format!("Internal error: {e}"))?;

    reply_rx.await.map_err(|e| format!("Internal error: {e}"))?
}

#[tauri::command]
async fn execute_query(
    state: tauri::State<'_, AppState>,
    sql: String,
) -> Result<QueryResultData, String> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ProfilerCommand::ExecuteQuery {
            sql,
            reply: reply_tx,
        })
        .await
        .map_err(|e| format!("Internal error: {e}"))?;

    reply_rx.await.map_err(|e| format!("Internal error: {e}"))?
}

#[tauri::command]
async fn load_connection(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let (conn, password) = settings::load(&app)?;
    let mut val = serde_json::to_value(&conn).map_err(|e| format!("Serialization error: {e}"))?;
    let Some(obj) = val.as_object_mut() else {
        return Err("Serialized connection settings were not an object".into());
    };
    obj.insert("password".into(), password.into());
    Ok(val)
}

pub fn run() {
    std::panic::set_hook(Box::new(|panic_info| {
        let location = panic_info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_else(|| "unknown location".to_string());

        let payload = if let Some(message) = panic_info.payload().downcast_ref::<&str>() {
            (*message).to_string()
        } else if let Some(message) = panic_info.payload().downcast_ref::<String>() {
            message.clone()
        } else {
            "non-string panic payload".to_string()
        };

        eprintln!("panic at {location}: {payload}");
        eprintln!("{}", Backtrace::force_capture());
    }));

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            execute_query,
            load_connection,
        ])
        .run(tauri::generate_context!());

    if let Err(error) = app {
        eprintln!("error while running tauri application: {error}");
    }
}
