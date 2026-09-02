//! Tide — desktop water reminder widget (Tauri 2 backend).

pub mod engine;
pub mod store;

use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, State, WebviewUrl,
    WebviewWindowBuilder, WindowEvent,
};

use engine::{compute_tick, day_key_local, should_merge_drink, Tick};
use store::{HistoryEntry, PersistedState, Position, Settings, Store};

pub const WIDGET_LABEL: &str = "widget";
pub const SETTINGS_LABEL: &str = "settings";
const SCREEN_MARGIN_PX: i32 = 24;

/// Milliseconds since the Unix epoch. Clocks before 1970 are not a case we support.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub struct AppState {
    pub store: Store,
    pub settings: Settings,
    pub state: PersistedState,
}

impl AppState {
    fn tick(&self, now: i64) -> Tick {
        compute_tick(
            now,
            self.state.last_drink_ts,
            self.settings.interval_ms(),
            self.state.today_count,
        )
    }

    /// Reset today's count when the logical day (04:00 rollover) changed.
    /// Returns true when anything changed.
    fn apply_day_rollover(&mut self, now: i64) -> bool {
        let key = day_key_local(now);
        if key == self.state.day_key {
            return false;
        }
        log::info!("day rollover {} -> {key}", self.state.day_key);
        self.state.day_key = key;
        self.state.today_count = 0;
        true
    }

    fn persist_state(&self) {
        if let Err(err) = self.store.save_state(&self.state) {
            log::error!("could not save state.json: {err}");
        }
    }
}

type SharedState<'a> = State<'a, Mutex<AppState>>;

/// The state mutex is only held for short, panic-free critical sections; if a
/// panic did poison it we recover rather than take the whole app down.
fn lock<'a>(state: &'a SharedState<'_>) -> std::sync::MutexGuard<'a, AppState> {
    state.inner().lock().unwrap_or_else(|e| e.into_inner())
}

fn emit_tick(app: &AppHandle, tick: &Tick) {
    if let Err(err) = app.emit("tick", tick) {
        log::warn!("could not emit tick: {err}");
    }
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_tick(state: SharedState<'_>) -> Tick {
    lock(&state).tick(now_ms())
}

#[tauri::command]
fn get_settings(state: SharedState<'_>) -> Settings {
    log::debug!("get_settings invoked");
    lock(&state).settings
}

#[tauri::command]
fn set_settings(app: AppHandle, state: SharedState<'_>, settings: Settings) -> Settings {
    let (settings, tick) = {
        let mut guard = lock(&state);
        let settings = settings.clamped();
        guard.settings = settings;
        if let Err(err) = guard.store.save_settings(&settings) {
            log::error!("could not save settings.json: {err}");
        }
        (settings, guard.tick(now_ms()))
    };
    if let Err(err) = app.emit("settings-changed", settings) {
        log::warn!("could not emit settings-changed: {err}");
    }
    emit_tick(&app, &tick);
    settings
}

#[tauri::command]
fn drink(app: AppHandle, state: SharedState<'_>, source: String) -> Tick {
    let now = now_ms();
    let tick = {
        let mut guard = lock(&state);
        guard.apply_day_rollover(now);

        // Repeated clicks inside the merge window refill the bar but count once.
        if !should_merge_drink(now, guard.state.last_drink_ts) {
            guard.state.today_count = guard.state.today_count.saturating_add(1);
        }
        guard.state.last_drink_ts = now;
        guard.persist_state();

        if let Err(err) = guard.store.append_history(&HistoryEntry::drink(now, &source)) {
            log::error!("could not append history.jsonl: {err}");
        }
        guard.tick(now)
    };
    emit_tick(&app, &tick);
    tick
}

/// Async on purpose: creating a window from a synchronous command deadlocks
/// the webview on Windows (blank, unresponsive window).
#[tauri::command]
async fn open_settings(app: AppHandle) -> Result<(), String> {
    show_settings_window(&app).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_position(state: SharedState<'_>, x: i32, y: i32) {
    let mut guard = lock(&state);
    guard.state.position = Some(Position { x, y });
    guard.persist_state();
}

#[tauri::command]
fn quit(app: AppHandle, state: SharedState<'_>) {
    lock(&state).persist_state();
    app.exit(0);
}

// ---------------------------------------------------------------- windows

fn show_settings_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        window.show()?;
        window.unminimize().ok();
        window.set_focus()?;
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(
        app,
        SETTINGS_LABEL,
        WebviewUrl::App("settings.html".into()),
    )
    .title("Tide — Settings")
    .inner_size(380.0, 320.0)
    .resizable(false)
    .visible(true)
    .center()
    .build()?;

    // Hide instead of destroy so reopening is instant and state is preserved.
    // The window is destroyed on close and recreated lazily on the next
    // open_settings; hide-on-close proved unreliable on Windows.
    window.set_focus()?;
    Ok(())
}

/// Restore the stored position when it still lands on a connected monitor,
/// otherwise park the widget bottom-right on the primary monitor.
fn place_widget(app: &AppHandle, stored: Option<Position>) {
    let Some(window) = app.get_webview_window(WIDGET_LABEL) else {
        log::error!("widget window missing at setup");
        return;
    };

    // Frameless windows on Windows can come up taller than the configured
    // inner size; pin it to the contract size explicitly.
    if let Err(err) = window.set_size(LogicalSize::new(220.0, 28.0)) {
        log::warn!("could not set widget size: {err}");
    }

    if let Some(pos) = stored {
        match window.available_monitors() {
            Ok(monitors) => {
                let inside = monitors.iter().any(|m| {
                    let origin = m.position();
                    let size = m.size();
                    pos.x >= origin.x
                        && pos.y >= origin.y
                        && pos.x < origin.x + size.width as i32
                        && pos.y < origin.y + size.height as i32
                });
                if inside {
                    if let Err(err) = window.set_position(PhysicalPosition::new(pos.x, pos.y)) {
                        log::warn!("could not restore widget position: {err}");
                    }
                    return;
                }
                log::info!("stored position {pos:?} is off-screen; using default");
            }
            Err(err) => log::warn!("could not enumerate monitors: {err}"),
        }
    }

    // Bottom-right of the primary monitor with a margin.
    let primary = match window.primary_monitor() {
        Ok(Some(m)) => m,
        Ok(None) => {
            log::warn!("no primary monitor reported; leaving widget where it is");
            return;
        }
        Err(err) => {
            log::warn!("could not query primary monitor: {err}");
            return;
        }
    };
    let scale = primary.scale_factor();
    // Work area excludes the taskbar, so the widget never sits underneath it.
    let area = primary.work_area();
    let widget = window
        .outer_size()
        .unwrap_or_else(|_| LogicalSize::new(220.0, 28.0).to_physical(scale));
    let margin = (SCREEN_MARGIN_PX as f64 * scale).round() as i32;

    let x = area.position.x + area.size.width as i32 - widget.width as i32 - margin;
    let y = area.position.y + area.size.height as i32 - widget.height as i32 - margin;
    if let Err(err) = window.set_position(PhysicalPosition::new(x, y)) {
        log::warn!("could not place widget: {err}");
    }
}

fn focus_widget(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WIDGET_LABEL) {
        window.show().ok();
        window.set_focus().ok();
    }
}

// ---------------------------------------------------------------- tray

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let drink_item = MenuItem::with_id(app, "drink", "Drink now", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&drink_item, &settings_item, &separator, &quit_item])?;

    let mut builder = TrayIconBuilder::with_id("tide-tray")
        .tooltip("Tide")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "drink" => tray_drink(app),
            "settings" => {
                if let Err(err) = show_settings_window(app) {
                    log::error!("could not open settings: {err}");
                }
            }
            "quit" => {
                if let Some(state) = app.try_state::<Mutex<AppState>>() {
                    lock(&state).persist_state();
                }
                app.exit(0);
            }
            other => log::debug!("unhandled tray menu id {other}"),
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                tray_drink(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Same effect as the `drink` command, for tray-originated events.
fn tray_drink(app: &AppHandle) {
    let Some(state) = app.try_state::<Mutex<AppState>>() else {
        return;
    };
    let now = now_ms();
    let tick = {
        let mut guard = lock(&state);
        guard.apply_day_rollover(now);
        if !should_merge_drink(now, guard.state.last_drink_ts) {
            guard.state.today_count = guard.state.today_count.saturating_add(1);
        }
        guard.state.last_drink_ts = now;
        guard.persist_state();
        if let Err(err) = guard.store.append_history(&HistoryEntry::drink(now, "click")) {
            log::error!("could not append history.jsonl: {err}");
        }
        guard.tick(now)
    };
    emit_tick(app, &tick);
}

// ---------------------------------------------------------------- setup

fn load_app_state(app: &AppHandle) -> AppState {
    let dir = match app.path().app_config_dir() {
        Ok(dir) => dir,
        Err(err) => {
            log::error!("no app config dir ({err}); falling back to the temp dir");
            std::env::temp_dir().join("Tide")
        }
    };
    let store = Store::new(dir);
    let settings = store.load_settings();
    let now = now_ms();

    // First launch (or unreadable state): the bar starts full.
    let mut state = store
        .load_state()
        .unwrap_or_else(|| PersistedState::new(now, day_key_local(now)));

    let key = day_key_local(now);
    if key != state.day_key {
        state.day_key = key;
        state.today_count = 0;
    }

    let app_state = AppState {
        store,
        settings,
        state,
    };
    // Persist immediately so a first-launch timer survives a crash or restart.
    app_state.persist_state();
    app_state
}

/// 1 Hz tick loop. A plain OS thread keeps the async runtime out of the picture.
fn spawn_tick_loop(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        let Some(state) = app.try_state::<Mutex<AppState>>() else {
            return;
        };
        let now = now_ms();
        let tick = {
            let mut guard = lock(&state);
            if guard.apply_day_rollover(now) {
                guard.persist_state();
            }
            guard.tick(now)
        };
        emit_tick(&app, &tick);
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            focus_widget(app);
        }))
        .invoke_handler(tauri::generate_handler![
            drink,
            get_tick,
            get_settings,
            set_settings,
            open_settings,
            save_position,
            quit
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let state = load_app_state(&handle);
            let stored_position = state.state.position;
            app.manage(Mutex::new(state));

            place_widget(&handle, stored_position);
            build_tray(&handle)?;
            spawn_tick_loop(handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the widget quits; the settings window installs its own
            // hide-on-close handler when it is created.
            if let WindowEvent::CloseRequested { .. } = event {
                if window.label() == WIDGET_LABEL {
                    if let Some(state) = window.app_handle().try_state::<Mutex<AppState>>() {
                        lock(&state).persist_state();
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Tide");
}
