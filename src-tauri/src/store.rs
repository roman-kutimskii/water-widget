//! Persistence: settings.json, state.json, history.jsonl under the app config dir.
//!
//! Every load is total: missing or corrupt files fall back to defaults instead
//! of failing. Writes are atomic (temp file + rename).

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const SETTINGS_VERSION: u32 = 1;
pub const STATE_VERSION: u32 = 1;

pub const INTERVAL_MIN_MIN: u32 = 10;
pub const INTERVAL_MIN_MAX: u32 = 180;
pub const OPACITY_MIN: f64 = 0.3;
pub const OPACITY_MAX: f64 = 1.0;
pub const DAILY_GOAL_MIN: u32 = 1;
pub const DAILY_GOAL_MAX: u32 = 30;
pub const NUDGE_EVERY_MIN_MIN: u32 = 1;
pub const NUDGE_EVERY_MIN_MAX: u32 = 60;
pub const NUDGE_MAX_MAX: u32 = 10;
pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+W";

const SETTINGS_FILE: &str = "settings.json";
const STATE_FILE: &str = "state.json";
const HISTORY_FILE: &str = "history.jsonl";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default = "default_interval_min")]
    pub interval_min: u32,
    #[serde(default = "default_opacity")]
    pub opacity: f64,
    #[serde(default = "default_true")]
    pub show_text: bool,
    #[serde(default = "default_true")]
    pub show_count: bool,

    // --- Timing (v0.2)
    #[serde(default = "default_active_start")]
    pub active_start: String,
    #[serde(default = "default_active_end")]
    pub active_end: String,
    #[serde(default = "default_quiet_start")]
    pub quiet_start: String,
    #[serde(default = "default_quiet_end")]
    pub quiet_end: String,
    #[serde(default = "default_daily_goal")]
    pub daily_goal: u32,

    // --- Behavior (v0.2)
    #[serde(default = "default_true")]
    pub always_on_top: bool,
    #[serde(default)]
    pub click_through: bool,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default = "default_true")]
    pub hotkey_enabled: bool,
    #[serde(default = "default_hotkey")]
    pub hotkey: String,

    // --- Alerts (v0.2)
    #[serde(default = "default_true")]
    pub toast_enabled: bool,
    #[serde(default = "default_nudge_every_min")]
    pub nudge_every_min: u32,
    #[serde(default = "default_nudge_max")]
    pub nudge_max: u32,
    #[serde(default)]
    pub sound_enabled: bool,
    #[serde(default = "default_sound_volume")]
    pub sound_volume: f64,
}

fn default_version() -> u32 {
    SETTINGS_VERSION
}
fn default_interval_min() -> u32 {
    45
}
fn default_opacity() -> f64 {
    0.9
}
fn default_true() -> bool {
    true
}
fn default_active_start() -> String {
    "08:00".to_string()
}
fn default_active_end() -> String {
    "22:00".to_string()
}
fn default_quiet_start() -> String {
    "22:00".to_string()
}
fn default_quiet_end() -> String {
    "08:00".to_string()
}
fn default_daily_goal() -> u32 {
    8
}
fn default_hotkey() -> String {
    DEFAULT_HOTKEY.to_string()
}
fn default_nudge_every_min() -> u32 {
    10
}
fn default_nudge_max() -> u32 {
    3
}
fn default_sound_volume() -> f64 {
    0.5
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            interval_min: default_interval_min(),
            opacity: default_opacity(),
            show_text: true,
            show_count: true,
            active_start: default_active_start(),
            active_end: default_active_end(),
            quiet_start: default_quiet_start(),
            quiet_end: default_quiet_end(),
            daily_goal: default_daily_goal(),
            always_on_top: true,
            click_through: false,
            autostart: false,
            hotkey_enabled: true,
            hotkey: default_hotkey(),
            toast_enabled: true,
            nudge_every_min: default_nudge_every_min(),
            nudge_max: default_nudge_max(),
            sound_enabled: false,
            sound_volume: default_sound_volume(),
        }
    }
}

impl Settings {
    /// Clamp every numeric field into its contract range, validate the "HH:MM"
    /// strings and the hotkey; anything invalid falls back to its default (NaN
    /// floats included). This is the single validation entry point, and
    /// `set_settings` returns the result so the UI sees what was applied.
    pub fn clamped(mut self) -> Self {
        self.version = SETTINGS_VERSION;
        self.interval_min = self.interval_min.clamp(INTERVAL_MIN_MIN, INTERVAL_MIN_MAX);
        self.opacity = clamp_f64(self.opacity, OPACITY_MIN, OPACITY_MAX, default_opacity());

        self.active_start = valid_hhmm(self.active_start, default_active_start);
        self.active_end = valid_hhmm(self.active_end, default_active_end);
        self.quiet_start = valid_hhmm(self.quiet_start, default_quiet_start);
        self.quiet_end = valid_hhmm(self.quiet_end, default_quiet_end);
        self.daily_goal = self.daily_goal.clamp(DAILY_GOAL_MIN, DAILY_GOAL_MAX);

        if !is_valid_hotkey(&self.hotkey) {
            log::warn!(
                "invalid hotkey {:?}; falling back to {DEFAULT_HOTKEY}",
                self.hotkey
            );
            self.hotkey = default_hotkey();
        }

        self.nudge_every_min = self
            .nudge_every_min
            .clamp(NUDGE_EVERY_MIN_MIN, NUDGE_EVERY_MIN_MAX);
        self.nudge_max = self.nudge_max.min(NUDGE_MAX_MAX);
        self.sound_volume = clamp_f64(self.sound_volume, 0.0, 1.0, default_sound_volume());
        self
    }

    pub fn interval_ms(&self) -> i64 {
        i64::from(self.interval_min) * 60_000
    }
}

fn clamp_f64(value: f64, min: f64, max: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

fn valid_hhmm(value: String, fallback: fn() -> String) -> String {
    if crate::engine::parse_hhmm(&value).is_some() {
        value
    } else {
        log::warn!("invalid time {value:?}; falling back to the default");
        fallback()
    }
}

/// Shape check for a `tauri-plugin-global-shortcut` accelerator: at least one
/// modifier plus a key, joined by `+`, no empty segments and no whitespace.
/// Whether the OS accepts it is only known at registration time; that failure
/// also falls back to the default.
pub fn is_valid_hotkey(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 {
        return false;
    }
    let parts: Vec<&str> = value.split('+').collect();
    parts.len() >= 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedState {
    #[serde(default = "default_state_version")]
    pub version: u32,
    pub last_drink_ts: i64,
    #[serde(default)]
    pub today_count: u32,
    #[serde(default)]
    pub day_key: String,
    #[serde(default)]
    pub position: Option<Position>,
    // --- v0.2; all default so MVP state.json files still load.
    #[serde(default)]
    pub paused_accum_ms: i64,
    #[serde(default)]
    pub paused_since: Option<i64>,
    #[serde(default)]
    pub snooze_ms: i64,
    #[serde(default = "default_last_mode")]
    pub last_mode: String,
}

fn default_state_version() -> u32 {
    STATE_VERSION
}

fn default_last_mode() -> String {
    "active".to_string()
}

impl PersistedState {
    /// First-launch state: the bar starts full.
    pub fn new(now_ms: i64, day_key: String) -> Self {
        Self {
            version: STATE_VERSION,
            last_drink_ts: now_ms,
            today_count: 0,
            day_key,
            position: None,
            paused_accum_ms: 0,
            paused_since: None,
            snooze_ms: 0,
            last_mode: default_last_mode(),
        }
    }
}

/// One line of `history.jsonl`. `type` is one of
/// `drink | snooze | pause | resume | reset`; `minutes` is only present on snooze.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub ts: i64,
    #[serde(rename = "type")]
    pub kind: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes: Option<i64>,
}

impl HistoryEntry {
    pub fn drink(ts: i64, source: &str) -> Self {
        Self::new(ts, "drink", source)
    }

    pub fn snooze(ts: i64, minutes: i64, source: &str) -> Self {
        Self {
            minutes: Some(minutes),
            ..Self::new(ts, "snooze", source)
        }
    }

    pub fn pause(ts: i64, source: &str) -> Self {
        Self::new(ts, "pause", source)
    }

    pub fn resume(ts: i64, source: &str) -> Self {
        Self::new(ts, "resume", source)
    }

    pub fn reset(ts: i64, source: &str) -> Self {
        Self::new(ts, "reset", source)
    }

    fn new(ts: i64, kind: &str, source: &str) -> Self {
        Self {
            ts,
            kind: kind.to_string(),
            source: source.to_string(),
            minutes: None,
        }
    }
}

/// Handle to the on-disk data directory.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// Creates the directory if it does not exist. A failure here is logged and
    /// tolerated: the app still runs, it just cannot persist.
    pub fn new(dir: PathBuf) -> Self {
        if let Err(err) = fs::create_dir_all(&dir) {
            log::error!("could not create data dir {}: {err}", dir.display());
        }
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn settings_path(&self) -> PathBuf {
        self.dir.join(SETTINGS_FILE)
    }

    pub fn state_path(&self) -> PathBuf {
        self.dir.join(STATE_FILE)
    }

    pub fn history_path(&self) -> PathBuf {
        self.dir.join(HISTORY_FILE)
    }

    /// Never fails: missing or corrupt settings yield the defaults.
    pub fn load_settings(&self) -> Settings {
        match read_json::<Settings>(&self.settings_path()) {
            Ok(Some(s)) => s.clamped(),
            Ok(None) => Settings::default(),
            Err(err) => {
                log::warn!("settings.json unreadable ({err}); using defaults");
                Settings::default()
            }
        }
    }

    pub fn save_settings(&self, settings: &Settings) -> std::io::Result<()> {
        write_json_atomic(&self.settings_path(), settings)
    }

    /// `None` when there is no usable state on disk (first launch or corrupt file).
    pub fn load_state(&self) -> Option<PersistedState> {
        match read_json::<PersistedState>(&self.state_path()) {
            Ok(state) => state,
            Err(err) => {
                log::warn!("state.json unreadable ({err}); starting fresh");
                None
            }
        }
    }

    pub fn save_state(&self, state: &PersistedState) -> std::io::Result<()> {
        write_json_atomic(&self.state_path(), state)
    }

    pub fn append_history(&self, entry: &HistoryEntry) -> std::io::Result<()> {
        let line = serde_json::to_string(entry)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.history_path())?;
        writeln!(file, "{line}")?;
        file.flush()
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> std::io::Result<Option<T>> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    match serde_json::from_str::<T>(&raw) {
        Ok(value) => Ok(Some(value)),
        Err(err) => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, err)),
    }
}

/// Write to `<path>.tmp` and rename over the target so a crash mid-write cannot
/// leave a truncated file behind.
fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let file = File::create(&tmp)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, value)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
    }
    // Windows rename fails if the destination exists, so remove it first.
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("tide-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Store::new(dir)
    }

    #[test]
    fn defaults_match_contract() {
        let s = Settings::default();
        assert_eq!(s.interval_min, 45);
        assert!((s.opacity - 0.9).abs() < 1e-9);
        assert!(s.show_text);
        assert!(s.show_count);
        assert_eq!(s.version, 1);
        assert_eq!(s.interval_ms(), 45 * 60_000);

        assert_eq!(s.active_start, "08:00");
        assert_eq!(s.active_end, "22:00");
        assert_eq!(s.quiet_start, "22:00");
        assert_eq!(s.quiet_end, "08:00");
        assert_eq!(s.daily_goal, 8);
        assert!(s.always_on_top);
        assert!(!s.click_through);
        assert!(!s.autostart);
        assert!(s.hotkey_enabled);
        assert_eq!(s.hotkey, "Ctrl+Alt+W");
        assert!(s.toast_enabled);
        assert_eq!(s.nudge_every_min, 10);
        assert_eq!(s.nudge_max, 3);
        assert!(!s.sound_enabled);
        assert!((s.sound_volume - 0.5).abs() < 1e-9);
    }

    #[test]
    fn clamping() {
        let s = Settings {
            version: 99,
            interval_min: 1,
            opacity: 5.0,
            show_text: false,
            show_count: false,
            ..Settings::default()
        }
        .clamped();
        assert_eq!(s.interval_min, 10);
        assert!((s.opacity - 1.0).abs() < 1e-9);
        assert_eq!(s.version, SETTINGS_VERSION);

        let s = Settings {
            interval_min: 100_000,
            opacity: -1.0,
            ..Settings::default()
        }
        .clamped();
        assert_eq!(s.interval_min, 180);
        assert!((s.opacity - 0.3).abs() < 1e-9);

        let s = Settings {
            opacity: f64::NAN,
            sound_volume: f64::NAN,
            ..Settings::default()
        }
        .clamped();
        assert!((s.opacity - 0.9).abs() < 1e-9);
        assert!((s.sound_volume - 0.5).abs() < 1e-9);
    }

    #[test]
    fn clamping_v02_fields() {
        let s = Settings {
            daily_goal: 0,
            nudge_every_min: 0,
            nudge_max: 99,
            sound_volume: 7.5,
            ..Settings::default()
        }
        .clamped();
        assert_eq!(s.daily_goal, 1);
        assert_eq!(s.nudge_every_min, 1);
        assert_eq!(s.nudge_max, 10);
        assert!((s.sound_volume - 1.0).abs() < 1e-9);

        let s = Settings {
            daily_goal: 1000,
            nudge_every_min: 1000,
            sound_volume: -3.0,
            ..Settings::default()
        }
        .clamped();
        assert_eq!(s.daily_goal, 30);
        assert_eq!(s.nudge_every_min, 60);
        assert!(s.sound_volume.abs() < 1e-9);
    }

    #[test]
    fn bad_times_and_hotkeys_fall_back_to_defaults() {
        let s = Settings {
            active_start: "8:00".into(),
            active_end: "25:00".into(),
            quiet_start: "".into(),
            quiet_end: "12:75".into(),
            hotkey: "Ctrl+".into(),
            ..Settings::default()
        }
        .clamped();
        assert_eq!(s.active_start, "08:00");
        assert_eq!(s.active_end, "22:00");
        assert_eq!(s.quiet_start, "22:00");
        assert_eq!(s.quiet_end, "08:00");
        assert_eq!(s.hotkey, DEFAULT_HOTKEY);

        // Valid values survive untouched, midnight-crossing windows included.
        let s = Settings {
            active_start: "22:00".into(),
            active_end: "06:00".into(),
            hotkey: "Shift+Alt+F5".into(),
            ..Settings::default()
        }
        .clamped();
        assert_eq!(s.active_start, "22:00");
        assert_eq!(s.active_end, "06:00");
        assert_eq!(s.hotkey, "Shift+Alt+F5");
    }

    #[test]
    fn hotkey_shape_validation() {
        assert!(is_valid_hotkey("Ctrl+Alt+W"));
        assert!(is_valid_hotkey("CommandOrControl+Shift+K"));
        assert!(!is_valid_hotkey(""));
        assert!(!is_valid_hotkey("W"));
        assert!(!is_valid_hotkey("Ctrl+"));
        assert!(!is_valid_hotkey("+W"));
        assert!(!is_valid_hotkey("Ctrl + W"));
        assert!(!is_valid_hotkey("Ctrl++W"));
    }

    #[test]
    fn missing_files_fall_back_to_defaults() {
        let store = temp_store("missing");
        assert_eq!(store.load_settings(), Settings::default());
        assert!(store.load_state().is_none());
        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn corrupt_files_fall_back_to_defaults() {
        let store = temp_store("corrupt");
        fs::write(store.settings_path(), b"{ not json").expect("write");
        fs::write(store.state_path(), b"\0\0\0").expect("write");
        assert_eq!(store.load_settings(), Settings::default());
        assert!(store.load_state().is_none());
        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn round_trip_and_history() {
        let store = temp_store("roundtrip");

        let settings = Settings {
            interval_min: 30,
            opacity: 0.5,
            show_text: false,
            click_through: true,
            hotkey: "Ctrl+Alt+K".into(),
            ..Settings::default()
        };
        store.save_settings(&settings).expect("save settings");
        assert_eq!(store.load_settings(), settings);

        let state = PersistedState {
            position: Some(Position { x: 12, y: 34 }),
            snooze_ms: 600_000,
            paused_accum_ms: 1_000,
            paused_since: Some(1_700_000_100_000),
            last_mode: "paused".into(),
            ..PersistedState::new(1_700_000_000_000, "2026-09-02".into())
        };
        store.save_state(&state).expect("save state");
        assert_eq!(store.load_state().expect("state"), state);

        // Overwriting an existing file must work (atomic rename path).
        store.save_state(&state).expect("save state again");

        for entry in [
            HistoryEntry::drink(1, "click"),
            HistoryEntry::drink(2, "hotkey"),
            HistoryEntry::snooze(3, 10, "tray"),
            HistoryEntry::pause(4, "tray"),
            HistoryEntry::resume(5, "auto"),
            HistoryEntry::reset(6, "settings"),
        ] {
            store.append_history(&entry).expect("history");
        }
        let raw = fs::read_to_string(store.history_path()).expect("read history");
        assert_eq!(raw.lines().count(), 6);
        assert!(raw.contains("\"type\":\"drink\""));
        assert!(raw.contains("\"source\":\"hotkey\""));
        assert!(raw.contains("\"type\":\"snooze\",\"source\":\"tray\",\"minutes\":10"));
        assert!(raw.contains("\"type\":\"pause\""));
        assert!(raw.contains("\"type\":\"resume\""));
        assert!(raw.contains("\"type\":\"reset\""));
        // `minutes` is omitted on non-snooze entries.
        assert_eq!(raw.matches("minutes").count(), 1);

        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn partial_settings_json_uses_defaults_for_missing_fields() {
        let store = temp_store("partial");
        fs::write(store.settings_path(), br#"{"intervalMin": 60}"#).expect("write");
        let s = store.load_settings();
        assert_eq!(s.interval_min, 60);
        assert!((s.opacity - 0.9).abs() < 1e-9);
        assert!(s.show_text);
        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn mvp_files_without_v02_fields_still_load() {
        let store = temp_store("mvp-files");
        fs::write(
            store.settings_path(),
            br#"{"version":1,"intervalMin":60,"opacity":0.8,"showText":true,"showCount":false}"#,
        )
        .expect("write");
        let s = store.load_settings();
        assert_eq!(s.interval_min, 60);
        assert!(!s.show_count);
        assert_eq!(s.active_start, "08:00");
        assert_eq!(s.hotkey, DEFAULT_HOTKEY);
        assert_eq!(s.nudge_max, 3);

        fs::write(
            store.state_path(),
            br#"{"version":1,"lastDrinkTs":1700000000000,"todayCount":4,"dayKey":"2026-09-01","position":{"x":1,"y":2}}"#,
        )
        .expect("write");
        let st = store.load_state().expect("state");
        assert_eq!(st.today_count, 4);
        assert_eq!(st.paused_accum_ms, 0);
        assert_eq!(st.paused_since, None);
        assert_eq!(st.snooze_ms, 0);
        assert_eq!(st.last_mode, "active");

        let _ = fs::remove_dir_all(store.dir());
    }
}
