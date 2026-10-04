// Friction: a local firewall for AI agents
// Tauri v2 entry point: starts the inspecting proxy and exposes commands to the UI.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audit;
mod ca;
mod config;
mod inspect;
mod flight;
mod meter;
mod payments;
mod policy;
mod proxy;
mod rules_file;
mod spend;
mod secrets;
mod shell;

use proxy::{AppState, Decision, ProxyStatus};
use serde::Serialize;
use std::sync::Arc;
use tauri::{Emitter, Manager, RunEvent, State, WindowEvent};
use tauri_plugin_notification::{NotificationExt, PermissionState};

type AppStateRef<'a> = State<'a, Arc<AppState>>;

// ---------------------------------------------------------------------------
// Intercepts
// ---------------------------------------------------------------------------

#[tauri::command]
fn resolve_intercept(id: String, decision: String, state: AppStateRef) -> bool {
    let decision = match decision.as_str() {
        "approve" => Decision::Approve,
        "reject" => Decision::Reject,
        _ => return false,
    };
    state.resolve(&id, decision)
}

#[tauri::command]
fn get_pending_intercepts(state: AppStateRef) -> Vec<inspect::InterceptPayload> {
    state.pending_payloads()
}

#[tauri::command]
fn get_stats(state: AppStateRef) -> proxy::StatsSnapshot {
    state.stats.snapshot()
}

#[tauri::command]
fn send_demo_request(kind: Option<String>) {
    proxy::send_demo_request(kind.as_deref().unwrap_or("purchase"));
}

// ---------------------------------------------------------------------------
// Command-line tool
// ---------------------------------------------------------------------------

fn cli_candidates() -> Vec<std::path::PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    vec![
        "/opt/homebrew/bin".into(),
        "/usr/local/bin".into(),
        std::path::PathBuf::from(home).join(".local/bin"),
    ]
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CliStatus {
    installed_at: Option<String>,
}

#[tauri::command]
fn cli_status() -> CliStatus {
    let found = cli_candidates().into_iter().map(|d| d.join("friction")).find(|p| {
        std::fs::read_link(p)
            .map(|t| t.to_string_lossy().contains("Friction.app"))
            .unwrap_or(false)
    });
    CliStatus { installed_at: found.map(|p| p.to_string_lossy().to_string()) }
}

/// Symlinks the bundled `friction` script into the first writable bin directory.
#[tauri::command]
fn install_cli(app: tauri::AppHandle) -> Result<String, String> {
    let source = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join("friction");
    if !source.exists() {
        return Err("the friction script is missing from the app bundle".into());
    }
    let mut last_err = String::from("no writable bin directory");
    for (i, dir) in cli_candidates().into_iter().enumerate() {
        let is_local = i == 2;
        if !dir.exists() {
            if !is_local {
                continue;
            }
            if let Err(e) = std::fs::create_dir_all(&dir) {
                last_err = e.to_string();
                continue;
            }
        }
        let dest = dir.join("friction");
        if let Ok(meta) = std::fs::symlink_metadata(&dest) {
            let ours = meta.file_type().is_symlink()
                && std::fs::read_link(&dest)
                    .map(|t| t.to_string_lossy().contains("Friction.app"))
                    .unwrap_or(false);
            if !ours {
                last_err = format!("{} already exists", dest.display());
                continue;
            }
            let _ = std::fs::remove_file(&dest);
        }
        match std::os::unix::fs::symlink(&source, &dest) {
            Ok(()) => return Ok(dest.to_string_lossy().to_string()),
            Err(e) => last_err = format!("{}: {e}", dir.display()),
        }
    }
    Err(last_err)
}

#[tauri::command]
fn get_proxy_status(state: AppStateRef) -> Option<ProxyStatus> {
    state.proxy_status.lock().unwrap().clone()
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_config(state: AppStateRef) -> config::PublicConfig {
    state.public_config()
}

#[tauri::command]
fn update_config(patch: config::ConfigPatch, app: tauri::AppHandle, state: AppStateRef) -> config::PublicConfig {
    {
        let mut c = state.config.lock().unwrap();
        c.apply(patch);
        config::save(&c);
    }
    let public = state.public_config();
    let _ = app.emit("config:changed", public.clone());
    public
}


// ---------------------------------------------------------------------------
// Activity (flight recorder) and AI spend
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_flight(query: Option<flight::FlightQuery>, state: AppStateRef) -> Vec<flight::FlightEntry> {
    state.flight.list(&query.unwrap_or_default())
}

#[tauri::command]
fn get_flight_entry(id: String, state: AppStateRef) -> Option<flight::FlightDetail> {
    state.flight.get(&id)
}

/// Asks where to save, then writes the HAR file. Returns the path, or an
/// empty string when the user cancels.
#[tauri::command]
async fn export_har(
    query: Option<flight::FlightQuery>,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Export activity")
        .set_file_name(format!("friction-{}.har", chrono::Local::now().format("%Y-%m-%d-%H%M")))
        .add_filter("HTTP Archive", &["har"]);
    if let Ok(home) = std::env::var("HOME") {
        dialog = dialog.set_directory(std::path::PathBuf::from(home).join("Downloads"));
    }
    let Some(picked) = dialog.blocking_save_file() else {
        return Ok(String::new());
    };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    state.flight.export_har(&query.unwrap_or_default(), &path)?;
    let _ = std::process::Command::new("/usr/bin/open").arg("-R").arg(&path).status();
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn clear_flight(state: AppStateRef) -> bool {
    state.flight.clear()
}

#[tauri::command]
fn clear_spend(app: tauri::AppHandle, state: AppStateRef) -> bool {
    state.ledger.lock().unwrap().clear();
    let _ = app.emit("spend:changed", state.spend_summary());
    true
}

#[tauri::command]
fn get_spend(state: AppStateRef) -> meter::SpendSummary {
    state.spend_summary()
}

#[tauri::command]
fn set_ai_cap(cap: Option<f64>, app: tauri::AppHandle, state: AppStateRef) -> meter::SpendSummary {
    {
        let mut c = state.config.lock().unwrap();
        c.daily_ai_cap = cap.filter(|v| v.is_finite() && *v > 0.0);
        config::save(&c);
    }
    let summary = state.spend_summary();
    let _ = app.emit("spend:changed", summary.clone());
    let _ = app.emit("config:changed", state.public_config());
    summary
}

// ---------------------------------------------------------------------------
// rules.toml
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_rules_file_status(state: AppStateRef) -> policy::RulesFileStatus {
    state.rules.lock().unwrap().status.clone()
}

#[tauri::command]
fn open_rules_file() -> bool {
    policy::ensure_template();
    std::process::Command::new("/usr/bin/open")
        .arg("-t")
        .arg(policy::path())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[tauri::command]
fn reveal_rules_file() -> bool {
    policy::ensure_template();
    std::process::Command::new("/usr/bin/open")
        .arg("-R")
        .arg(policy::path())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Reloads rules.toml whenever it changes on disk.
fn watch_rules_file(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            let state = app.state::<Arc<AppState>>();
            let next = {
                let loaded = state.rules.lock().unwrap();
                policy::changed(&loaded).then(|| policy::reload(&loaded))
            };
            let Some(next) = next else { continue };
            let status = next.status.clone();
            *state.rules.lock().unwrap() = next;
            let _ = app.emit("rules:changed", status.clone());
            let _ = app.emit("config:changed", state.public_config());
            let _ = app.emit("spend:changed", state.spend_summary());
            if let Some(e) = status.error {
                proxy::notify(
                    &app,
                    "rules.toml has an error",
                    &format!("{e}. Friction keeps using your previous rules until it's fixed."),
                );
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_audit_log(state: AppStateRef) -> Vec<audit::AuditEntry> {
    let _guard = state.audit_lock.lock().unwrap();
    audit::load()
}

#[tauri::command]
fn clear_audit_log(state: AppStateRef) -> bool {
    let _guard = state.audit_lock.lock().unwrap();
    audit::save(&[])
}

// ---------------------------------------------------------------------------
// HTTPS certificate
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CaInfo {
    available: bool,
    path: Option<String>,
    trusted: bool,
    error: Option<String>,
}

#[tauri::command]
async fn get_ca_info(state: State<'_, Arc<AppState>>) -> Result<CaInfo, ()> {
    Ok(match state.ca.as_ref() {
        Some(ca) => CaInfo {
            available: true,
            path: Some(ca.cert_path.to_string_lossy().to_string()),
            trusted: ca.is_trusted(),
            error: None,
        },
        None => CaInfo { available: false, path: None, trusted: false, error: state.ca_error.clone() },
    })
}

#[tauri::command]
async fn install_ca(state: State<'_, Arc<AppState>>) -> Result<bool, String> {
    let ca = state.ca.as_ref().ok_or("certificate authority unavailable")?;
    ca.install_trust()?;
    Ok(ca.is_trusted())
}

#[tauri::command]
fn reveal_ca(state: AppStateRef) -> bool {
    match state.ca.as_ref() {
        Some(ca) => std::process::Command::new("/usr/bin/open")
            .arg("-R")
            .arg(&ca.cert_path)
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

#[tauri::command]
fn request_notification_permission(app: tauri::AppHandle) -> String {
    let n = app.notification();
    let state = match n.permission_state() {
        Ok(PermissionState::Granted) => PermissionState::Granted,
        _ => n.request_permission().unwrap_or(PermissionState::Denied),
    };
    match state {
        PermissionState::Granted => "granted".into(),
        PermissionState::Denied => "denied".into(),
        _ => "default".into(),
    }
}

#[tauri::command]
fn send_native_notification(title: String, body: String, app: tauri::AppHandle) -> bool {
    app.notification().builder().title(title).body(body).show().is_ok()
}

// ---------------------------------------------------------------------------
// Launch at login
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_autostart(app: tauri::AppHandle) -> bool {
    shell::autostart_enabled(&app)
}

#[tauri::command]
fn set_autostart(enabled: bool, app: tauri::AppHandle) -> bool {
    shell::set_autostart(&app, enabled)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    // rustls needs one process-wide crypto provider.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let state = Arc::new(AppState::new());

    let launched_hidden = std::env::args().any(|a| a == "--hidden");

    let app = tauri::Builder::default()
        // Must be first: a second launch just brings the running copy forward.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| shell::show_main(app)))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all() & !tauri_plugin_window_state::StateFlags::VISIBLE,
                )
                .build(),
        )
        .manage(state.clone())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let handle = app.handle().clone();
            shell::setup(&handle)?;
            watch_rules_file(handle.clone());

            let proxy_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                proxy::run(proxy_handle, state).await;
            });

            if let Some(window) = app.get_webview_window("main") {
                #[cfg(target_os = "macos")]
                {
                    use tauri::window::{Effect, EffectState, EffectsBuilder};
                    let _ = window.set_effects(
                        EffectsBuilder::new()
                            .effect(Effect::UnderWindowBackground)
                            .state(EffectState::Active)
                            .radius(10.0)
                            .build(),
                    );
                }
                // Started at login: stay in the menu bar until needed.
                if !launched_hidden {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            Ok(())
        })
        // Closing the window hides it; the proxy keeps protecting until Quit.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                    shell::note_hidden_once(window.app_handle());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            resolve_intercept,
            get_pending_intercepts,
            get_proxy_status,
            get_stats,
            send_demo_request,
            cli_status,
            install_cli,
            get_flight,
            get_flight_entry,
            export_har,
            clear_flight,
            get_spend,
            set_ai_cap,
            get_rules_file_status,
            open_rules_file,
            reveal_rules_file,
            get_config,
            update_config,
            get_audit_log,
            clear_audit_log,
            get_ca_info,
            install_ca,
            reveal_ca,
            request_notification_permission,
            send_native_notification,
            get_autostart,
            set_autostart,
            clear_spend
        ])
        .build(tauri::generate_context!())
        .expect("error while building Friction");

    app.run(|app, event| {
        // Clicking the Dock icon while the window is hidden reopens it.
        #[cfg(target_os = "macos")]
        if let RunEvent::Reopen { has_visible_windows, .. } = &event {
            if !has_visible_windows {
                shell::show_main(app);
            }
        }
        let _ = (app, event);
    });
}
