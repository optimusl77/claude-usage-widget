use std::io::Write;
use std::time::Duration;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use usage_core::{auth, cache, usage};

struct AppState {
    http_client: reqwest::Client,
}

/// Debug-Logging fuer diesen Build: alles Relevante geht in eine Datei
/// (`<app-data-dir>/debug.log`), damit auch Fehler sichtbar sind, die vor
/// dem Anzeigen jeder UI passieren, und damit sie sich nach dem Release-Build
/// (kein Konsolenfenster, siehe main.rs) noch inspizieren lassen. Wird bei
/// jedem Programmstart neu angelegt (frueherer Inhalt geht verloren), damit
/// die Datei immer den letzten Lauf zeigt.
fn log_path() -> Option<std::path::PathBuf> {
    cache::app_data_dir().ok().map(|d| d.join("debug.log"))
}

fn log_line(msg: &str) {
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let line = format!("[{timestamp}] {msg}\n");
    eprint!("{line}");
    if let Some(path) = log_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

fn reset_log_file() {
    if let Some(path) = log_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, b"");
    }
}

/// Nimmt Log-Zeilen vom Frontend entgegen, damit JS-Fehler und UI-Events im
/// selben debug.log landen wie die Rust-seitigen Ereignisse - ein Log statt
/// zwei getrennter Quellen (Konsole ist im Release-Build ohnehin unsichtbar).
#[tauri::command]
fn log_frontend(message: String) {
    log_line(&format!("[frontend] {message}"));
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
    log_line("get_usage_status: called");

    let session = match auth::load_session() {
        Ok(s) => {
            log_line(&format!(
                "get_usage_status: session loaded (expires_at_ms={}, expired={}, subscription_type={:?}, rate_limit_tier={:?})",
                s.expires_at_ms,
                s.is_expired(),
                s.subscription_type,
                s.rate_limit_tier
            ));
            s
        }
        Err(auth::AuthError::NotLoggedIn(path)) => {
            log_line(&format!(
                "get_usage_status: not logged in, credentials file missing at {path:?}"
            ));
            return Ok(UsageStatus::NotLoggedIn);
        }
        Err(e) => {
            log_line(&format!("get_usage_status: auth error while loading session: {e}"));
            return Err(e.to_string());
        }
    };

    if session.is_expired() {
        log_line("get_usage_status: session is expired, returning SessionExpired");
        return Ok(UsageStatus::SessionExpired);
    }

    let dir = state_dir()?;
    log_line("get_usage_status: calling fetch_usage (live request to api.anthropic.com)");
    let poll_result = usage::fetch_usage(&state.http_client, &session.access_token).await;

    match poll_result {
        Ok(snapshot) => {
            log_line(&format!(
                "get_usage_status: fetch_usage OK - five_hour={:?}, seven_day={:?}, overage={:?}, representative_claim={:?}",
                snapshot.five_hour, snapshot.seven_day, snapshot.overage, snapshot.representative_claim
            ));
            let cache_data = cache::UsageCache { last_snapshot: Some(snapshot.clone()) };
            match cache::save_usage_cache(&dir, &cache_data) {
                Ok(()) => log_line("get_usage_status: usage cache saved"),
                Err(e) => log_line(&format!("get_usage_status: WARNING failed to save usage cache: {e}")),
            }
            Ok(UsageStatus::Ok { snapshot, subscription_type: session.subscription_type })
        }
        Err(e) => {
            log_line(&format!("get_usage_status: fetch_usage FAILED: {e}"));
            let stale = cache::load_usage_cache(&dir).ok().and_then(|c| c.last_snapshot);
            log_line(&format!(
                "get_usage_status: falling back to stale cache, available={}",
                stale.is_some()
            ));
            Ok(UsageStatus::Error { message: e.to_string(), stale_snapshot: stale })
        }
    }
}

#[tauri::command]
fn get_settings() -> Result<cache::Settings, String> {
    log_line("get_settings: called");
    let dir = state_dir()?;
    match cache::load_settings(&dir) {
        Ok(s) => {
            log_line(&format!("get_settings: loaded {s:?}"));
            Ok(s)
        }
        Err(e) => {
            log_line(&format!("get_settings: FAILED: {e}"));
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings: cache::Settings) -> Result<(), String> {
    log_line(&format!("save_settings: called with {settings:?}"));
    let dir = state_dir()?;
    cache::save_settings(&dir, &settings).map_err(|e| {
        log_line(&format!("save_settings: FAILED to write settings file: {e}"));
        e.to_string()
    })?;

    if let Some(widget) = app.get_webview_window("widget") {
        let _ = widget.set_always_on_top(settings.always_on_top);
        log_line(&format!("save_settings: applied always_on_top={}", settings.always_on_top));
    } else {
        log_line("save_settings: WARNING widget window not found");
    }

    let autolaunch = app.autolaunch();
    let is_enabled = autolaunch.is_enabled().unwrap_or(false);
    log_line(&format!(
        "save_settings: autostart requested={}, currently_enabled={}",
        settings.autostart, is_enabled
    ));
    if settings.autostart && !is_enabled {
        match autolaunch.enable() {
            Ok(()) => log_line("save_settings: autostart enabled"),
            Err(e) => log_line(&format!("save_settings: FAILED to enable autostart: {e}")),
        }
    } else if !settings.autostart && is_enabled {
        match autolaunch.disable() {
            Ok(()) => log_line("save_settings: autostart disabled"),
            Err(e) => log_line(&format!("save_settings: FAILED to disable autostart: {e}")),
        }
    }

    let _ = app.emit("settings:changed", ());
    log_line("save_settings: done, emitted settings:changed");
    Ok(())
}

#[tauri::command]
fn show_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    log_line("show_settings_window: called");
    if let Some(win) = app.get_webview_window("settings") {
        win.show().map_err(|e| {
            log_line(&format!("show_settings_window: FAILED to show: {e}"));
            e.to_string()
        })?;
        win.set_focus().map_err(|e| {
            log_line(&format!("show_settings_window: FAILED to focus: {e}"));
            e.to_string()
        })?;
        log_line("show_settings_window: shown and focused");
    } else {
        log_line("show_settings_window: WARNING settings window not found");
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
    log_line("login_command: platform=windows, running `cmd /C claude login` with CREATE_NO_WINDOW");
    let mut cmd = tokio::process::Command::new("cmd");
    cmd.args(["/C", "claude", "login"]);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(not(target_os = "windows"))]
fn login_command() -> tokio::process::Command {
    log_line("login_command: platform=non-windows, running `claude login` directly");
    let mut cmd = tokio::process::Command::new("claude");
    cmd.arg("login");
    cmd
}

/// Startet den offiziellen Claude-Code-Login-Flow (echter Browser-OAuth ueber
/// Anthropic). Dieses Programm implementiert kein eigenes OAuth - es ruft nur
/// die vom Nutzer bereits installierte `claude` CLI auf und liest anschliessend
/// die Session, die sie selbst anlegt.
///
/// Wartet bewusst NICHT auf das Prozessende: der Login-Flow haengt an einer
/// Browser-Interaktion mit unbekannter Dauer, und der CLI-Prozess muss nach
/// erfolgreichem Login nicht zwingend zeitnah sauber terminieren. Das Frontend
/// pollt stattdessen selbst, bis die Session-Datei eine gueltige Session zeigt.
/// stdout/stderr des Kindprozesses werden mitgeschnitten und geloggt, damit
/// z.B. eine "'claude' is not recognized"-Meldung von cmd.exe sichtbar wird,
/// obwohl kein Konsolenfenster angezeigt wird.
#[tauri::command]
async fn start_claude_login() -> Result<(), String> {
    log_line("start_claude_login: called");

    let mut cmd = login_command();
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = format!(
                "Konnte 'claude login' nicht starten (ist die Claude Code CLI installiert und im PATH?): {e}"
            );
            log_line(&format!(
                "start_claude_login: spawn FAILED: {msg} (io_error_kind={:?}, raw_os_error={:?})",
                e.kind(),
                e.raw_os_error()
            ));
            return Err(msg);
        }
    };

    log_line(&format!("start_claude_login: spawned successfully, pid={:?}", child.id()));

    if let Some(stdout) = child.stdout.take() {
        tauri::async_runtime::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => log_line(&format!("[claude login stdout] {line}")),
                    Ok(None) => break,
                    Err(e) => {
                        log_line(&format!("[claude login stdout] read error: {e}"));
                        break;
                    }
                }
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        tauri::async_runtime::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stderr).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => log_line(&format!("[claude login stderr] {line}")),
                    Ok(None) => break,
                    Err(e) => {
                        log_line(&format!("[claude login stderr] read error: {e}"));
                        break;
                    }
                }
            }
        });
    }

    // Bewusst kein child.wait().await hier - siehe Doc-Kommentar oben.
    // Der Child-Prozess laeuft unabhaengig weiter; wenn er beendet, ohne dass
    // stdout/stderr etwas Nennenswertes gesagt haben, ist das normal (z.B.
    // der `claude`-Prozess uebergibt an den Browser und beendet sich selbst).
    tauri::async_runtime::spawn(async move {
        match child.wait().await {
            Ok(status) => log_line(&format!("start_claude_login: child process exited with {status}")),
            Err(e) => log_line(&format!("start_claude_login: error waiting on child process: {e}")),
        }
    });

    Ok(())
}

pub fn run() {
    reset_log_file();
    log_line("=== Claude Usage Widget starting ===");
    log_line(&format!("version: {}", env!("CARGO_PKG_VERSION")));
    log_line(&format!("target_os: {}", std::env::consts::OS));
    if let Some(path) = log_path() {
        log_line(&format!("log file path: {path:?}"));
    }
    match auth::credentials_path() {
        Ok(p) => log_line(&format!("expected credentials file path: {p:?} (exists={})", p.exists())),
        Err(e) => log_line(&format!("WARNING could not determine credentials path: {e}")),
    }

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
            log_frontend,
        ])
        .setup(|app| {
            log_line("setup: starting");
            let handle = app.handle().clone();
            let dir = cache::app_data_dir()?;
            log_line(&format!("setup: app data dir = {dir:?}"));
            let settings = cache::load_settings(&dir).unwrap_or_else(|e| {
                log_line(&format!("setup: could not load settings ({e}), using defaults"));
                cache::Settings::default()
            });
            log_line(&format!("setup: settings = {settings:?}"));

            if let Some(widget) = app.get_webview_window("widget") {
                log_line("setup: widget window found");
                let _ = widget.set_always_on_top(settings.always_on_top);
                if let (Some(x), Some(y)) = (settings.window_x, settings.window_y) {
                    log_line(&format!("setup: restoring widget position to ({x}, {y})"));
                    let _ = widget.set_position(tauri::PhysicalPosition::new(x, y));
                }
                match widget.show() {
                    Ok(()) => log_line("setup: widget window shown"),
                    Err(e) => log_line(&format!("setup: WARNING failed to show widget window: {e}")),
                }

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
            } else {
                log_line("setup: WARNING widget window not found (check tauri.conf.json)");
            }

            if let Some(settings_win) = app.get_webview_window("settings") {
                log_line("setup: settings window found, wiring close-to-hide behavior");
                let settings_win_for_close = settings_win.clone();
                settings_win.on_window_event(move |event| {
                    log_line(&format!("settings window event: {event:?}"));
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        log_line("settings window: CloseRequested - preventing close, hiding instead");
                        api.prevent_close();
                        match settings_win_for_close.hide() {
                            Ok(()) => log_line("settings window: hide() succeeded"),
                            Err(e) => log_line(&format!("settings window: hide() FAILED: {e}")),
                        }
                    }
                });
            } else {
                log_line("setup: WARNING settings window not found (check tauri.conf.json)");
            }

            let refresh_item = MenuItem::with_id(app, "refresh", "Jetzt aktualisieren", true, None::<&str>)?;
            let settings_item = MenuItem::with_id(app, "settings", "Einstellungen", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Beenden", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&refresh_item, &settings_item, &quit_item])?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().unwrap())
                .menu(&tray_menu)
                .tooltip("Claude Usage")
                .on_menu_event(|app, event| {
                    log_line(&format!("tray menu event: {}", event.id.as_ref()));
                    match event.id.as_ref() {
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
                            log_line("=== Claude Usage Widget exiting via tray menu ===");
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .build(app)?;
            log_line("setup: tray icon built");

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

                    log_line(&format!("background poll: emitting usage:poll-tick (next in {interval_secs}s)"));
                    let _ = handle.emit("usage:poll-tick", ());
                    tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                }
            });

            log_line("setup: complete");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
