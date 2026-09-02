//! Pure, testable timer logic for Tide. No Tauri, no IO.

use chrono::{DateTime, Duration, FixedOffset, Local, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// Day rollover happens at 04:00 local time.
pub const DAY_ROLLOVER_HOURS: i64 = 4;

/// Two drinks logged within this window count as one.
pub const DRINK_MERGE_WINDOW_MS: i64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Zone {
    Fresh,
    Fading,
    Urgent,
    Overdue,
}

/// Payload of the `tick` event. Field names must match CONTRACT.md exactly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tick {
    pub fill: f64,
    pub zone: Zone,
    pub remaining_ms: i64,
    pub overdue_ms: i64,
    pub today_count: u32,
    pub interval_ms: i64,
    pub last_drink_ts: i64,
}

/// Compute the current tick. `interval_ms` must be > 0; a non-positive value is
/// treated as "immediately overdue" rather than panicking.
pub fn compute_tick(now_ms: i64, last_drink_ts: i64, interval_ms: i64, today_count: u32) -> Tick {
    let interval = interval_ms.max(1);
    // Clock set backwards (or a future timestamp on disk): clamp to 0 elapsed.
    let elapsed = (now_ms - last_drink_ts).max(0);

    let fill = (1.0 - (elapsed as f64 / interval as f64)).clamp(0.0, 1.0);
    let zone = zone_for_fill(fill);

    Tick {
        fill,
        zone,
        remaining_ms: (interval - elapsed).max(0),
        overdue_ms: (elapsed - interval).max(0),
        today_count,
        interval_ms: interval,
        last_drink_ts,
    }
}

/// Zone thresholds per CONTRACT.md: fresh >= 0.6, fading >= 0.3, urgent > 0, overdue = 0.
pub fn zone_for_fill(fill: f64) -> Zone {
    if fill >= 0.6 {
        Zone::Fresh
    } else if fill >= 0.3 {
        Zone::Fading
    } else if fill > 0.0 {
        Zone::Urgent
    } else {
        Zone::Overdue
    }
}

/// `YYYY-MM-DD` key for the "logical day" of `now_ms`, in the given local UTC
/// offset (seconds east of UTC), with the 04:00 rollover applied.
pub fn day_key(now_ms: i64, local_offset_secs: i32) -> String {
    let offset = FixedOffset::east_opt(local_offset_secs).unwrap_or_else(|| {
        // east_opt only fails for out-of-range offsets; fall back to UTC.
        FixedOffset::east_opt(0).expect("UTC offset is always valid")
    });
    let utc: DateTime<Utc> = Utc.timestamp_millis_opt(now_ms).single().unwrap_or_else(Utc::now);
    let local = utc.with_timezone(&offset) - Duration::hours(DAY_ROLLOVER_HOURS);
    local.format("%Y-%m-%d").to_string()
}

/// `day_key` using the machine's current local timezone offset.
pub fn day_key_local(now_ms: i64) -> String {
    let offset_secs = Local::now().offset().local_minus_utc();
    day_key(now_ms, offset_secs)
}

/// True when a drink at `now_ms` should merge into the previous one
/// (state updates, but the daily count does not increment).
pub fn should_merge_drink(now_ms: i64, last_drink_ts: i64) -> bool {
    let delta = now_ms - last_drink_ts;
    (0..DRINK_MERGE_WINDOW_MS).contains(&delta)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;
    const INTERVAL: i64 = 45 * MIN;
    const NOW: i64 = 1_800_000_000_000; // 2027-01-15T08:00:00Z, arbitrary anchor

    #[test]
    fn full_at_drink_time() {
        let t = compute_tick(NOW, NOW, INTERVAL, 3);
        assert_eq!(t.fill, 1.0);
        assert_eq!(t.zone, Zone::Fresh);
        assert_eq!(t.remaining_ms, INTERVAL);
        assert_eq!(t.overdue_ms, 0);
        assert_eq!(t.today_count, 3);
        assert_eq!(t.last_drink_ts, NOW);
        assert_eq!(t.interval_ms, INTERVAL);
    }

    #[test]
    fn half_at_half_interval() {
        let t = compute_tick(NOW + INTERVAL / 2, NOW, INTERVAL, 0);
        assert!((t.fill - 0.5).abs() < 1e-9);
        assert_eq!(t.zone, Zone::Fading);
        assert_eq!(t.remaining_ms, INTERVAL / 2);
        assert_eq!(t.overdue_ms, 0);
    }

    #[test]
    fn zone_boundaries() {
        // Exactly 0.6 -> fresh, a hair below -> fading.
        assert_eq!(zone_for_fill(0.6), Zone::Fresh);
        assert_eq!(zone_for_fill(0.6 - 1e-9), Zone::Fading);
        // Exactly 0.3 -> fading, a hair below -> urgent.
        assert_eq!(zone_for_fill(0.3), Zone::Fading);
        assert_eq!(zone_for_fill(0.3 - 1e-9), Zone::Urgent);
        // Anything above 0 -> urgent, exactly 0 -> overdue.
        assert_eq!(zone_for_fill(1e-9), Zone::Urgent);
        assert_eq!(zone_for_fill(0.0), Zone::Overdue);
        assert_eq!(zone_for_fill(1.0), Zone::Fresh);
    }

    #[test]
    fn zone_boundaries_through_compute_tick() {
        // fill = 0.6 at 40% elapsed
        let t = compute_tick(NOW + (INTERVAL * 2) / 5, NOW, INTERVAL, 0);
        assert_eq!(t.zone, Zone::Fresh);
        // fill = 0.3 at 70% elapsed
        let t = compute_tick(NOW + (INTERVAL * 7) / 10, NOW, INTERVAL, 0);
        assert_eq!(t.zone, Zone::Fading);
        // 1 ms before the interval elapses
        let t = compute_tick(NOW + INTERVAL - 1, NOW, INTERVAL, 0);
        assert_eq!(t.zone, Zone::Urgent);
        assert_eq!(t.remaining_ms, 1);
    }

    #[test]
    fn overdue_counting() {
        let t = compute_tick(NOW + INTERVAL, NOW, INTERVAL, 0);
        assert_eq!(t.fill, 0.0);
        assert_eq!(t.zone, Zone::Overdue);
        assert_eq!(t.remaining_ms, 0);
        assert_eq!(t.overdue_ms, 0);

        let t = compute_tick(NOW + INTERVAL + 12 * MIN, NOW, INTERVAL, 0);
        assert_eq!(t.zone, Zone::Overdue);
        assert_eq!(t.fill, 0.0);
        assert_eq!(t.remaining_ms, 0);
        assert_eq!(t.overdue_ms, 12 * MIN);
    }

    #[test]
    fn clock_set_backwards_clamps_to_full() {
        let t = compute_tick(NOW - 10 * MIN, NOW, INTERVAL, 0);
        assert_eq!(t.fill, 1.0);
        assert_eq!(t.remaining_ms, INTERVAL);
        assert_eq!(t.overdue_ms, 0);
    }

    fn ms_at_utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .expect("valid timestamp")
            .timestamp_millis()
    }

    #[test]
    fn day_rollover_just_before_and_after_0400() {
        // UTC offset zero for a deterministic test.
        let before = ms_at_utc(2026, 9, 2, 3, 59);
        let after = ms_at_utc(2026, 9, 2, 4, 0);
        assert_eq!(day_key(before, 0), "2026-09-01");
        assert_eq!(day_key(after, 0), "2026-09-02");
        // Late evening still belongs to the same logical day.
        assert_eq!(day_key(ms_at_utc(2026, 9, 2, 23, 30), 0), "2026-09-02");
        // ... and 03:59 the next morning still does too.
        assert_eq!(day_key(ms_at_utc(2026, 9, 3, 3, 59), 0), "2026-09-02");
    }

    #[test]
    fn day_key_respects_local_offset() {
        // 01:00 UTC is 04:00 in UTC+3 -> already the new logical day there,
        // while it is still the previous day in UTC.
        let ts = ms_at_utc(2026, 9, 2, 1, 0);
        assert_eq!(day_key(ts, 3 * 3600), "2026-09-02");
        assert_eq!(day_key(ts, 0), "2026-09-01");
    }

    #[test]
    fn merge_window() {
        assert!(should_merge_drink(NOW + 10_000, NOW));
        assert!(should_merge_drink(NOW, NOW));
        assert!(should_merge_drink(NOW + 59_999, NOW));
        assert!(!should_merge_drink(NOW + 60_000, NOW));
        assert!(!should_merge_drink(NOW + 10 * MIN, NOW));
        // Clock jumped backwards: do not merge, treat as a fresh drink.
        assert!(!should_merge_drink(NOW - 1, NOW));
    }
}
