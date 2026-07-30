use std::io::Write;
use std::time::Duration;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use usage_core::{auth, cache, estimate, usage};

struct AppState {
    http_client: reqwest::Client,
}

/// Base widget window size at widget_scale=1.0, in logical pixels. Must match
/// the "widget" window's width/height in tauri.conf.json.
const BASE_WIDGET_WIDTH: f64 = 300.0;
const BASE_WIDGET_HEIGHT: f64 = 210.0;

/// Debug logging for this build: everything relevant goes into a file
/// (`<app-data-dir>/debug.log`), so failures are visible even if they happen
/// before any UI shows up, and so they can still be inspected after a
/// release build (no console window, see main.rs). Rewritten fresh on every
/// program start so the file always reflects the most recent run.
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

/// Accepts log lines from the frontend, so JS errors and UI events land in
/// the same debug.log as the Rust-side events, one log instead of two
/// separate sources (the console is invisible in a release build anyway).
#[tauri::command]
fn log_frontend(message: String) {
    log_line(&format!("[frontend] {message}"));
}

/// Status the frontend needs for display. Deliberately a flat, serializable
/// model instead of the internal usage-core types directly, so frontend
/// changes aren't coupled to the Rust-internal structure.
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

/// Fills in each window's `estimated_full_unix` from history, using the pure
/// extrapolation logic in `usage_core::estimate`. Kept separate from the raw
/// header parsing in usage.rs since it's a derived value computed from
/// stored history, not something the API itself reports.
fn apply_estimates(snapshot: &mut usage::UsageSnapshot, cache: &cache::UsageCache) {
    if let Some(w) = snapshot.five_hour.as_mut() {
        let samples = cache.samples_for(|s| s.five_hour.as_ref());
        w.estimated_full_unix = estimate::estimate_full_at(&samples);
    }
    if let Some(w) = snapshot.seven_day.as_mut() {
        let samples = cache.samples_for(|s| s.seven_day.as_ref());
        w.estimated_full_unix = estimate::estimate_full_at(&samples);
    }
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

            let mut cache_data = cache::load_usage_cache(&dir).unwrap_or_default();
            cache_data.record(snapshot.clone());
            match cache::save_usage_cache(&dir, &cache_data) {
                Ok(()) => log_line("get_usage_status: usage cache saved"),
                Err(e) => log_line(&format!("get_usage_status: WARNING failed to save usage cache: {e}")),
            }

            let mut response_snapshot = snapshot;
            apply_estimates(&mut response_snapshot, &cache_data);
            log_line(&format!(
                "get_usage_status: estimates - five_hour_full={:?}, seven_day_full={:?}",
                response_snapshot.five_hour.as_ref().and_then(|w| w.estimated_full_unix),
                response_snapshot.seven_day.as_ref().and_then(|w| w.estimated_full_unix),
            ));

            Ok(UsageStatus::Ok { snapshot: response_snapshot, subscription_type: session.subscription_type })
        }
        Err(e) => {
            log_line(&format!("get_usage_status: fetch_usage FAILED: {e}"));
            let cache_data = cache::load_usage_cache(&dir).unwrap_or_default();
            let mut stale = cache_data.last_snapshot.clone();
            if let Some(s) = stale.as_mut() {
                apply_estimates(s, &cache_data);
            }
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

/// Resizes the widget window to match `widget_scale`, keeping the same
/// aspect ratio as the base size defined in tauri.conf.json. `resizable` is
/// left `false` in the config so users can't drag-resize it by accident;
/// this is the only intended way to change its size.
fn apply_widget_scale(app: &tauri::AppHandle, scale: f32) {
    if let Some(widget) = app.get_webview_window("widget") {
        let scale = (scale as f64).clamp(0.5, 3.0);
        let size = tauri::LogicalSize::new(BASE_WIDGET_WIDTH * scale, BASE_WIDGET_HEIGHT * scale);
        match widget.set_size(size) {
            Ok(()) => log_line(&format!("apply_widget_scale: resized widget window to {size:?} (scale={scale})")),
            Err(e) => log_line(&format!("apply_widget_scale: FAILED to resize widget window: {e}")),
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
    apply_widget_scale(&app, settings.widget_scale);

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

/// Builds the login command. On Windows, `claude` (from a global npm install)
/// is a `claude.cmd` shim that `CreateProcess` can't resolve without a
/// shell, so it runs through `cmd /C` there. CREATE_NO_WINDOW suppresses the
/// console window that would otherwise briefly flash; the browser OAuth flow
/// itself doesn't need a visible console.
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

/// Starts the official Claude Code login flow (real browser OAuth through
/// Anthropic). This app implements no OAuth of its own, it only calls the
/// `claude` CLI the user already has installed and later reads the session
/// it creates.
///
/// Deliberately does NOT wait for the process to exit: the login flow hangs
/// on a browser interaction of unknown duration, and the CLI process isn't
/// guaranteed to terminate cleanly right after a successful login. The
/// frontend polls instead, until the session file shows a valid session.
/// The child's stdout/stderr are captured and logged, so e.g. a "'claude' is
/// not recognized" message from cmd.exe is visible even though no console
/// window is shown.
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
                "Could not start 'claude login' (is the Claude Code CLI installed and on your PATH?): {e}"
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

    // Deliberately no child.wait().await here, see the doc comment above.
    // The child process keeps running independently; if it exits without
    // stdout/stderr having said anything notable, that's normal (e.g. the
    // `claude` process hands off to the browser and exits on its own).
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
            apply_widget_scale(&handle, settings.widget_scale);

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

            let refresh_item = MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?;
            let settings_item = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
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

            // Background polling: re-reads the configured interval on every
            // tick, so settings changes take effect without a restart.
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
