//! Streak bookkeeping and the 14-day history summary. Pure functions over a
//! slice of history entries — no IO, no Tauri.

use std::collections::BTreeMap;

use chrono::{Duration, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::engine::{day_key, MINUTE_MS};
use crate::store::HistoryEntry;

/// How many days `compute_stats` reports, today included.
pub const STATS_DAYS: usize = 14;

/// One entry of `Stats::days`. Field names must match CONTRACT.md exactly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayStat {
    pub day_key: String,
    pub drinks: u32,
    /// Mean minutes between consecutive drinks that day; `None` with < 2 drinks.
    pub avg_gap_min: Option<f64>,
    /// `max(0, gap − interval)` over the day's gaps, in minutes.
    pub longest_overdue_min: f64,
    pub goal_met: bool,
}

/// Payload of `get_stats`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub days: Vec<DayStat>,
    pub streak: u32,
    pub best_streak: u32,
    pub total_drinks: u32,
}

/// Day rollover result: what the streak becomes once a day ends with
/// `today_count` drinks logged. Returns `(streak, best_streak)`.
pub fn rolled_over_streak(
    streak: u32,
    best_streak: u32,
    today_count: u32,
    daily_goal: u32,
) -> (u32, u32) {
    let streak = if today_count >= daily_goal.max(1) {
        streak.saturating_add(1)
    } else {
        0
    };
    (streak, best_streak.max(streak))
}

/// Drinks per logical day (04:00 rollover), oldest first.
fn drinks_per_day(entries: &[HistoryEntry], local_offset_secs: i32) -> BTreeMap<String, Vec<i64>> {
    let mut per_day: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for entry in entries.iter().filter(|e| e.kind == "drink") {
        per_day
            .entry(day_key(entry.ts, local_offset_secs))
            .or_default()
            .push(entry.ts);
    }
    for timestamps in per_day.values_mut() {
        timestamps.sort_unstable();
    }
    per_day
}

/// `YYYY-MM-DD` shifted by `days` (negative = into the past). Falls back to the
/// input when it is not a parseable date.
fn shift_day_key(key: &str, days: i64) -> String {
    match NaiveDate::parse_from_str(key, "%Y-%m-%d") {
        Ok(date) => (date + Duration::days(days)).format("%Y-%m-%d").to_string(),
        Err(_) => key.to_string(),
    }
}

/// Rebuild `(streak, bestStreak)` from history, used once when upgrading a
/// pre-v0.3 `state.json`. Today is excluded: it has not rolled over yet.
pub fn rebuild_streaks(
    entries: &[HistoryEntry],
    now_ms: i64,
    daily_goal: u32,
    local_offset_secs: i32,
) -> (u32, u32) {
    let per_day = drinks_per_day(entries, local_offset_secs);
    let Some(first_key) = per_day.keys().next().cloned() else {
        return (0, 0);
    };
    let today = day_key(now_ms, local_offset_secs);

    let goal = daily_goal.max(1);
    let (mut streak, mut best) = (0u32, 0u32);
    // Walk every calendar day from the first recorded drink to yesterday, so a
    // day with no drinks at all breaks the streak just like a rollover would.
    let mut key = first_key;
    while key < today {
        let drinks = per_day.get(&key).map_or(0, |d| d.len() as u32);
        let (next_streak, next_best) = rolled_over_streak(streak, best, drinks, goal);
        streak = next_streak;
        best = next_best;
        key = shift_day_key(&key, 1);
    }
    (streak, best)
}

/// The last [`STATS_DAYS`] days ending today, plus the streak counters.
///
/// `streak` / `best_streak` are recomputed from history here as well, so the
/// stats view is consistent even if `state.json` drifted.
pub fn compute_stats(
    entries: &[HistoryEntry],
    now_ms: i64,
    interval_ms: i64,
    daily_goal: u32,
    local_offset_secs: i32,
) -> Stats {
    let per_day = drinks_per_day(entries, local_offset_secs);
    let goal = daily_goal.max(1);
    let interval_min = interval_ms.max(1) as f64 / MINUTE_MS as f64;

    let today = day_key(now_ms, local_offset_secs);
    let mut days = Vec::with_capacity(STATS_DAYS);
    for back in (0..STATS_DAYS as i64).rev() {
        let key = shift_day_key(&today, -back);
        let timestamps = per_day.get(&key).map(Vec::as_slice).unwrap_or(&[]);
        days.push(day_stat(key, timestamps, interval_min, goal));
    }

    let (streak, best_streak) = rebuild_streaks(entries, now_ms, goal, local_offset_secs);
    let total_drinks = per_day.values().map(|d| d.len() as u32).sum();

    Stats {
        days,
        streak,
        best_streak,
        total_drinks,
    }
}

/// Gaps are only taken *within* a day, so the long overnight span between the
/// last drink of one day and the first of the next never counts as overdue.
fn day_stat(day_key: String, timestamps: &[i64], interval_min: f64, goal: u32) -> DayStat {
    let drinks = timestamps.len() as u32;
    let gaps: Vec<f64> = timestamps
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).max(0) as f64 / MINUTE_MS as f64)
        .collect();

    let avg_gap_min = if gaps.is_empty() {
        None
    } else {
        Some(gaps.iter().sum::<f64>() / gaps.len() as f64)
    };
    let longest_overdue_min = gaps
        .iter()
        .map(|gap| (gap - interval_min).max(0.0))
        .fold(0.0, f64::max);

    DayStat {
        day_key,
        drinks,
        avg_gap_min,
        longest_overdue_min,
        goal_met: drinks >= goal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    const MIN: i64 = 60_000;
    const HOUR: i64 = 60 * MIN;
    const INTERVAL: i64 = 45 * MIN;

    fn ms_at_utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .expect("valid timestamp")
            .timestamp_millis()
    }

    fn drink(ts: i64) -> HistoryEntry {
        HistoryEntry::drink(ts, "click")
    }

    /// `n` drinks on the given date, one every 90 minutes from 09:00 UTC.
    fn day_of_drinks(y: i32, m: u32, d: u32, n: i64) -> Vec<HistoryEntry> {
        let start = ms_at_utc(y, m, d, 9, 0);
        (0..n).map(|i| drink(start + i * 90 * MIN)).collect()
    }

    #[test]
    fn rollover_increments_resets_and_tracks_best() {
        // Goal met -> +1, best follows.
        assert_eq!(rolled_over_streak(0, 0, 8, 8), (1, 1));
        assert_eq!(rolled_over_streak(1, 1, 9, 8), (2, 2));
        // Goal missed -> back to zero, best is kept.
        assert_eq!(rolled_over_streak(2, 2, 7, 8), (0, 2));
        // A new run has to beat the old best to move it.
        assert_eq!(rolled_over_streak(0, 5, 8, 8), (1, 5));
        assert_eq!(rolled_over_streak(4, 5, 8, 8), (5, 5));
        assert_eq!(rolled_over_streak(5, 5, 8, 8), (6, 6));
        // A zero goal cannot make every day a free win.
        assert_eq!(rolled_over_streak(0, 0, 0, 0), (0, 0));
    }

    #[test]
    fn rebuild_from_history_with_gaps() {
        let mut history = Vec::new();
        history.extend(day_of_drinks(2026, 8, 20, 8)); // met
        history.extend(day_of_drinks(2026, 8, 21, 8)); // met
        history.extend(day_of_drinks(2026, 8, 22, 3)); // missed -> reset
                                                       // 2026-08-23: nothing at all -> still zero
        history.extend(day_of_drinks(2026, 8, 24, 8)); // met
        history.extend(day_of_drinks(2026, 8, 25, 8)); // met
        history.extend(day_of_drinks(2026, 8, 26, 8)); // met
                                                       // Today (not rolled over yet) must not count, even though the goal is met.
        history.extend(day_of_drinks(2026, 8, 27, 8));

        let now = ms_at_utc(2026, 8, 27, 20, 0);
        assert_eq!(rebuild_streaks(&history, now, 8, 0), (3, 3));

        // With a goal of 3 the missed day counts too, so the run is unbroken
        // apart from the empty 23rd.
        assert_eq!(rebuild_streaks(&history, now, 3, 0), (3, 3));

        // Empty history: nothing to rebuild.
        assert_eq!(rebuild_streaks(&[], now, 8, 0), (0, 0));
        // Non-drink entries alone do not create days.
        let noise = vec![HistoryEntry::pause(now - HOUR, "tray")];
        assert_eq!(rebuild_streaks(&noise, now, 8, 0), (0, 0));
    }

    #[test]
    fn rebuild_respects_the_0400_boundary() {
        // 03:00 on the 21st still belongs to the 20th, so both drinks land on
        // the same logical day and the goal of 2 is met exactly once.
        let history = vec![
            drink(ms_at_utc(2026, 8, 20, 23, 0)),
            drink(ms_at_utc(2026, 8, 21, 3, 0)),
        ];
        let now = ms_at_utc(2026, 8, 21, 12, 0);
        assert_eq!(rebuild_streaks(&history, now, 2, 0), (1, 1));

        // Move the second drink past 04:00 and each day has one drink: the
        // 20th misses the goal of 2 and the 21st is today, so nothing counts.
        let history = vec![
            drink(ms_at_utc(2026, 8, 20, 23, 0)),
            drink(ms_at_utc(2026, 8, 21, 4, 0)),
        ];
        assert_eq!(rebuild_streaks(&history, now, 2, 0), (0, 0));
    }

    #[test]
    fn stats_cover_fourteen_days_oldest_first_with_zeros() {
        let history = day_of_drinks(2026, 8, 27, 4);
        let now = ms_at_utc(2026, 8, 27, 20, 0);
        let stats = compute_stats(&history, now, INTERVAL, 8, 0);

        assert_eq!(stats.days.len(), STATS_DAYS);
        assert_eq!(stats.days[0].day_key, "2026-08-14");
        assert_eq!(stats.days[STATS_DAYS - 1].day_key, "2026-08-27");
        // Oldest first, one calendar day apart.
        assert!(stats.days.windows(2).all(|w| w[0].day_key < w[1].day_key));

        // Empty days are present and zeroed.
        let empty = &stats.days[0];
        assert_eq!(empty.drinks, 0);
        assert_eq!(empty.avg_gap_min, None);
        assert_eq!(empty.longest_overdue_min, 0.0);
        assert!(!empty.goal_met);

        let today = &stats.days[STATS_DAYS - 1];
        assert_eq!(today.drinks, 4);
        assert!(!today.goal_met);
        assert_eq!(stats.total_drinks, 4);
    }

    #[test]
    fn day_stat_gaps_average_and_overdue() {
        // One drink: no gaps at all.
        let history = vec![drink(ms_at_utc(2026, 8, 27, 9, 0))];
        let now = ms_at_utc(2026, 8, 27, 20, 0);
        let stats = compute_stats(&history, now, INTERVAL, 1, 0);
        let today = stats.days.last().expect("today");
        assert_eq!(today.drinks, 1);
        assert_eq!(today.avg_gap_min, None);
        assert_eq!(today.longest_overdue_min, 0.0);
        assert!(today.goal_met);

        // Gaps of 30, 90 and 60 min: mean 60, worst overdue 90 - 45 = 45.
        let base = ms_at_utc(2026, 8, 27, 9, 0);
        let history = vec![
            drink(base),
            drink(base + 30 * MIN),
            drink(base + 120 * MIN),
            drink(base + 180 * MIN),
        ];
        let stats = compute_stats(&history, now, INTERVAL, 8, 0);
        let today = stats.days.last().expect("today");
        assert_eq!(today.drinks, 4);
        assert!((today.avg_gap_min.expect("avg") - 60.0).abs() < 1e-9);
        assert!((today.longest_overdue_min - 45.0).abs() < 1e-9);
    }

    #[test]
    fn overnight_gaps_never_count_as_overdue() {
        // Last drink 22:00, next one 10:00 the following day: 12 h apart, but
        // the two belong to different logical days, so neither reports it.
        let history = vec![
            drink(ms_at_utc(2026, 8, 26, 22, 0)),
            drink(ms_at_utc(2026, 8, 27, 10, 0)),
        ];
        let now = ms_at_utc(2026, 8, 27, 20, 0);
        let stats = compute_stats(&history, now, INTERVAL, 8, 0);
        for day in &stats.days {
            assert_eq!(day.longest_overdue_min, 0.0);
            assert_eq!(day.avg_gap_min, None);
        }
        assert_eq!(stats.total_drinks, 2);
    }

    #[test]
    fn stats_streaks_match_the_rebuild_and_count_all_history() {
        let mut history = day_of_drinks(2026, 7, 1, 8); // far outside the window
        history.extend(day_of_drinks(2026, 8, 25, 8));
        history.extend(day_of_drinks(2026, 8, 26, 8));
        history.extend(day_of_drinks(2026, 8, 27, 2)); // today, not counted yet
        let now = ms_at_utc(2026, 8, 27, 20, 0);

        let stats = compute_stats(&history, now, INTERVAL, 8, 0);
        assert_eq!(stats.streak, 2);
        assert!(stats.best_streak >= 2);
        // `totalDrinks` spans the whole file, not just the 14-day window.
        assert_eq!(stats.total_drinks, 26);
        // ... while `days` only holds the window.
        let windowed: u32 = stats.days.iter().map(|d| d.drinks).sum();
        assert_eq!(windowed, 18);
    }
}
