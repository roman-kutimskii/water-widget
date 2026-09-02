//! Pure, testable timer logic for Tide. No Tauri, no IO.

use chrono::{DateTime, Duration, FixedOffset, Local, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// Day rollover happens at 04:00 local time.
pub const DAY_ROLLOVER_HOURS: i64 = 4;

/// Two drinks logged within this window count as one.
/// Clicks this close to the last *counted* drink refill the bar but don't add
/// to the count (double-click protection). Anchored to the counted drink, not
/// the last click, so steady clicking can't suppress counting forever.
pub const DRINK_MERGE_WINDOW_MS: i64 = 10_000;

pub const MINUTE_MS: i64 = 60_000;

/// A manual pause is lifted automatically after this long (CONTRACT v0.2).
pub const AUTO_RESUME_MS: i64 = 120 * MINUTE_MS;

/// Being away at least this long starts a fresh session (bar back to full).
pub const AWAY_RESET_MS: i64 = 4 * 60 * MINUTE_MS;

/// After a `welcome-back` nudge, repeats are suppressed for this long.
pub const WELCOME_BACK_SUPPRESS_MS: i64 = 5 * MINUTE_MS;

/// A wall-clock jump larger than this between 1 Hz ticks means the machine slept.
pub const SLEEP_GAP_MS: i64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Zone {
    Fresh,
    Fading,
    Urgent,
    Overdue,
}

/// Widget mode. `Sleeping` means "outside active hours".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Active,
    Paused,
    Sleeping,
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
    pub mode: Mode,
    pub quiet: bool,
    pub snooze_ms: i64,
    pub paused_since: Option<i64>,
    /// Consecutive completed days that met `dailyGoal` (v0.3).
    pub streak: u32,
}

/// The mutable timing state the tick is computed from.
///
/// `sleeping_since` is deliberately not persisted: the sleeping → active
/// transition resets everything anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TimerState {
    pub last_drink_ts: i64,
    pub paused_accum_ms: i64,
    pub paused_since: Option<i64>,
    pub sleeping_since: Option<i64>,
    pub snooze_ms: i64,
}

impl TimerState {
    pub fn new(now_ms: i64) -> Self {
        Self {
            last_drink_ts: now_ms,
            ..Self::default()
        }
    }

    pub fn mode(&self) -> Mode {
        if self.paused_since.is_some() {
            Mode::Paused
        } else if self.sleeping_since.is_some() {
            Mode::Sleeping
        } else {
            Mode::Active
        }
    }

    /// `elapsed = now - lastDrinkTs - pausedAccumMs - (frozen ? now - frozenSince : 0)`
    pub fn elapsed_ms(&self, now_ms: i64) -> i64 {
        let mut elapsed = now_ms - self.last_drink_ts - self.paused_accum_ms;
        if let Some(since) = self.paused_since {
            elapsed -= (now_ms - since).max(0);
        }
        if let Some(since) = self.sleeping_since {
            elapsed -= (now_ms - since).max(0);
        }
        elapsed.max(0)
    }

    /// Elapsed with the accumulated snooze taken off.
    pub fn effective_ms(&self, now_ms: i64) -> i64 {
        (self.elapsed_ms(now_ms) - self.snooze_ms).max(0)
    }

    /// Drink: bar back to full, snooze cleared. Pause state is untouched.
    pub fn drink(&mut self, now_ms: i64) {
        self.last_drink_ts = now_ms;
        self.snooze_ms = 0;
        self.paused_accum_ms = 0;
        if self.paused_since.is_some() {
            self.paused_since = Some(now_ms);
        }
        if self.sleeping_since.is_some() {
            self.sleeping_since = Some(now_ms);
        }
    }

    /// Adds `minutes` (clamped to 1..=60) of snooze without moving `lastDrinkTs`.
    pub fn snooze(&mut self, minutes: i64) -> i64 {
        let minutes = minutes.clamp(SNOOZE_MIN_MINUTES, SNOOZE_MAX_MINUTES);
        self.snooze_ms += minutes * MINUTE_MS;
        minutes
    }

    /// Idempotent. Returns true when the pause state actually changed.
    pub fn set_paused(&mut self, paused: bool, now_ms: i64) -> bool {
        match (paused, self.paused_since) {
            (true, None) => {
                self.paused_since = Some(now_ms);
                true
            }
            (false, Some(since)) => {
                self.paused_accum_ms += (now_ms - since).max(0);
                self.paused_since = None;
                true
            }
            _ => false,
        }
    }

    /// Auto-resume once a pause has lasted [`AUTO_RESUME_MS`].
    pub fn should_auto_resume(&self, now_ms: i64) -> bool {
        matches!(self.paused_since, Some(since) if now_ms - since >= AUTO_RESUME_MS)
    }

    /// Enter/leave the "outside active hours" freeze. Returns true on a change.
    /// Leaving sleeping resets the timer: full bar, no snooze, no paused accum.
    pub fn set_sleeping(&mut self, sleeping: bool, now_ms: i64) -> bool {
        match (sleeping, self.sleeping_since) {
            (true, None) => {
                self.sleeping_since = Some(now_ms);
                true
            }
            (false, Some(_)) => {
                self.sleeping_since = None;
                self.last_drink_ts = now_ms;
                self.snooze_ms = 0;
                self.paused_accum_ms = 0;
                true
            }
            _ => false,
        }
    }

    /// Fresh session after a long absence.
    pub fn reset_session(&mut self, now_ms: i64) {
        self.last_drink_ts = now_ms;
        self.snooze_ms = 0;
        self.paused_accum_ms = 0;
    }
}

pub const SNOOZE_MIN_MINUTES: i64 = 1;
pub const SNOOZE_MAX_MINUTES: i64 = 60;

/// Compute the current tick from the full v0.2 timing state.
pub fn compute_tick_full(
    now_ms: i64,
    timer: &TimerState,
    interval_ms: i64,
    today_count: u32,
    quiet: bool,
    streak: u32,
) -> Tick {
    let interval = interval_ms.max(1);
    let effective = timer.effective_ms(now_ms);

    let fill = (1.0 - (effective as f64 / interval as f64)).clamp(0.0, 1.0);

    Tick {
        fill,
        zone: zone_for_fill(fill),
        remaining_ms: (interval - effective).max(0),
        overdue_ms: (effective - interval).max(0),
        today_count,
        interval_ms: interval,
        last_drink_ts: timer.last_drink_ts,
        mode: timer.mode(),
        quiet,
        snooze_ms: timer.snooze_ms,
        paused_since: timer.paused_since,
        streak,
    }
}

/// MVP-shaped helper: plain active timer, no pause/snooze/quiet.
/// `interval_ms` must be > 0; a non-positive value is treated as
/// "immediately overdue" rather than panicking.
pub fn compute_tick(now_ms: i64, last_drink_ts: i64, interval_ms: i64, today_count: u32) -> Tick {
    let timer = TimerState {
        last_drink_ts,
        ..TimerState::default()
    };
    compute_tick_full(now_ms, &timer, interval_ms, today_count, false, 0)
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

// ------------------------------------------------------------- time windows

/// Parses `"HH:MM"` into minutes since midnight. `None` when malformed.
pub fn parse_hhmm(value: &str) -> Option<u32> {
    let (h, m) = value.split_once(':')?;
    if h.len() != 2 || m.len() != 2 || !h.bytes().chain(m.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hours: u32 = h.parse().ok()?;
    let minutes: u32 = m.parse().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(hours * 60 + minutes)
}

/// Minutes since local midnight for `now_ms` at the given UTC offset (seconds east).
pub fn minutes_of_day(now_ms: i64, local_offset_secs: i32) -> u32 {
    let offset = FixedOffset::east_opt(local_offset_secs)
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("UTC offset is always valid"));
    let utc: DateTime<Utc> = Utc
        .timestamp_millis_opt(now_ms)
        .single()
        .unwrap_or_else(Utc::now);
    let local = utc.with_timezone(&offset);
    use chrono::Timelike;
    local.hour() * 60 + local.minute()
}

/// Half-open `[start, end)` window over minutes-of-day; ranges may cross midnight.
/// `start == end` yields false (callers give that case its own meaning).
pub fn in_window(minute: u32, start: u32, end: u32) -> bool {
    if start == end {
        false
    } else if start < end {
        minute >= start && minute < end
    } else {
        minute >= start || minute < end
    }
}

/// Active hours. `start == end` means "always active".
pub fn is_active_at(minute: u32, start: &str, end: &str) -> bool {
    let (Some(s), Some(e)) = (parse_hhmm(start), parse_hhmm(end)) else {
        return true; // unparseable settings must never lock the user out
    };
    if s == e {
        return true;
    }
    in_window(minute, s, e)
}

/// Quiet hours. `start == end` means "never quiet".
pub fn is_quiet_at(minute: u32, start: &str, end: &str) -> bool {
    let (Some(s), Some(e)) = (parse_hhmm(start), parse_hhmm(end)) else {
        return false;
    };
    if s == e {
        return false;
    }
    in_window(minute, s, e)
}

// ------------------------------------------------------------------ nudges

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NudgeKind {
    Overdue,
    Repeat,
    WelcomeBack,
    AutoResume,
}

/// Payload of the `nudge` event.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Nudge {
    pub kind: NudgeKind,
    pub overdue_ms: i64,
}

/// Bookkeeping for the overdue nudge schedule. Purely in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NudgeState {
    /// The one-shot `overdue` nudge has already fired for this overdue streak.
    pub fired_overdue: bool,
    /// How many `repeat` nudges fired in this streak.
    pub repeats: u32,
    /// Timestamp of the last nudge of any kind.
    pub last_nudge_ts: i64,
    /// No nudges before this timestamp (welcome-back grace).
    pub suppress_until: i64,
}

impl NudgeState {
    /// Called when the bar is no longer overdue (drink, reset, snooze).
    pub fn reset(&mut self) {
        self.fired_overdue = false;
        self.repeats = 0;
        self.last_nudge_ts = 0;
    }

    /// Suppress every nudge for the next 5 minutes and count the streak as
    /// already announced.
    pub fn suppress_after_welcome_back(&mut self, now_ms: i64) {
        self.fired_overdue = true;
        self.repeats = 0;
        self.last_nudge_ts = now_ms;
        self.suppress_until = now_ms + WELCOME_BACK_SUPPRESS_MS;
    }

    /// Decide whether a nudge is due now. Mutates the schedule when it returns
    /// `Some`. `quiet` suppresses (ambient only) without advancing the schedule.
    pub fn poll(
        &mut self,
        now_ms: i64,
        overdue: bool,
        every_min: u32,
        max_repeats: u32,
        quiet: bool,
    ) -> Option<NudgeKind> {
        if !overdue {
            self.reset();
            return None;
        }
        if quiet || now_ms < self.suppress_until {
            return None;
        }
        if !self.fired_overdue {
            self.fired_overdue = true;
            self.last_nudge_ts = now_ms;
            return Some(NudgeKind::Overdue);
        }
        let every_ms = i64::from(every_min.max(1)) * MINUTE_MS;
        if self.repeats < max_repeats && now_ms - self.last_nudge_ts >= every_ms {
            self.repeats += 1;
            self.last_nudge_ts = now_ms;
            return Some(NudgeKind::Repeat);
        }
        None
    }
}

/// What to do after returning from a locked session or system sleep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwayOutcome {
    /// Nothing special; the timer just kept running.
    Continue,
    /// `away >= interval`: nudge with `welcome-back`, suppress repeats 5 min.
    WelcomeBack,
    /// `away >= 4h`: fresh session, bar back to full, no count.
    ResetSession,
}

pub fn away_outcome(away_ms: i64, interval_ms: i64) -> AwayOutcome {
    if away_ms >= AWAY_RESET_MS {
        AwayOutcome::ResetSession
    } else if away_ms >= interval_ms.max(1) {
        AwayOutcome::WelcomeBack
    } else {
        AwayOutcome::Continue
    }
}

// -------------------------------------------------------------------- dates

/// `YYYY-MM-DD` key for the "logical day" of `now_ms`, in the given local UTC
/// offset (seconds east of UTC), with the 04:00 rollover applied.
pub fn day_key(now_ms: i64, local_offset_secs: i32) -> String {
    let offset = FixedOffset::east_opt(local_offset_secs).unwrap_or_else(|| {
        // east_opt only fails for out-of-range offsets; fall back to UTC.
        FixedOffset::east_opt(0).expect("UTC offset is always valid")
    });
    let utc: DateTime<Utc> = Utc
        .timestamp_millis_opt(now_ms)
        .single()
        .unwrap_or_else(Utc::now);
    let local = utc.with_timezone(&offset) - Duration::hours(DAY_ROLLOVER_HOURS);
    local.format("%Y-%m-%d").to_string()
}

/// `day_key` using the machine's current local timezone offset.
pub fn day_key_local(now_ms: i64) -> String {
    day_key(now_ms, local_offset_secs())
}

/// The machine's current UTC offset in seconds east of UTC.
pub fn local_offset_secs() -> i32 {
    Local::now().offset().local_minus_utc()
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
        assert_eq!(t.mode, Mode::Active);
        assert!(!t.quiet);
        assert_eq!(t.snooze_ms, 0);
        assert_eq!(t.paused_since, None);
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
        assert!(should_merge_drink(NOW + 5_000, NOW));
        assert!(should_merge_drink(NOW, NOW));
        assert!(should_merge_drink(NOW + 9_999, NOW));
        assert!(!should_merge_drink(NOW + 10_000, NOW));
        assert!(!should_merge_drink(NOW + 10 * MIN, NOW));
        // Never merge against "no counted drink yet" (e.g. right after a reset).
        assert!(!should_merge_drink(NOW, 0));
        // Clock jumped backwards: do not merge, treat as a fresh drink.
        assert!(!should_merge_drink(NOW - 1, NOW));
    }

    // ------------------------------------------------------------ v0.2

    #[test]
    fn snooze_pushes_the_bar_back_without_moving_last_drink() {
        let mut timer = TimerState::new(NOW);
        // 30 min in, 15 min left.
        let now = NOW + 30 * MIN;
        assert_eq!(timer.snooze(10), 10);
        let t = compute_tick_full(now, &timer, INTERVAL, 0, false, 0);
        assert_eq!(t.last_drink_ts, NOW);
        assert_eq!(t.snooze_ms, 10 * MIN);
        assert_eq!(t.remaining_ms, 25 * MIN);
        assert!((t.fill - 25.0 / 45.0).abs() < 1e-9);

        // Snooze accumulates and is clamped into 1..=60.
        assert_eq!(timer.snooze(500), 60);
        assert_eq!(timer.snooze(0), 1);
        assert_eq!(timer.snooze_ms, 71 * MIN);

        // Drinking clears it.
        timer.drink(now);
        assert_eq!(timer.snooze_ms, 0);
        assert_eq!(timer.last_drink_ts, now);
    }

    #[test]
    fn pause_resume_accumulates_paused_time() {
        let mut timer = TimerState::new(NOW);
        // Pause 10 min in.
        assert!(timer.set_paused(true, NOW + 10 * MIN));
        assert_eq!(timer.mode(), Mode::Paused);
        // Pausing again is a no-op.
        assert!(!timer.set_paused(true, NOW + 11 * MIN));

        // While paused the elapsed time is frozen at 10 min.
        let t = compute_tick_full(NOW + 40 * MIN, &timer, INTERVAL, 0, false, 0);
        assert_eq!(t.remaining_ms, 35 * MIN);
        assert_eq!(t.mode, Mode::Paused);
        assert_eq!(t.paused_since, Some(NOW + 10 * MIN));

        // Resume after 30 min of pause; time starts flowing again.
        assert!(timer.set_paused(false, NOW + 40 * MIN));
        assert_eq!(timer.paused_accum_ms, 30 * MIN);
        assert!(!timer.set_paused(false, NOW + 41 * MIN));
        let t = compute_tick_full(NOW + 45 * MIN, &timer, INTERVAL, 0, false, 0);
        assert_eq!(t.mode, Mode::Active);
        assert_eq!(t.remaining_ms, 30 * MIN);
    }

    #[test]
    fn auto_resume_after_120_minutes() {
        let mut timer = TimerState::new(NOW);
        timer.set_paused(true, NOW);
        assert!(!timer.should_auto_resume(NOW + 119 * MIN));
        assert!(timer.should_auto_resume(NOW + 120 * MIN));
        timer.set_paused(false, NOW + 120 * MIN);
        assert_eq!(timer.paused_accum_ms, 120 * MIN);
        assert!(!timer.should_auto_resume(NOW + 500 * MIN));
    }

    #[test]
    fn sleeping_freezes_and_transition_to_active_resets() {
        let mut timer = TimerState::new(NOW);
        timer.snooze(5);
        timer.set_paused(true, NOW + MIN);
        timer.set_paused(false, NOW + 3 * MIN);
        assert_eq!(timer.paused_accum_ms, 2 * MIN);

        // Go to sleep 30 min in; the bar freezes.
        assert!(timer.set_sleeping(true, NOW + 30 * MIN));
        assert_eq!(timer.mode(), Mode::Sleeping);
        let frozen = compute_tick_full(NOW + 30 * MIN, &timer, INTERVAL, 0, false, 0);
        let later = compute_tick_full(NOW + 8 * 60 * MIN, &timer, INTERVAL, 0, false, 0);
        assert_eq!(frozen.remaining_ms, later.remaining_ms);
        assert_eq!(later.mode, Mode::Sleeping);

        // Waking up resets to a full bar without touching the count.
        let wake = NOW + 10 * 60 * MIN;
        assert!(timer.set_sleeping(false, wake));
        assert_eq!(timer.last_drink_ts, wake);
        assert_eq!(timer.snooze_ms, 0);
        assert_eq!(timer.paused_accum_ms, 0);
        let t = compute_tick_full(wake, &timer, INTERVAL, 7, false, 0);
        assert_eq!(t.fill, 1.0);
        assert_eq!(t.today_count, 7);
        assert_eq!(t.mode, Mode::Active);
    }

    #[test]
    fn paused_wins_over_sleeping_in_mode() {
        let mut timer = TimerState::new(NOW);
        timer.set_sleeping(true, NOW);
        timer.set_paused(true, NOW);
        assert_eq!(timer.mode(), Mode::Paused);
    }

    #[test]
    fn hhmm_parsing() {
        assert_eq!(parse_hhmm("00:00"), Some(0));
        assert_eq!(parse_hhmm("08:30"), Some(510));
        assert_eq!(parse_hhmm("23:59"), Some(23 * 60 + 59));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("08:60"), None);
        assert_eq!(parse_hhmm("8:30"), None);
        assert_eq!(parse_hhmm("0830"), None);
        assert_eq!(parse_hhmm(""), None);
        assert_eq!(parse_hhmm("ab:cd"), None);
    }

    #[test]
    fn active_window_including_midnight_crossing() {
        let at = |h: u32, m: u32| h * 60 + m;
        // Normal daytime window.
        assert!(is_active_at(at(8, 0), "08:00", "22:00"));
        assert!(is_active_at(at(21, 59), "08:00", "22:00"));
        assert!(!is_active_at(at(22, 0), "08:00", "22:00"));
        assert!(!is_active_at(at(7, 59), "08:00", "22:00"));
        // Night shift: 22:00 -> 06:00 crosses midnight.
        assert!(is_active_at(at(23, 0), "22:00", "06:00"));
        assert!(is_active_at(at(0, 30), "22:00", "06:00"));
        assert!(is_active_at(at(5, 59), "22:00", "06:00"));
        assert!(!is_active_at(at(6, 0), "22:00", "06:00"));
        assert!(!is_active_at(at(12, 0), "22:00", "06:00"));
        // start == end -> always active.
        assert!(is_active_at(at(3, 0), "09:00", "09:00"));
        // Garbage settings must not disable the widget.
        assert!(is_active_at(at(3, 0), "nope", "22:00"));
    }

    #[test]
    fn quiet_window_including_midnight_crossing() {
        let at = |h: u32, m: u32| h * 60 + m;
        assert!(is_quiet_at(at(23, 0), "22:00", "08:00"));
        assert!(is_quiet_at(at(2, 0), "22:00", "08:00"));
        assert!(!is_quiet_at(at(8, 0), "22:00", "08:00"));
        assert!(!is_quiet_at(at(12, 0), "22:00", "08:00"));
        // start == end -> never quiet.
        assert!(!is_quiet_at(at(23, 0), "12:00", "12:00"));
        // Garbage settings -> not quiet.
        assert!(!is_quiet_at(at(23, 0), "22:00", "oops"));
    }

    #[test]
    fn minutes_of_day_uses_offset() {
        let ts = ms_at_utc(2026, 9, 2, 1, 15);
        assert_eq!(minutes_of_day(ts, 0), 75);
        assert_eq!(minutes_of_day(ts, 3 * 3600), 4 * 60 + 15);
        // Crossing back over midnight.
        assert_eq!(minutes_of_day(ts, -2 * 3600), 23 * 60 + 15);
    }

    #[test]
    fn nudge_schedule_fires_once_then_repeats_up_to_max() {
        let mut n = NudgeState::default();
        // Not overdue: nothing.
        assert_eq!(n.poll(NOW, false, 10, 3, false), None);
        // First moment of overdue.
        assert_eq!(n.poll(NOW, true, 10, 3, false), Some(NudgeKind::Overdue));
        // Not again on the next tick.
        assert_eq!(n.poll(NOW + 1000, true, 10, 3, false), None);
        assert_eq!(n.poll(NOW + 9 * MIN, true, 10, 3, false), None);
        assert_eq!(
            n.poll(NOW + 10 * MIN, true, 10, 3, false),
            Some(NudgeKind::Repeat)
        );
        assert_eq!(
            n.poll(NOW + 20 * MIN, true, 10, 3, false),
            Some(NudgeKind::Repeat)
        );
        assert_eq!(
            n.poll(NOW + 30 * MIN, true, 10, 3, false),
            Some(NudgeKind::Repeat)
        );
        // Cap reached.
        assert_eq!(n.poll(NOW + 40 * MIN, true, 10, 3, false), None);
        assert_eq!(n.poll(NOW + 90 * MIN, true, 10, 3, false), None);

        // Drinking clears the streak, so the next overdue nudges again.
        assert_eq!(n.poll(NOW + 91 * MIN, false, 10, 3, false), None);
        assert_eq!(
            n.poll(NOW + 92 * MIN, true, 10, 3, false),
            Some(NudgeKind::Overdue)
        );
    }

    #[test]
    fn nudge_max_zero_means_no_repeats() {
        let mut n = NudgeState::default();
        assert_eq!(n.poll(NOW, true, 1, 0, false), Some(NudgeKind::Overdue));
        assert_eq!(n.poll(NOW + 60 * MIN, true, 1, 0, false), None);
    }

    #[test]
    fn nudges_are_suppressed_while_quiet() {
        let mut n = NudgeState::default();
        // Quiet: never fires, and the schedule does not advance.
        assert_eq!(n.poll(NOW, true, 10, 3, true), None);
        assert_eq!(n.poll(NOW + 60 * MIN, true, 10, 3, true), None);
        assert!(!n.fired_overdue);
        // Quiet hours end: the overdue nudge is still owed.
        assert_eq!(
            n.poll(NOW + 61 * MIN, true, 10, 3, false),
            Some(NudgeKind::Overdue)
        );
    }

    #[test]
    fn welcome_back_suppresses_nudges_for_five_minutes() {
        let mut n = NudgeState::default();
        n.suppress_after_welcome_back(NOW);
        assert_eq!(n.poll(NOW + MIN, true, 1, 3, false), None);
        assert_eq!(n.poll(NOW + 4 * MIN, true, 1, 3, false), None);
        // After the grace period repeats resume (the overdue one-shot is spent).
        assert_eq!(
            n.poll(NOW + 5 * MIN, true, 1, 3, false),
            Some(NudgeKind::Repeat)
        );
    }

    #[test]
    fn away_rules() {
        assert_eq!(away_outcome(0, INTERVAL), AwayOutcome::Continue);
        assert_eq!(away_outcome(INTERVAL - 1, INTERVAL), AwayOutcome::Continue);
        assert_eq!(away_outcome(INTERVAL, INTERVAL), AwayOutcome::WelcomeBack);
        assert_eq!(
            away_outcome(3 * 60 * MIN + 59 * MIN, INTERVAL),
            AwayOutcome::WelcomeBack
        );
        assert_eq!(
            away_outcome(4 * 60 * MIN, INTERVAL),
            AwayOutcome::ResetSession
        );
        assert_eq!(
            away_outcome(10 * 60 * MIN, INTERVAL),
            AwayOutcome::ResetSession
        );
    }

    #[test]
    fn away_reset_restores_a_full_bar() {
        let mut timer = TimerState::new(NOW);
        timer.snooze(10);
        let back = NOW + 5 * 60 * MIN;
        assert_eq!(
            away_outcome(5 * 60 * MIN, INTERVAL),
            AwayOutcome::ResetSession
        );
        timer.reset_session(back);
        let t = compute_tick_full(back, &timer, INTERVAL, 2, false, 0);
        assert_eq!(t.fill, 1.0);
        assert_eq!(t.snooze_ms, 0);
        assert_eq!(t.today_count, 2);
    }

    #[test]
    fn quiet_flag_rides_on_the_tick() {
        let timer = TimerState::new(NOW);
        let t = compute_tick_full(NOW, &timer, INTERVAL, 0, true, 0);
        assert!(t.quiet);
    }

    #[test]
    fn streak_rides_on_the_tick() {
        let timer = TimerState::new(NOW);
        let t = compute_tick_full(NOW, &timer, INTERVAL, 0, false, 6);
        assert_eq!(t.streak, 6);
        // The MVP helper reports no streak.
        assert_eq!(compute_tick(NOW, NOW, INTERVAL, 0).streak, 0);
    }
}
