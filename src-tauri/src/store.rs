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

const SETTINGS_FILE: &str = "settings.json";
const STATE_FILE: &str = "state.json";
const HISTORY_FILE: &str = "history.jsonl";

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            interval_min: default_interval_min(),
            opacity: default_opacity(),
            show_text: true,
            show_count: true,
        }
    }
}

impl Settings {
    /// Clamp every field into its contract range. NaN opacity falls back to the default.
    pub fn clamped(mut self) -> Self {
        self.version = SETTINGS_VERSION;
        self.interval_min = self.interval_min.clamp(INTERVAL_MIN_MIN, INTERVAL_MIN_MAX);
        self.opacity = if self.opacity.is_finite() {
            self.opacity.clamp(OPACITY_MIN, OPACITY_MAX)
        } else {
            default_opacity()
        };
        self
    }

    pub fn interval_ms(&self) -> i64 {
        i64::from(self.interval_min) * 60_000
    }
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
}

fn default_state_version() -> u32 {
    STATE_VERSION
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
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub ts: i64,
    #[serde(rename = "type")]
    pub kind: String,
    pub source: String,
}

impl HistoryEntry {
    pub fn drink(ts: i64, source: &str) -> Self {
        Self {
            ts,
            kind: "drink".to_string(),
            source: source.to_string(),
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
    }

    #[test]
    fn clamping() {
        let s = Settings {
            version: 99,
            interval_min: 1,
            opacity: 5.0,
            show_text: false,
            show_count: false,
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
            ..Settings::default()
        }
        .clamped();
        assert!((s.opacity - 0.9).abs() < 1e-9);
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
            ..Settings::default()
        };
        store.save_settings(&settings).expect("save settings");
        assert_eq!(store.load_settings(), settings);

        let state = PersistedState {
            position: Some(Position { x: 12, y: 34 }),
            ..PersistedState::new(1_700_000_000_000, "2026-09-02".into())
        };
        store.save_state(&state).expect("save state");
        assert_eq!(store.load_state().expect("state"), state);

        // Overwriting an existing file must work (atomic rename path).
        store.save_state(&state).expect("save state again");

        store
            .append_history(&HistoryEntry::drink(1, "click"))
            .expect("history");
        store
            .append_history(&HistoryEntry::drink(2, "hotkey"))
            .expect("history");
        let raw = fs::read_to_string(store.history_path()).expect("read history");
        assert_eq!(raw.lines().count(), 2);
        assert!(raw.contains("\"type\":\"drink\""));
        assert!(raw.contains("\"source\":\"hotkey\""));

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
}
