//! Tide — desktop water reminder widget (Tauri 2 backend).
//!
//! All timing logic lives in [`engine`] (pure and unit-tested) and all OS
//! probing in [`platform`]; this module only wires them to Tauri.

pub mod engine;
pub mod platform;
pub mod stats;
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
use stats::Stats;
use store::{
    HistoryEntry, Layout, PersistedState, Position, Settings, Store, SCALE_MAX, SCALE_MIN,
};

pub const WIDGET_LABEL: &str = "widget";
pub const SETTINGS_LABEL: &str = "settings";
const SCREEN_MARGIN_PX: i32 = 24;
/// Logical size of the horizontal layout before `scale`; the vertical layout
/// is the same rectangle turned on its side (CONTRACT v0.3).
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
            self.state.streak,
        )
    }

    /// Reset today's count when the logical day (04:00 rollover) changed, and
    /// fold the day that just ended into the streak first. Returns true when
    /// anything changed.
    fn apply_day_rollover(&mut self, now: i64) -> bool {
        let key = day_key_local(now);
        if key == self.state.day_key {
            return false;
        }
        let (streak, best) = stats::rolled_over_streak(
            self.state.streak,
            self.state.best_streak,
            self.state.today_count,
            self.settings.daily_goal,
        );
        log::info!(
            "day rollover {} -> {key}: {} drinks, streak {} -> {streak} (best {best})",
            self.state.day_key,
            self.state.today_count,
            self.state.streak
        );
        self.state.streak = streak;
        self.state.best_streak = best;
        self.state.day_key = key;
        self.state.today_count = 0;
        true
    }

    fn compute_stats(&self, now: i64) -> Stats {
        stats::compute_stats(
            &self.store.load_history(),
            now,
            self.settings.interval_ms(),
            self.settings.daily_goal,
            local_offset_secs(),
        )
    }

    /// Mirror the in-memory timer into the persisted shape and write it out.
    fn persist_state(&mut self) {
        // Writing always upgrades the file to the current shape.
        self.state.version = store::STATE_VERSION;
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
        // Measured from the last *counted* drink so steady clicking still counts.
        if !should_merge_drink(now, self.state.last_counted_ts) {
            self.state.today_count = self.state.today_count.saturating_add(1);
            self.state.last_counted_ts = now;
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
        if settings.layout != previous.layout || settings.scale != previous.scale {
            apply_widget_size(&app, settings.layout, settings.scale);
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
        guard.state.last_counted_ts = 0; // the next drink must count
        // "Reset" also refills the bar: fresh timer, no snooze, no pending nudges.
        guard.timer.reset_session(now);
        guard.nudge = NudgeState::default();
        guard.persist_state();
        guard.log_history(&HistoryEntry::reset(now, "ui"));
        guard.tick(now)
    };
    emit_tick(&app, &tick);
    tick
}

#[tauri::command]
fn get_stats(state: SharedState<'_>) -> Stats {
    lock(&state).compute_stats(now_ms())
}

#[tauri::command]
fn open_data_dir(state: SharedState<'_>) -> Result<(), String> {
    let dir = lock(&state).store.dir().to_path_buf();
    // Best effort: a missing file manager must not fail the command loudly.
    let target = shell_path(&dir);
    let mut cmd = std::process::Command::new("explorer.exe");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.raw_arg(format!("\"{target}\""));
    }
    #[cfg(not(windows))]
    cmd.arg(&target);
    if let Err(err) = cmd.spawn() {
        log::warn!("could not open {target}: {err}");
    }
    Ok(())
}

/// Wipes history and the counters, keeps settings and the widget position.
/// The UI is responsible for confirming first.
#[tauri::command]
fn reset_all(app: AppHandle, state: SharedState<'_>) -> Tick {
    let now = now_ms();
    let tick = {
        let mut guard = lock(&state);
        if let Err(err) = guard.store.truncate_history() {
            log::error!("could not truncate history.jsonl: {err}");
        }
        // Appended right after the truncation so the file is never empty.
        guard.log_history(&HistoryEntry::reset(now, "ui"));

        guard.state.today_count = 0;
        guard.state.last_counted_ts = 0;
        guard.state.day_key = day_key_local(now);
        guard.state.streak = 0; // bestStreak is a record; it survives.
        guard.timer.reset_session(now);
        guard.nudge = NudgeState::default();
        guard.persist_state();
        log::info!("reset_all: history cleared, counters back to zero");
        guard.tick(now)
    };
    emit_tick(&app, &tick);
    tick
}

/// Path form that Explorer accepts.
/// Explorer refuses verbatim (`\\?\`) paths with "Location is not available",
/// and canonicalised paths on Windows carry that prefix; strip it.
fn shell_path(path: &std::path::Path) -> String {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.into_owned()
    }
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
    compute_tick_full(now, &TimerState::new(now), 45 * 60_000, 0, false, 0)
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
fn place_widget(app: &AppHandle, stored: Option<Position>, layout: Layout, ui_scale: f64) {
    let Some(window) = app.get_webview_window(WIDGET_LABEL) else {
        log::error!("widget window missing at setup");
        return;
    };

    // Frameless windows on Windows can come up taller than the configured
    // inner size; pin it to the layout size explicitly.
    let size = widget_size(layout, ui_scale);
    if let Err(err) = window.set_size(size) {
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
                    // The stored corner was chosen for a possibly smaller
                    // window; a taller layout must not hang off the screen.
                    clamp_widget_into_work_area(app);
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
        .unwrap_or_else(|_| size.to_physical(scale));
    let margin = (SCREEN_MARGIN_PX as f64 * scale).round() as i32;

    let x = area.position.x + area.size.width as i32 - widget.width as i32 - margin;
    let y = area.position.y + area.size.height as i32 - widget.height as i32 - margin;
    if let Err(err) = window.set_position(PhysicalPosition::new(x, y)) {
        log::warn!("could not place widget: {err}");
    }
    clamp_widget_into_work_area(app);
}

/// The window size a layout asks for, in logical pixels, with `ui_scale`
/// (Settings `scale`, 0.75..1.5) applied. Rust owns this: the UI only ever
/// paints inside whatever it gets.
pub fn widget_size(layout: Layout, ui_scale: f64) -> LogicalSize<f64> {
    let factor = if ui_scale.is_finite() {
        ui_scale.clamp(SCALE_MIN, SCALE_MAX)
    } else {
        1.0
    };
    let (w, h) = match layout {
        // `compact` keeps the full pill window; the UI shrinks what it draws.
        Layout::Horizontal | Layout::Compact => (WIDGET_W, WIDGET_H),
        Layout::Vertical => (WIDGET_H, WIDGET_W),
    };
    LogicalSize::new(w * factor, h * factor)
}

/// Resize the widget in place: the outer top-left corner is restored after the
/// resize so the bar does not wander when the layout or scale changes.
fn apply_widget_size(app: &AppHandle, layout: Layout, ui_scale: f64) {
    let Some(window) = app.get_webview_window(WIDGET_LABEL) else {
        return;
    };
    let origin = window.outer_position().ok();
    if let Err(err) = window.set_size(widget_size(layout, ui_scale)) {
        log::warn!("could not resize widget: {err}");
        return;
    }
    if let Some(origin) = origin {
        if let Err(err) = window.set_position(origin) {
            log::warn!("could not keep widget position after resize: {err}");
        }
    }
    clamp_widget_into_work_area(app);
}

/// Nudge `pos` so a `size` rectangle sits inside `area`, without resizing it.
/// Everything is physical pixels; `area` is `(x, y, width, height)`. A window
/// larger than the area is aligned to its top-left corner rather than pushed
/// off the opposite edge.
pub fn clamp_rect_into(
    area: (i32, i32, i32, i32),
    pos: (i32, i32),
    size: (i32, i32),
) -> (i32, i32) {
    let (ax, ay, aw, ah) = area;
    let (w, h) = size;
    let axis = |start: i32, extent: i32, want: i32, len: i32| {
        if len >= extent {
            start
        } else {
            want.clamp(start, start + extent - len)
        }
    };
    (axis(ax, aw, pos.0, w), axis(ay, ah, pos.1, h))
}

/// Keep the widget inside the work area of the monitor it sits on (the one
/// containing its top-left corner, else the primary). Called after every
/// resize, because a taller layout can push the bar under the taskbar or off
/// the bottom of the screen. The corrected position is persisted.
fn clamp_widget_into_work_area(app: &AppHandle) {
    let Some(window) = app.get_webview_window(WIDGET_LABEL) else {
        return;
    };
    let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) else {
        log::warn!("could not measure the widget; skipping the work-area clamp");
        return;
    };
    let size = (size.width as i32, size.height as i32);

    let monitor = window
        .available_monitors()
        .ok()
        .and_then(|monitors| {
            monitors.into_iter().find(|m| {
                let origin = m.position();
                let extent = m.size();
                pos.x >= origin.x
                    && pos.y >= origin.y
                    && pos.x < origin.x + extent.width as i32
                    && pos.y < origin.y + extent.height as i32
            })
        })
        .or_else(|| window.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        log::warn!("no monitor to clamp the widget into; leaving it where it is");
        return;
    };

    let work = monitor.work_area();
    let area = (
        work.position.x,
        work.position.y,
        work.size.width as i32,
        work.size.height as i32,
    );
    let (x, y) = clamp_rect_into(area, (pos.x, pos.y), size);
    if (x, y) == (pos.x, pos.y) {
        return;
    }
    log::info!(
        "widget at ({}, {}) size {:?} does not fit {area:?}; moving to ({x}, {y})",
        pos.x,
        pos.y,
        size
    );
    if let Err(err) = window.set_position(PhysicalPosition::new(x, y)) {
        log::warn!("could not clamp widget into the work area: {err}");
        return;
    }
    if let Some(state) = app.try_state::<Mutex<AppState>>() {
        let mut guard = lock(&state);
        guard.state.position = Some(Position { x, y });
        guard.persist_state();
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
            let (layout, ui_scale) = match app.try_state::<Mutex<AppState>>() {
                Some(state) => {
                    let guard = lock(&state);
                    (guard.settings.layout, guard.settings.scale)
                }
                None => return,
            };
            let over_grip = cursor_over_grip(&window, layout, ui_scale).unwrap_or(false);
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
/// compared in physical pixels; the grip is `GRIP_WIDTH_PX` logical px times
/// the Settings scale, times the window's DPI factor.
fn cursor_over_grip<R: Runtime>(
    window: &tauri::WebviewWindow<R>,
    layout: Layout,
    ui_scale: f64,
) -> Option<bool> {
    let cursor = platform::cursor_position()?;
    let origin = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    let dpi = window.scale_factor().unwrap_or(1.0);
    let grip = grip_px(ui_scale, dpi);
    Some(in_grip(
        layout,
        (origin.x, origin.y),
        (size.width as i32, size.height as i32),
        grip,
        cursor,
    ))
}

/// Grip thickness in physical pixels.
fn grip_px(ui_scale: f64, dpi: f64) -> i32 {
    let factor = if ui_scale.is_finite() {
        ui_scale.clamp(SCALE_MIN, SCALE_MAX)
    } else {
        1.0
    };
    (GRIP_WIDTH_PX * factor * dpi).round().max(1.0) as i32
}

/// The grip strip follows the layout: the rightmost `grip` px for the
/// horizontal and compact bars, the bottom `grip` px for the vertical one.
/// All arguments are physical pixels.
fn in_grip(
    layout: Layout,
    origin: (i32, i32),
    size: (i32, i32),
    grip: i32,
    cursor: (i32, i32),
) -> bool {
    let (x, y) = origin;
    let (w, h) = size;
    let (cx, cy) = cursor;
    if cx < x || cx >= x + w || cy < y || cy >= y + h {
        return false;
    }
    match layout {
        Layout::Horizontal | Layout::Compact => cx >= x + w - grip,
        Layout::Vertical => cy >= y + h - grip,
    }
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
    // When enabled, always re-register: the Run entry stores the executable
    // path, and it must follow the binary that is actually running (e.g. after
    // moving from a dev build to the installed release).
    if !enabled && !current {
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

    // A pre-v0.3 state.json has no streak data; recover it from history once.
    // The rebuild already accounts for every completed day, so the rollover
    // below must not fold today's stale count in on top of it.
    let rebuilt = state.needs_streak_rebuild();
    if rebuilt {
        let (streak, best) = stats::rebuild_streaks(
            &store.load_history(),
            now,
            settings.daily_goal,
            local_offset_secs(),
        );
        log::info!("upgrading state.json: rebuilt streak {streak}, best {best} from history");
        state.streak = streak;
        state.best_streak = best;
        let key = day_key_local(now);
        if key != state.day_key {
            state.day_key = key;
            state.today_count = 0;
        }
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
    if !rebuilt {
        // Normal path: the day may have rolled over while the app was closed.
        app_state.apply_day_rollover(now);
    }
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
            get_stats,
            open_data_dir,
            reset_all,
            quit
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let state = load_app_state(&handle);
            let stored_position = state.state.position;
            let settings = state.settings.clone();
            let paused = state.timer.paused_since.is_some();
            app.manage(Mutex::new(state));

            place_widget(&handle, stored_position, settings.layout, settings.scale);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_size_table_matches_the_contract() {
        let s = widget_size(Layout::Horizontal, 1.0);
        assert!((s.width - 220.0).abs() < 1e-9);
        assert!((s.height - 28.0).abs() < 1e-9);

        // Compact keeps the full pill window; only the painting differs.
        let s = widget_size(Layout::Compact, 1.0);
        assert!((s.width - 220.0).abs() < 1e-9);
        assert!((s.height - 28.0).abs() < 1e-9);

        // Vertical is the same rectangle on its side.
        let s = widget_size(Layout::Vertical, 1.0);
        assert!((s.width - 28.0).abs() < 1e-9);
        assert!((s.height - 220.0).abs() < 1e-9);
    }

    #[test]
    fn widget_size_applies_and_clamps_scale() {
        let s = widget_size(Layout::Horizontal, 1.5);
        assert!((s.width - 330.0).abs() < 1e-9);
        assert!((s.height - 42.0).abs() < 1e-9);

        let s = widget_size(Layout::Vertical, 0.75);
        assert!((s.width - 21.0).abs() < 1e-9);
        assert!((s.height - 165.0).abs() < 1e-9);

        // Out-of-range and non-finite scales cannot produce a silly window.
        assert_eq!(
            widget_size(Layout::Horizontal, 99.0).width,
            widget_size(Layout::Horizontal, SCALE_MAX).width
        );
        assert_eq!(
            widget_size(Layout::Horizontal, 0.0).width,
            widget_size(Layout::Horizontal, SCALE_MIN).width
        );
        assert_eq!(
            widget_size(Layout::Horizontal, f64::NAN).width,
            widget_size(Layout::Horizontal, 1.0).width
        );
    }

    #[test]
    fn grip_thickness_follows_scale_and_dpi() {
        assert_eq!(grip_px(1.0, 1.0), 20);
        assert_eq!(grip_px(1.5, 1.0), 30);
        assert_eq!(grip_px(1.0, 2.0), 40);
        assert_eq!(grip_px(1.25, 1.5), 38); // 20 * 1.25 * 1.5 = 37.5 -> 38
                                            // Never zero-width, whatever nonsense comes in.
        assert!(grip_px(f64::NAN, 1.0) > 0);
        assert!(grip_px(0.0, 0.0) > 0);
    }

    #[test]
    fn grip_rectangle_follows_the_layout() {
        let origin = (100, 200);
        let horizontal = (220, 28);
        let vertical = (28, 220);
        let grip = 20;

        // Horizontal and compact: the rightmost 20 px.
        for layout in [Layout::Horizontal, Layout::Compact] {
            assert!(in_grip(layout, origin, horizontal, grip, (300, 210)));
            assert!(in_grip(layout, origin, horizontal, grip, (319, 200)));
            assert!(!in_grip(layout, origin, horizontal, grip, (299, 210)));
            assert!(!in_grip(layout, origin, horizontal, grip, (110, 210)));
            // Just outside the window on either axis.
            assert!(!in_grip(layout, origin, horizontal, grip, (320, 210)));
            assert!(!in_grip(layout, origin, horizontal, grip, (300, 228)));
            assert!(!in_grip(layout, origin, horizontal, grip, (300, 199)));
        }

        // Vertical: the bottom 20 px instead.
        assert!(in_grip(
            Layout::Vertical,
            origin,
            vertical,
            grip,
            (110, 400)
        ));
        assert!(in_grip(
            Layout::Vertical,
            origin,
            vertical,
            grip,
            (110, 419)
        ));
        assert!(!in_grip(
            Layout::Vertical,
            origin,
            vertical,
            grip,
            (110, 399)
        ));
        assert!(!in_grip(
            Layout::Vertical,
            origin,
            vertical,
            grip,
            (110, 420)
        ));
        assert!(!in_grip(
            Layout::Vertical,
            origin,
            vertical,
            grip,
            (128, 410)
        ));
    }

    #[test]
    fn clamp_keeps_the_widget_inside_the_work_area() {
        // 1920x1080 with a 40 px taskbar at the bottom.
        let area = (0, 0, 1920, 1040);

        // Already inside: untouched.
        assert_eq!(clamp_rect_into(area, (100, 200), (220, 28)), (100, 200));
        // Flush against each far edge is still inside.
        assert_eq!(clamp_rect_into(area, (1700, 1012), (220, 28)), (1700, 1012));

        // The reported live case: vertical at scale 1.25 is 35x275, and a
        // top-left of (1861, 986) would end at y = 1261, well past the work
        // area; it gets pushed up (and left) just enough to fit.
        assert_eq!(clamp_rect_into(area, (1861, 986), (35, 275)), (1861, 765));
        assert_eq!(clamp_rect_into(area, (1900, 986), (35, 275)), (1885, 765));

        // Negative overshoot is clamped to the origin, no margin.
        assert_eq!(clamp_rect_into(area, (-50, -80), (220, 28)), (0, 0));

        // A window larger than the work area aligns to the top-left corner
        // instead of being pushed off the opposite edge.
        assert_eq!(clamp_rect_into(area, (500, 500), (2200, 1200)), (0, 0));
        assert_eq!(clamp_rect_into(area, (500, 500), (220, 1040)), (500, 0));

        // A secondary monitor's work area does not start at the origin.
        let right = (1920, 100, 1280, 900);
        assert_eq!(clamp_rect_into(right, (0, 0), (220, 28)), (1920, 100));
        // Past the right edge: pulled back to 1920 + 1280 - 35.
        assert_eq!(clamp_rect_into(right, (3300, 200), (35, 275)), (3165, 200));
        assert_eq!(clamp_rect_into(right, (3100, 950), (35, 275)), (3100, 725));
    }
}
