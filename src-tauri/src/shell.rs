//! macOS app shell: menu bar (tray) icon, Dock badge, app menu, hide-on-close
//! and launch-at-login. Friction keeps protecting while its window is closed;
//! it only stops when the user quits.

use crate::proxy::AppState;
use std::sync::Arc;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_autostart::ManagerExt as _;

pub struct TrayHandles {
    status: MenuItem<Wry>,
    login: CheckMenuItem<Wry>,
}

pub fn show_main(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.show();
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Show the window and switch the UI to a view ("intercepts" | "audit" | "rules").
pub fn open_view(app: &AppHandle, view: &str) {
    show_main(app);
    let _ = app.emit("navigate", view);
}

pub fn autostart_enabled(app: &AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

pub fn set_autostart(app: &AppHandle, enabled: bool) -> bool {
    let res = if enabled { app.autolaunch().enable() } else { app.autolaunch().disable() };
    if let Err(e) = res {
        eprintln!("[friction] launch-at-login change failed: {e}");
    }
    let now = autostart_enabled(app);
    if let Some(h) = app.try_state::<TrayHandles>() {
        let _ = h.login.set_checked(now);
    }
    let _ = app.emit("autostart:changed", now);
    now
}

/// Updates the menu bar title, status line and Dock badge from current state.
pub fn refresh(app: &AppHandle) {
    let Some(state) = app.try_state::<Arc<AppState>>() else { return };
    let pending = state.pending.lock().unwrap().len();
    let kind = state.proxy_status.lock().unwrap().as_ref().map(|s| s.kind.clone());

    let label = match kind.as_deref() {
        Some("ok") if pending == 0 => "Protecting · nothing waiting".to_string(),
        Some("ok") => format!(
            "{pending} transaction{} waiting for you",
            if pending == 1 { "" } else { "s" }
        ),
        Some("port_in_use") => "Proxy offline · ports 8080–8099 in use".to_string(),
        Some(_) => "Proxy offline".to_string(),
        None => "Starting…".to_string(),
    };

    if let Some(h) = app.try_state::<TrayHandles>() {
        let _ = h.status.set_text(label);
    }
    if let Some(tray) = app.tray_by_id("main") {
        let title: Option<String> = (pending > 0).then(|| pending.to_string());
        let _ = tray.set_title(title);
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_badge_count((pending > 0).then_some(pending as i64));
    }
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    // --- App menu: standard macOS menu plus "Settings…" (⌘,) -----------------
    let menu = Menu::default(app)?;
    if let Some(first) = menu.items()?.first() {
        if let Some(app_menu) = first.as_submenu() {
            let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
            app_menu.insert(&PredefinedMenuItem::separator(app)?, 1)?;
            app_menu.insert(&settings, 2)?;
        }
    }
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        if event.id().as_ref() == "settings" {
            open_view(app, "rules");
        }
    });

    // --- Menu bar icon ---------------------------------------------------------
    let status = MenuItem::with_id(app, "status", "Starting…", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open Friction", true, None::<&str>)?;
    let audit = MenuItem::with_id(app, "audit", "Audit Log", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "tray-settings", "Settings…", true, None::<&str>)?;
    let login = CheckMenuItem::with_id(app, "login", "Open at Login", true, autostart_enabled(app), None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Friction", true, None::<&str>)?;
    let tray_menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &open,
            &audit,
            &settings,
            &PredefinedMenuItem::separator(app)?,
            &login,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    TrayIconBuilder::with_id("main")
        .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?)
        .icon_as_template(true)
        .tooltip("Friction")
        .menu(&tray_menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => open_view(app, "intercepts"),
            "audit" => open_view(app, "audit"),
            "tray-settings" => open_view(app, "rules"),
            "login" => {
                let want = !autostart_enabled(app);
                set_autostart(app, want);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    app.manage(TrayHandles { status, login });
    refresh(app);
    Ok(())
}

/// First time the window is closed, explain that protection continues.
pub fn note_hidden_once(app: &AppHandle) {
    let marker = crate::config::data_dir().join(".closed-once");
    if marker.exists() {
        return;
    }
    let _ = std::fs::write(&marker, b"1");
    crate::proxy::notify(
        app,
        "Friction is still protecting you",
        "Closing the window keeps the proxy running. Use the shield in the menu bar to reopen or quit.",
    );
}
