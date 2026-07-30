use std::time::Duration;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use usage_core::{auth, cache, usage};

struct AppState {
    http_client: reqwest::Client,
}

/// Status, den das Frontend zur Anzeige braucht. Bewusst ein flaches,
/// serialisierbares Modell statt der internen usage-core-Typen 1:1, damit
/// Frontend-Aenderungen nicht an die Rust-interne Struktur gekoppelt sind.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum UsageStatus {
    NotLoggedIn,
    SessionExpired,
    Ok { snapshot: usage::UsageSnapshot, subscription_type: Option<String> },
    Error { message: String, stale_snapshot: Option<usage::UsageSnapshot> },
}

fn state_dir() -> Result<std::path::PathBuf, String> {
    cache::app_data_dir().map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_usage_status(state: State<'_, AppState>) -> Result<UsageStatus, String> {
    let session = match auth::load_session() {
        Ok(s) => s,
        Err(auth::AuthError::NotLoggedIn(_)) => return Ok(UsageStatus::NotLoggedIn),
        Err(e) => return Err(e.to_string()),
    };

    if session.is_expired() {
        return Ok(UsageStatus::SessionExpired);
    }

    let dir = state_dir()?;
    let poll_result = usage::fetch_usage(&state.http_client, &session.access_token).await;

    match poll_result {
        Ok(snapshot) => {
            let cache_data = cache::UsageCache { last_snapshot: Some(snapshot.clone()) };
            let _ = cache::save_usage_cache(&dir, &cache_data);
            Ok(UsageStatus::Ok { snapshot, subscription_type: session.subscription_type })
        }
        Err(e) => {
            let stale = cache::load_usage_cache(&dir).ok().and_then(|c| c.last_snapshot);
            Ok(UsageStatus::Error { message: e.to_string(), stale_snapshot: stale })
        }
    }
}

#[tauri::command]
fn get_settings() -> Result<cache::Settings, String> {
    let dir = state_dir()?;
    cache::load_settings(&dir).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings: cache::Settings) -> Result<(), String> {
    let dir = state_dir()?;
    cache::save_settings(&dir, &settings).map_err(|e| e.to_string())?;

    if let Some(widget) = app.get_webview_window("widget") {
        let _ = widget.set_always_on_top(settings.always_on_top);
    }

    let autolaunch = app.autolaunch();
    let is_enabled = autolaunch.is_enabled().unwrap_or(false);
    if settings.autostart && !is_enabled {
        let _ = autolaunch.enable();
    } else if !settings.autostart && is_enabled {
        let _ = autolaunch.disable();
    }

    let _ = app.emit("settings:changed", ());
    Ok(())
}

#[tauri::command]
fn show_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("settings") {
        win.show().map_err(|e| e.to_string())?;
        win.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Baut den Login-Befehl. Auf Windows ist `claude` (npm-Global-Install) ein
/// `claude.cmd`-Shim, das `CreateProcess` ohne Shell nicht auflöst - deshalb
/// läuft es dort über `cmd /C`. CREATE_NO_WINDOW unterdrückt das sonst kurz
/// aufblitzende Konsolenfenster; der Browser-OAuth-Flow selbst braucht keine
/// sichtbare Konsole.
#[cfg(target_os = "windows")]
fn login_command() -> tokio::process::Command {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = tokio::process::Command::new("cmd");
    cmd.args(["/C", "claude", "login"]);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(not(target_os = "windows"))]
fn login_command() -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("claude");
    cmd.arg("login");
    cmd
}

/// Startet den offiziellen Claude-Code-Login-Flow (echter Browser-OAuth ueber
/// Anthropic). Dieses Programm implementiert kein eigenes OAuth - es ruft nur
/// die vom Nutzer bereits installierte `claude` CLI auf und liest anschliessend
/// die Session, die sie selbst anlegt.
#[tauri::command]
async fn start_claude_login() -> Result<(), String> {
    let mut child = login_command()
        .spawn()
        .map_err(|e| format!("Konnte 'claude login' nicht starten (ist die Claude Code CLI installiert und im PATH?): {e}"))?;

    let status = child.wait().await.map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("Login wurde nicht erfolgreich abgeschlossen.".to_string());
    }
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_opener::init())
        .manage(AppState { http_client: reqwest::Client::new() })
        .invoke_handler(tauri::generate_handler![
            get_usage_status,
            get_settings,
            save_settings,
            show_settings_window,
            start_claude_login,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let dir = cache::app_data_dir()?;
            let settings = cache::load_settings(&dir).unwrap_or_default();

            if let Some(widget) = app.get_webview_window("widget") {
                let _ = widget.set_always_on_top(settings.always_on_top);
                if let (Some(x), Some(y)) = (settings.window_x, settings.window_y) {
                    let _ = widget.set_position(tauri::PhysicalPosition::new(x, y));
                }
                let _ = widget.show();

                widget.on_window_event(move |event| {
                    if let tauri::WindowEvent::Moved(pos) = event {
                        if let Ok(dir) = state_dir() {
                            let mut s = cache::load_settings(&dir).unwrap_or_default();
                            s.window_x = Some(pos.x);
                            s.window_y = Some(pos.y);
                            let _ = cache::save_settings(&dir, &s);
                        }
                    }
                });
            }

            if let Some(settings_win) = app.get_webview_window("settings") {
                settings_win.on_window_event(|event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                    }
                });
            }

            let refresh_item = MenuItem::with_id(app, "refresh", "Jetzt aktualisieren", true, None::<&str>)?;
            let settings_item = MenuItem::with_id(app, "settings", "Einstellungen", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Beenden", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&refresh_item, &settings_item, &quit_item])?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().unwrap())
                .menu(&tray_menu)
                .tooltip("Claude Usage")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "refresh" => {
                        let _ = app.emit("usage:poll-tick", ());
                    }
                    "settings" => {
                        if let Some(win) = app.get_webview_window("settings") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            // Hintergrund-Polling: liest das konfigurierte Intervall bei jedem
            // Tick neu, damit Settings-Aenderungen ohne Neustart wirken.
            tauri::async_runtime::spawn(async move {
                loop {
                    let interval_secs = state_dir()
                        .ok()
                        .and_then(|dir| cache::load_settings(&dir).ok())
                        .map(|s| s.poll_interval_secs)
                        .unwrap_or(300)
                        .max(60);

                    let _ = handle.emit("usage:poll-tick", ());
                    tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
