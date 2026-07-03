// System tray + close-to-tray (opt-in, default off).
//
// The tray icon only exists while the "Close to tray" setting is enabled — the
// frontend mirrors the persisted setting into `set_close_to_tray` on startup
// and whenever the toggle changes. While enabled, closing the main window hides
// it instead (see the CloseRequested hook in lib.rs); the tray menu and a
// left-click on the icon bring it back. Playback controls in the menu reuse the
// existing `media-control` event that the SMTC/media-key path already feeds
// into PlayerControls.vue, so behavior is identical to pressing a media key.

use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State};

pub struct TrayState {
    icon: Mutex<Option<TrayIcon>>,
    close_to_tray: AtomicBool,
}

impl TrayState {
    pub fn new() -> Self {
        Self {
            icon: Mutex::new(None),
            close_to_tray: AtomicBool::new(false),
        }
    }

    pub fn close_to_tray(&self) -> bool {
        self.close_to_tray.load(Ordering::SeqCst)
    }
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

#[tauri::command]
pub fn set_close_to_tray(
    app: AppHandle,
    state: State<TrayState>,
    enabled: bool,
) -> Result<(), String> {
    state.close_to_tray.store(enabled, Ordering::SeqCst);
    let mut icon = state.icon.lock();
    if enabled {
        if icon.is_none() {
            *icon = Some(build_tray(&app)?);
        }
    } else {
        // Dropping the TrayIcon removes it from the system tray.
        *icon = None;
    }
    Ok(())
}

fn build_tray(app: &AppHandle) -> Result<TrayIcon, String> {
    let item = |id: &str, label: &str| {
        MenuItem::with_id(app, id, label, true, None::<&str>).map_err(|e| e.to_string())
    };
    let show = item("tray-show", "Show ts-music")?;
    let toggle = item("tray-toggle", "Play / Pause")?;
    let next = item("tray-next", "Next Track")?;
    let prev = item("tray-prev", "Previous Track")?;
    let quit = item("tray-quit", "Quit ts-music")?;
    let sep1 = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    let sep2 = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    let menu = Menu::with_items(app, &[&show, &sep1, &toggle, &next, &prev, &sep2, &quit])
        .map_err(|e| e.to_string())?;

    TrayIconBuilder::with_id("main-tray")
        .icon(
            app.default_window_icon()
                .cloned()
                .ok_or_else(|| "no default window icon".to_string())?,
        )
        .tooltip("ts-music")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray-show" => show_main_window(app),
            // Same payloads handleMediaControl already understands (SMTC path).
            "tray-toggle" => {
                let _ = app.emit("media-control", "toggle");
            }
            "tray-next" => {
                let _ = app.emit("media-control", "next");
            }
            "tray-prev" => {
                let _ = app.emit("media-control", "previous");
            }
            "tray-quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)
        .map_err(|e| e.to_string())
}
