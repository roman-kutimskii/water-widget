//! Tide — desktop water reminder widget (Tauri 2 backend).
//!
//! All timing logic lives in [`engine`] (pure and unit-tested) and all OS
//! probing in [`platform`]; this module only wires them to Tauri.

pub mod engine;
pub mod platform;
pub mod store;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, Runtime, State, WebviewUrl,
    WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_notification::NotificationExt;

use engine::{
    away_outcome, compute_tick_full, day_key_local, is_active_at, is_quiet_at, local_offset_secs,
    minutes_of_day, should_merge_drink, AwayOutcome, Mode, Nudge, NudgeKind, NudgeState, Tick,
    TimerState, SLEEP_GAP_MS,
};
use store::{HistoryEntry, PersistedState, Position, Settings, Store};

pub const WIDGET_LABEL: &str = "widget";
pub const SETTINGS_LABEL: &str = "settings";
const SCREEN_MARGIN_PX: i32 = 24;
const WIDGET_W: f64 = 220.0;
const WIDGET_H: f64 = 28.0;
/// Width of the always-clickable grip at the right edge, in logical px.
const GRIP_WIDTH_PX: f64 = 20.0;
const CLICK_THROUGH_POLL_MS: u64 = 100;
const PLATFORM_POLL_MS: u64 = 2_000;

/// Updated by the platform poll thread, read by the 1 Hz tick loop.
static SESSION_LOCKED: AtomicBool = AtomicBool::new(false);
static NOTIFICATIONS_SUPPRESSED: AtomicBool = AtomicBool::new(false);
/// Bumped on every click-through change so a stale poll thread stops itself.
static CLICK_THROUGH_GEN: AtomicU64 = AtomicU64::new(0);

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
    pub timer: TimerState,
    pub nudge: NudgeState,
    /// When the session went away (locked or the machine slept).
    pub locked_at: Option<i64>,
    /// `now_ms` at the previous tick, for wall-clock gap (= system sleep) detection.
    pub last_tick_ms: i64,
}

impl AppState {
    fn quiet(&self, now: i64) -> bool {
        NOTIFICATIONS_SUPPRESSED.load(Ordering::Relaxed)
            || SESSION_LOCKED.load(Ordering::Relaxed)
            || is_quiet_at(
                minutes_of_day(now, local_offset_secs()),
                &self.settings.quiet_start,
                &self.settings.quiet_end,
            )
    }

    fn tick(&self, now: i64) -> Tick {
        compute_tick_full(
            now,
            &self.timer,
            self.settings.interval_ms(),
            self.state.today_count,
            self.quiet(now),
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

    /// Mirror the in-memory timer into the persisted shape and write it out.
    fn persist_state(&mut self) {
        self.state.last_drink_ts = self.timer.last_drink_ts;
        self.state.paused_accum_ms = self.timer.paused_accum_ms;
        self.state.paused_since = self.timer.paused_since;
        self.state.snooze_ms = self.timer.snooze_ms;
        self.state.last_mode = match self.timer.mode() {
            Mode::Active => "active",
            Mode::Paused => "paused",
            Mode::Sleeping => "sleeping",
        }
        .to_string();
        if let Err(err) = self.store.save_state(&self.state) {
            log::error!("could not save state.json: {err}");
        }
    }

    fn log_history(&self, entry: &HistoryEntry) {
        if let Err(err) = self.store.append_history(entry) {
            log::error!("could not append history.jsonl: {err}");
        }
    }

    /// Shared by the `drink` command, the tray and the hotkey.
    fn record_drink(&mut self, now: i64, source: &str) -> Tick {
        self.apply_day_rollover(now);
        // Repeated clicks inside the merge window refill the bar but count once.
        if !should_merge_drink(now, self.timer.last_drink_ts) {
            self.state.today_count = self.state.today_count.saturating_add(1);
        }
        self.timer.drink(now);
        self.nudge.reset();
        self.persist_state();
        self.log_history(&HistoryEntry::drink(now, source));
        self.tick(now)
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

fn emit_nudge(app: &AppHandle, kind: NudgeKind, overdue_ms: i64, toast_enabled: bool, quiet: bool) {
    if let Err(err) = app.emit("nudge", Nudge { kind, overdue_ms }) {
        log::warn!("could not emit nudge: {err}");
    }
    // `auto-resume` is the one kind that also speaks up during quiet hours.
    let quiet_ok = !quiet || kind == NudgeKind::AutoResume;
    if toast_enabled && quiet_ok {
        let body = match kind {
            NudgeKind::Overdue | NudgeKind::Repeat => "Time for water",
            NudgeKind::WelcomeBack => "Welcome back — drink now?",
            NudgeKind::AutoResume => "Timer resumed",
        };
        if let Err(err) = app.notification().builder().title("Tide").body(body).show() {
            log::warn!("could not show toast: {err}");
        }
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
    lock(&state).settings.clone()
}

#[tauri::command]
fn set_settings(app: AppHandle, state: SharedState<'_>, settings: Settings) -> Settings {
    let (settings, tick) = {
        let mut guard = lock(&state);
        let previous = guard.settings.clone();
        let mut settings = settings.clamped();

        // The OS is the final judge of a hotkey: registration may still fail,
        // in which case we fall back and report the setting we actually applied.
        if settings.hotkey_enabled != previous.hotkey_enabled || settings.hotkey != previous.hotkey
        {
            settings.hotkey = apply_hotkey(&app, &settings);
        }
        if settings.autostart != previous.autostart {
            apply_autostart(&app, settings.autostart);
        }
        if settings.always_on_top != previous.always_on_top {
            apply_always_on_top(&app, settings.always_on_top);
        }
        if settings.click_through != previous.click_through {
            apply_click_through(&app, settings.click_through);
        }

        guard.settings = settings.clone();
        if let Err(err) = guard.store.save_settings(&settings) {
            log::error!("could not save settings.json: {err}");
        }
        let now = now_ms();
        (settings, guard.tick(now))
    };
    if let Err(err) = app.emit("settings-changed", settings.clone()) {
        log::warn!("could not emit settings-changed: {err}");
    }
    emit_tick(&app, &tick);
    settings
}

#[tauri::command]
fn drink(app: AppHandle, state: SharedState<'_>, source: String) -> Tick {
    let tick = lock(&state).record_drink(now_ms(), &source);
    emit_tick(&app, &tick);
    tick
}

#[tauri::command]
fn snooze(app: AppHandle, state: SharedState<'_>, minutes: i64) -> Tick {
    let now = now_ms();
    let tick = {
        let mut guard = lock(&state);
        let applied = guard.timer.snooze(minutes);
        guard.nudge.reset();
        guard.persist_state();
        guard.log_history(&HistoryEntry::snooze(now, applied, "ui"));
        log::info!("snoozed {applied} min");
        guard.tick(now)
    };
    emit_tick(&app, &tick);
    tick
}

#[tauri::command]
fn set_paused(app: AppHandle, paused: bool) -> Tick {
    let tick = set_paused_inner(&app, paused, "ui");
    emit_tick(&app, &tick);
    tick
}

#[tauri::command]
fn reset_today(app: AppHandle, state: SharedState<'_>) -> Tick {
    let now = now_ms();
    let tick = {
        let mut guard = lock(&state);
        guard.state.today_count = 0;
        guard.persist_state();
        guard.log_history(&HistoryEntry::reset(now, "ui"));
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

// ------------------------------------------------------- shared transitions

/// Pause/resume from any source (UI, tray, auto-resume). Idempotent.
fn set_paused_inner(app: &AppHandle, paused: bool, source: &str) -> Tick {
    let now = now_ms();
    let Some(state) = app.try_state::<Mutex<AppState>>() else {
        return placeholder_tick(now);
    };
    let mut guard = lock(&state);
    if guard.timer.set_paused(paused, now) {
        if paused {
            guard.log_history(&HistoryEntry::pause(now, source));
        } else {
            guard.log_history(&HistoryEntry::resume(now, source));
            guard.nudge.reset();
        }
        guard.persist_state();
    }
    let tick = guard.tick(now);
    drop(guard);
    update_tray_pause_label(app, paused);
    tick
}

/// Only reachable if the managed state disappeared (shutdown); keeps the
/// command signatures total instead of unwrapping.
fn placeholder_tick(now: i64) -> Tick {
    compute_tick_full(now, &TimerState::new(now), 45 * 60_000, 0, false)
}

// ---------------------------------------------------------------- windows

fn show_settings_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        window.show()?;
        window.unminimize().ok();
        window.set_focus()?;
        return Ok(());
    }

    let window =
        WebviewWindowBuilder::new(app, SETTINGS_LABEL, WebviewUrl::App("settings.html".into()))
            .title("Tide — Settings")
            .inner_size(380.0, 560.0)
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
    if let Err(err) = window.set_size(LogicalSize::new(WIDGET_W, WIDGET_H)) {
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
        .unwrap_or_else(|_| LogicalSize::new(WIDGET_W, WIDGET_H).to_physical(scale));
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

fn apply_always_on_top(app: &AppHandle, on_top: bool) {
    if let Some(window) = app.get_webview_window(WIDGET_LABEL) {
        if let Err(err) = window.set_always_on_top(on_top) {
            log::warn!("could not set always-on-top: {err}");
        }
    }
}

// ----------------------------------------------------------- click-through

/// `clickThrough` makes the whole bar transparent to the mouse except a
/// [`GRIP_WIDTH_PX`] strip at its right edge; a 100 ms cursor poll flips
/// `ignore_cursor_events` as the cursor enters and leaves that strip.
fn apply_click_through(app: &AppHandle, enabled: bool) {
    // Any previously running poll thread belongs to an older generation and
    // stops on its next iteration, so the two never fight over the flag.
    let generation = CLICK_THROUGH_GEN.fetch_add(1, Ordering::SeqCst) + 1;

    let Some(window) = app.get_webview_window(WIDGET_LABEL) else {
        return;
    };
    if !enabled {
        if let Err(err) = window.set_ignore_cursor_events(false) {
            log::warn!("could not clear ignore-cursor-events: {err}");
        }
        return;
    }
    if let Err(err) = window.set_ignore_cursor_events(true) {
        log::warn!("could not set ignore-cursor-events: {err}");
        return;
    }

    let app = app.clone();
    std::thread::spawn(move || {
        let mut ignoring = true;
        while CLICK_THROUGH_GEN.load(Ordering::SeqCst) == generation {
            std::thread::sleep(Duration::from_millis(CLICK_THROUGH_POLL_MS));
            let Some(window) = app.get_webview_window(WIDGET_LABEL) else {
                return;
            };
            let over_grip = cursor_over_grip(&window).unwrap_or(false);
            if over_grip == ignoring {
                // Cursor on the grip -> stop ignoring; off it -> ignore again.
                if let Err(err) = window.set_ignore_cursor_events(!over_grip) {
                    log::warn!("could not toggle ignore-cursor-events: {err}");
                    continue;
                }
                ignoring = !over_grip;
            }
        }
        log::debug!("click-through poll generation {generation} stopped");
    });
}

/// True when the global cursor sits inside the grip rectangle. Everything is
/// compared in physical pixels; the grip width is scaled by the window's DPI.
fn cursor_over_grip<R: Runtime>(window: &tauri::WebviewWindow<R>) -> Option<bool> {
    let (cx, cy) = platform::cursor_position()?;
    let origin = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    let scale = window.scale_factor().unwrap_or(1.0);
    let grip = (GRIP_WIDTH_PX * scale).round() as i32;

    let right = origin.x + size.width as i32;
    let bottom = origin.y + size.height as i32;
    Some(cx >= right - grip && cx < right && cy >= origin.y && cy < bottom)
}

// -------------------------------------------------------- hotkey/autostart

/// Registers the configured hotkey and returns the accelerator that is actually
/// active (the default when the configured one is rejected by the OS, or the
/// configured string unchanged when hotkeys are disabled).
fn apply_hotkey(app: &AppHandle, settings: &Settings) -> String {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    let shortcuts = app.global_shortcut();
    if let Err(err) = shortcuts.unregister_all() {
        log::warn!("could not unregister previous hotkeys: {err}");
    }
    if !settings.hotkey_enabled {
        return settings.hotkey.clone();
    }

    for candidate in [settings.hotkey.as_str(), store::DEFAULT_HOTKEY] {
        match shortcuts.on_shortcut(candidate, |app, _shortcut, event| {
            use tauri_plugin_global_shortcut::ShortcutState;
            if event.state() == ShortcutState::Pressed {
                hotkey_drink(app);
            }
        }) {
            Ok(()) => {
                log::info!("hotkey registered: {candidate}");
                return candidate.to_string();
            }
            Err(err) => log::warn!("could not register hotkey {candidate}: {err}"),
        }
    }
    log::error!("no hotkey could be registered; hotkey disabled for this session");
    settings.hotkey.clone()
}

fn hotkey_drink(app: &AppHandle) {
    let Some(state) = app.try_state::<Mutex<AppState>>() else {
        return;
    };
    let tick = lock(&state).record_drink(now_ms(), "hotkey");
    emit_tick(app, &tick);
}

/// The setting is the source of truth; reconcile the OS registration with it.
fn apply_autostart(app: &AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;

    let manager = app.autolaunch();
    let current = manager.is_enabled().unwrap_or(false);
    if current == enabled {
        return;
    }
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    match result {
        Ok(()) => log::info!("autostart set to {enabled}"),
        Err(err) => log::error!("could not set autostart to {enabled}: {err}"),
    }
}

// ---------------------------------------------------------------- tray

/// Menu items we need to mutate later.
struct TrayItems {
    pause: MenuItem<tauri::Wry>,
}

fn update_tray_pause_label(app: &AppHandle, paused: bool) {
    if let Some(items) = app.try_state::<TrayItems>() {
        let label = if paused { "Resume" } else { "Pause" };
        if let Err(err) = items.pause.set_text(label) {
            log::warn!("could not update tray pause label: {err}");
        }
    }
}

fn build_tray(app: &AppHandle, paused: bool) -> tauri::Result<()> {
    let drink_item = MenuItem::with_id(app, "drink", "Drink now", true, None::<&str>)?;
    let snooze_item = MenuItem::with_id(app, "snooze", "Snooze 10 min", true, None::<&str>)?;
    let pause_item = MenuItem::with_id(
        app,
        "pause",
        if paused { "Resume" } else { "Pause" },
        true,
        None::<&str>,
    )?;
    let settings_item = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &drink_item,
            &snooze_item,
            &pause_item,
            &settings_item,
            &separator,
            &quit_item,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("tide-tray")
        .tooltip("Tide")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "drink" => tray_drink(app),
            "snooze" => tray_snooze(app),
            "pause" => tray_toggle_pause(app),
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
    app.manage(TrayItems { pause: pause_item });
    Ok(())
}

/// Same effect as the `drink` command, for tray-originated events.
fn tray_drink(app: &AppHandle) {
    let Some(state) = app.try_state::<Mutex<AppState>>() else {
        return;
    };
    let tick = lock(&state).record_drink(now_ms(), "click");
    emit_tick(app, &tick);
}

fn tray_snooze(app: &AppHandle) {
    let Some(state) = app.try_state::<Mutex<AppState>>() else {
        return;
    };
    let now = now_ms();
    let tick = {
        let mut guard = lock(&state);
        let applied = guard.timer.snooze(10);
        guard.nudge.reset();
        guard.persist_state();
        guard.log_history(&HistoryEntry::snooze(now, applied, "tray"));
        guard.tick(now)
    };
    emit_tick(app, &tick);
}

fn tray_toggle_pause(app: &AppHandle) {
    let Some(state) = app.try_state::<Mutex<AppState>>() else {
        return;
    };
    let paused = lock(&state).timer.paused_since.is_some();
    let tick = set_paused_inner(app, !paused, "tray");
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

    let timer = TimerState {
        last_drink_ts: state.last_drink_ts,
        paused_accum_ms: state.paused_accum_ms,
        paused_since: state.paused_since,
        sleeping_since: None,
        snooze_ms: state.snooze_ms,
    };

    let mut app_state = AppState {
        store,
        settings,
        state,
        timer,
        nudge: NudgeState::default(),
        locked_at: None,
        last_tick_ms: now,
    };
    // Persist immediately so a first-launch timer survives a crash or restart.
    app_state.persist_state();
    app_state
}

/// 1 Hz tick loop. A plain OS thread keeps the async runtime out of the picture.
/// It owns every automatic transition: system-sleep/lock returns, active-hours
/// sleep/wake, auto-resume, the day rollover and the nudge schedule.
fn spawn_tick_loop(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        let Some(state) = app.try_state::<Mutex<AppState>>() else {
            return;
        };
        let now = now_ms();

        let mut pending: Vec<NudgeKind> = Vec::new();
        let (tick, toast_enabled, pause_changed) = {
            let mut guard = lock(&state);
            let mut dirty = false;
            let was_paused = guard.timer.paused_since.is_some();

            // --- system sleep: a wall-clock gap much larger than the 1 s cadence.
            let gap = now - guard.last_tick_ms;
            if gap > SLEEP_GAP_MS && guard.locked_at.is_none() {
                log::info!("wall-clock gap of {gap} ms; treating as system sleep");
                guard.locked_at = Some(guard.last_tick_ms);
            }
            guard.last_tick_ms = now;

            // --- session lock / unlock.
            let locked = SESSION_LOCKED.load(Ordering::Relaxed);
            if locked {
                if guard.locked_at.is_none() {
                    guard.locked_at = Some(now);
                }
            } else if let Some(since) = guard.locked_at.take() {
                let away = (now - since).max(0);
                let interval = guard.settings.interval_ms();
                match away_outcome(away, interval) {
                    AwayOutcome::Continue => {}
                    AwayOutcome::WelcomeBack => {
                        guard.nudge.suppress_after_welcome_back(now);
                        pending.push(NudgeKind::WelcomeBack);
                    }
                    AwayOutcome::ResetSession => {
                        log::info!("away for {away} ms; starting a fresh session");
                        guard.timer.reset_session(now);
                        guard.nudge.reset();
                    }
                }
                dirty = true;
            }

            // --- active hours.
            let minute = minutes_of_day(now, local_offset_secs());
            let sleeping = !is_active_at(
                minute,
                &guard.settings.active_start,
                &guard.settings.active_end,
            );
            if guard.timer.set_sleeping(sleeping, now) {
                log::info!("active-hours transition: sleeping={sleeping}");
                guard.nudge.reset();
                dirty = true;
            }

            // --- auto-resume after a two-hour pause.
            if guard.timer.should_auto_resume(now) {
                guard.timer.set_paused(false, now);
                guard.log_history(&HistoryEntry::resume(now, "auto"));
                guard.nudge.reset();
                pending.push(NudgeKind::AutoResume);
                dirty = true;
            }

            if guard.apply_day_rollover(now) {
                dirty = true;
            }
            if dirty {
                guard.persist_state();
            }

            // --- nudge schedule.
            let tick = guard.tick(now);
            let overdue = tick.mode == Mode::Active && tick.fill <= 0.0;
            let (every, max) = (guard.settings.nudge_every_min, guard.settings.nudge_max);
            let quiet = tick.quiet;
            if let Some(kind) = guard.nudge.poll(now, overdue, every, max, quiet) {
                pending.push(kind);
            }

            let paused_changed = was_paused != guard.timer.paused_since.is_some();
            (tick, guard.settings.toast_enabled, paused_changed)
        };

        if pause_changed {
            update_tray_pause_label(&app, tick.paused_since.is_some());
        }
        for kind in pending {
            emit_nudge(&app, kind, tick.overdue_ms, toast_enabled, tick.quiet);
        }
        emit_tick(&app, &tick);
    });
}

/// 2 s poll of the Win32 bits: session lock and Focus Assist / full-screen.
fn spawn_platform_poll() {
    std::thread::spawn(move || loop {
        SESSION_LOCKED.store(platform::session_locked(), Ordering::Relaxed);
        NOTIFICATIONS_SUPPRESSED.store(platform::notifications_suppressed(), Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(PLATFORM_POLL_MS));
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            focus_widget(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            drink,
            snooze,
            set_paused,
            reset_today,
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
            let settings = state.settings.clone();
            let paused = state.timer.paused_since.is_some();
            app.manage(Mutex::new(state));

            place_widget(&handle, stored_position);
            apply_always_on_top(&handle, settings.always_on_top);
            apply_click_through(&handle, settings.click_through);
            apply_autostart(&handle, settings.autostart);

            // Reconcile the hotkey the OS actually accepted back into settings.
            let applied = apply_hotkey(&handle, &settings);
            if applied != settings.hotkey {
                if let Some(state) = handle.try_state::<Mutex<AppState>>() {
                    let mut guard = lock(&state);
                    guard.settings.hotkey = applied;
                    let settings = guard.settings.clone();
                    if let Err(err) = guard.store.save_settings(&settings) {
                        log::error!("could not save settings.json: {err}");
                    }
                }
            }

            build_tray(&handle, paused)?;
            spawn_platform_poll();
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
