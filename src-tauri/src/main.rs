// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod collect;
mod ffi;
mod gpu;
mod lhm;
mod pdh;
mod procs;
mod settings;

use collect::{Collector, Info, Snapshot};
use serde::Serialize;
use settings::Settings;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::image::Image;
use tauri::window::Color;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, Theme, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent, Wry};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

/// How much history the backend keeps for the graphs (10 minutes).
const HISTORY_MS: u64 = 600_000;

struct Hub {
    info: Info,
    history: Mutex<VecDeque<Snapshot>>,
    interval_ms: AtomicU64,
}

struct SettingsState(Mutex<Settings>);
struct AutostartItem(CheckMenuItem<Wry>);

#[derive(Serialize)]
struct Hello {
    info: Info,
    interval_ms: u64,
    history: Vec<Snapshot>,
    settings: Settings,
    monitors: Vec<MonitorRect>,
    autostart: bool,
    settings_path: String,
}

#[derive(Serialize, Clone)]
struct MonitorRect {
    index: usize,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    primary: bool,
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show(app)))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .invoke_handler(tauri::generate_handler![hello, save_settings, set_autostart, toggle_fullscreen, run_action, restart])
        .setup(|app| {
            let settings = Settings::load().clamp();
            let lhm = (!settings.lhm_url.is_empty()).then(|| settings.lhm_url.clone());
            let collector = Collector::new(settings.nvml, lhm);
            let hub = Arc::new(Hub {
                info: collector.info(),
                history: Mutex::new(VecDeque::new()),
                interval_ms: AtomicU64::new(settings.interval_ms),
            });
            app.manage(hub.clone());
            app.manage(SettingsState(Mutex::new(settings.clone())));
            start_sampler(app.handle().clone(), hub, collector);
            create_window(app.handle(), &settings)?;
            build_tray(app.handle())?;
            apply_theme(app.handle(), &settings.theme);
            place_window(app.handle(), &settings);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Closing hides to the tray; "Quit Oriel" lives in the tray menu.
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Oriel");
}

// --- sampling ------------------------------------------------------------------

fn start_sampler(app: AppHandle, hub: Arc<Hub>, mut collector: Collector) {
    std::thread::Builder::new()
        .name("sampler".into())
        .spawn(move || {
            let mut next = Instant::now();
            loop {
                let snap = collector.sample();
                let interval = hub.interval_ms.load(Ordering::Relaxed);
                {
                    let mut h = hub.history.lock().unwrap();
                    h.push_back(snap.slim());
                    let cap = (HISTORY_MS / interval) as usize;
                    while h.len() > cap {
                        h.pop_front();
                    }
                }
                // Only feed the page while it's on screen; history keeps filling regardless.
                let visible = app.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false));
                if visible {
                    let _ = app.emit_to("main", "sample", &snap);
                }
                next += Duration::from_millis(interval);
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else {
                    next = now; // fell behind (sleep/hibernate): don't try to catch up
                }
            }
        })
        .expect("failed to start sampler");
}

// --- commands (called from the dashboard) ---------------------------------------

#[tauri::command]
fn hello(app: AppHandle, hub: State<Arc<Hub>>, settings: State<SettingsState>) -> Hello {
    Hello {
        info: hub.info.clone(),
        interval_ms: hub.interval_ms.load(Ordering::Relaxed),
        history: hub.history.lock().unwrap().iter().cloned().collect(),
        settings: settings.0.lock().unwrap().clone(),
        monitors: app.get_webview_window("main").map(|w| monitor_list(&w)).unwrap_or_default(),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        settings_path: settings::path().display().to_string(),
    }
}

#[tauri::command]
fn save_settings(app: AppHandle, hub: State<Arc<Hub>>, state: State<SettingsState>, settings: Settings) -> Result<Settings, String> {
    let settings = settings.clamp();
    let previous = std::mem::replace(&mut *state.0.lock().unwrap(), settings.clone());
    settings.save().map_err(|e| e.to_string())?;
    hub.interval_ms.store(settings.interval_ms, Ordering::Relaxed);
    if previous.theme != settings.theme {
        apply_theme(&app, &settings.theme);
    }
    if previous.monitor != settings.monitor || previous.fullscreen != settings.fullscreen {
        place_window(&app, &settings);
    }
    Ok(settings)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let al = app.autolaunch();
    if enabled { al.enable() } else { al.disable() }.map_err(|e| e.to_string())?;
    let on = al.is_enabled().map_err(|e| e.to_string())?;
    if let Some(item) = app.try_state::<AutostartItem>() {
        let _ = item.0.set_checked(on);
    }
    Ok(on)
}

#[tauri::command]
fn toggle_fullscreen(window: WebviewWindow) {
    let full = window.is_fullscreen().unwrap_or(false);
    let _ = window.set_fullscreen(!full);
}

/// Things the dashboard can ask Windows to open. A fixed list, so the page can't launch arbitrary programs.
fn action_target(name: &str) -> Option<&str> {
    match name {
        "task-manager" => Some("taskmgr.exe"),
        "resource-monitor" => Some("resmon.exe"),
        _ => {
            let page = name.strip_prefix("ms-settings:")?;
            page.chars().all(|c| c.is_ascii_alphanumeric() || c == '-').then_some(name)
        }
    }
}

#[tauri::command]
fn run_action(name: String) -> Result<(), String> {
    let target = action_target(&name).ok_or_else(|| format!("unknown action: {name}"))?;
    let (op, file) = (ffi::wide("open"), ffi::wide(target));
    // ShellExecute rather than spawning: Task Manager asks for elevation, and ms-settings: is a URI.
    let r = unsafe { ffi::ShellExecuteW(0, op.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), ffi::SW_SHOWNORMAL) };
    if r > 32 {
        Ok(())
    } else {
        Err(format!("couldn't open {target} (error {r})"))
    }
}

/// For settings that are only read at startup (sensor sources).
#[tauri::command]
fn restart(app: AppHandle) {
    app.restart();
}

// --- window --------------------------------------------------------------------

/// Made in code rather than tauri.conf.json because the WebView2 flags depend on settings.
fn create_window(app: &AppHandle, s: &Settings) -> tauri::Result<WebviewWindow> {
    // Tauri's defaults, which setting our own flags would otherwise drop.
    let mut args = String::from("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection");
    if !s.gpu {
        args.push_str(" --disable-gpu");
    }
    WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
        .title("Oriel")
        .inner_size(1280.0, 800.0)
        .visible(false)
        .background_color(Color(0x0d, 0x0d, 0x0d, 255))
        .disable_drag_drop_handler() // or HTML drag and drop (the layout editor) doesn't work
        .additional_browser_args(&args)
        .build()
}

/// Monitors in physical pixels: primary first, then left to right, numbered from 1.
fn monitor_list(w: &WebviewWindow) -> Vec<MonitorRect> {
    let primary = w.primary_monitor().ok().flatten().map(|m| *m.position());
    let mut list: Vec<MonitorRect> = w
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| MonitorRect {
            index: 0,
            x: m.position().x,
            y: m.position().y,
            width: m.size().width,
            height: m.size().height,
            primary: Some(*m.position()) == primary,
        })
        .collect();
    list.sort_by_key(|m| (!m.primary, m.x, m.y));
    for (i, m) in list.iter_mut().enumerate() {
        m.index = i + 1;
    }
    list
}

fn place_window(app: &AppHandle, s: &Settings) {
    let Some(w) = app.get_webview_window("main") else { return };
    let mons = monitor_list(&w);
    let target = match s.monitor.as_str() {
        "auto" => mons.iter().find(|m| !m.primary).or(mons.first()),
        n => n.parse::<usize>().ok().and_then(|i| mons.iter().find(|m| m.index == i)).or(mons.first()),
    };
    let Some(m) = target.cloned() else {
        let _ = w.show();
        return;
    };
    // With one screen and "auto", don't cover the screen you're working on.
    let fullscreen = s.fullscreen && !(s.monitor == "auto" && mons.len() < 2);
    let _ = w.set_fullscreen(false);
    if fullscreen {
        let _ = w.set_position(PhysicalPosition::new(m.x, m.y));
        let _ = w.set_size(PhysicalSize::new(m.width, m.height));
        let _ = w.show();
        let _ = w.set_fullscreen(true);
    } else {
        let (ww, hh) = (m.width * 7 / 10, m.height * 3 / 4);
        let _ = w.set_position(PhysicalPosition::new(m.x + (m.width - ww) as i32 / 2, m.y + (m.height - hh) as i32 / 2));
        let _ = w.set_size(PhysicalSize::new(ww, hh));
        let _ = w.show();
    }
}

/// Title bar and the blank frame before the page paints follow the dashboard theme.
fn apply_theme(app: &AppHandle, theme: &str) {
    let Some(w) = app.get_webview_window("main") else { return };
    let (native, bg) = match theme {
        "dark" => (Some(Theme::Dark), Color(0x0d, 0x0d, 0x0d, 255)),
        "light" => (Some(Theme::Light), Color(0xf9, 0xf9, 0xf7, 255)),
        _ => (None, Color(0x0d, 0x0d, 0x0d, 255)),
    };
    let _ = w.set_theme(native);
    let _ = w.set_background_color(Some(bg));
}

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        // The page got no samples while hidden; have it reload history.
        let _ = app.emit_to("main", "resync", ());
    }
}

fn toggle(app: &AppHandle) {
    match app.get_webview_window("main") {
        Some(w) if w.is_visible().unwrap_or(false) => {
            let _ = w.hide();
        }
        _ => show(app),
    }
}

// --- tray ----------------------------------------------------------------------

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let autostart_on = app.autolaunch().is_enabled().unwrap_or(false);
    let toggle_i = MenuItem::with_id(app, "toggle", "Show / hide dashboard", true, None::<&str>)?;
    let settings_i = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let autostart_i = CheckMenuItem::with_id(app, "autostart", "Start at sign-in", true, autostart_on, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "Quit Oriel", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&toggle_i, &settings_i, &PredefinedMenuItem::separator(app)?, &autostart_i, &PredefinedMenuItem::separator(app)?, &quit_i],
    )?;
    app.manage(AutostartItem(autostart_i));

    TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
        .tooltip("Oriel")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "toggle" => toggle(app),
            "settings" => {
                show(app);
                let _ = app.emit_to("main", "open-settings", ());
            }
            "autostart" => {
                let on = !app.autolaunch().is_enabled().unwrap_or(false);
                if let Ok(on) = set_autostart(app.clone(), on) {
                    let _ = app.emit_to("main", "autostart", on);
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                toggle(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}
