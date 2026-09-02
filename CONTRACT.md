# Tide MVP — IPC contract (Rust core ⇄ web UI)

Frozen for MVP. Both sides implement exactly this. Any change goes here first.

## Windows

| Label      | Page                  | Properties |
|------------|-----------------------|------------|
| `widget`   | `index.html`          | 220×28 logical px, frameless (`decorations: false`), `transparent: true`, `alwaysOnTop: true`, `skipTaskbar: true`, `resizable: false`, `shadow: false`. Position restored from state, default: primary monitor bottom-right with 24 px margin. |
| `settings` | `settings.html`       | 380×320, normal decorations, `resizable: false`, created lazily, destroyed on close and recreated on next open, single instance. |

## Events (Rust → UI), via `emit`

### `tick` — every 1000 ms, and immediately after any state change
```ts
interface Tick {
  fill: number;          // 0..1, 1 = just drank
  zone: 'fresh' | 'fading' | 'urgent' | 'overdue';
  remainingMs: number;   // >= 0; 0 when overdue
  overdueMs: number;     // >= 0; 0 when not overdue
  todayCount: number;
  intervalMs: number;
  lastDrinkTs: number;   // unix ms
}
```
Zone thresholds: fresh ≥ 0.6, fading ≥ 0.3, urgent > 0, overdue = 0.

### `settings-changed` — after any settings write
Payload: `Settings` (below). Both windows receive it.

## Commands (UI → Rust), via `invoke`

| Command          | Args                          | Returns     | Notes |
|------------------|-------------------------------|-------------|-------|
| `drink`          | `{ source: 'click'\|'hotkey'\|'toast' }` | `Tick`      | Resets `lastDrinkTs = now`, `todayCount += 1`. Events within 10 s of the previous *counted* drink are merged: state updates but count does not increment (double-click protection; measured from the last counted drink, never from a reset, so steady clicking still counts). Appends to history. Emits `tick`. |
| `get_tick`       | –                             | `Tick`      | Current computed tick (for initial render). |
| `get_settings`   | –                             | `Settings`  | |
| `set_settings`   | `{ settings: Settings }`      | `Settings`  | Validates + clamps, persists, emits `settings-changed` and `tick`. |
| `open_settings`  | –                             | `void`      | Show/focus the settings window. |
| `save_position`  | `{ x: number, y: number }`    | `void`      | Physical px, called by UI after a drag ends (`onMoved`). |
| `quit`           | –                             | `void`      | Flush state, exit. |

```ts
interface Settings {
  intervalMin: number;   // 10..180, default 45
  opacity: number;       // 0.3..1.0, default 0.9
  showText: boolean;     // default true
  showCount: boolean;    // default true
}
```

## Persistence (Rust only)

Directory: `%APPDATA%/dev.kutimskii.tide/` (Tauri `app_config_dir`, named after the app identifier).
- `settings.json` — `{ "version": 1, ...Settings }`
- `state.json` — `{ "version": 1, "lastDrinkTs": number, "todayCount": number, "dayKey": "YYYY-MM-DD", "position": { "x": number, "y": number } | null }`
- `history.jsonl` — one JSON object per line: `{ "ts": number, "type": "drink", "source": string }`

Day rollover at 04:00 local: when the computed `dayKey` (date of `now - 4h`) differs from stored, reset `todayCount` to 0.
First launch: `lastDrinkTs = now` (bar starts full).

## Colors (UI only)

Interpolate in OKLCH between stops by fill:
- 1.00 → `#3B82F6`
- 0.60 → `#22C55E`
- 0.45 → `#EAB308`
- 0.30 → `#F97316`
- 0.00 → `#EF4444`
Overdue: `#EF4444` with 1.5 s opacity pulse 0.6↔1.0.

## Tray

Icon + menu: **Drink now**, **Snooze 10 min**, **Pause / Resume** (label reflects state), **Settings…**, separator, **Quit**. Left-click on tray icon = Drink now.

---

# v0.2 additions (2026-09-02)

Everything above stays valid. The MVP shapes are extended as follows; all new Settings fields have defaults so old `settings.json` files still load.

## Tick (extended)

```ts
interface Tick {
  // ...MVP fields unchanged...
  mode: 'active' | 'paused' | 'sleeping';  // sleeping = outside active hours
  quiet: boolean;        // true during quiet hours, DND/Focus Assist, full-screen app, or session locked
  snoozeMs: number;      // total snooze added since last drink (0 if none)
  pausedSince: number | null;  // unix ms while mode === 'paused'
}
```

Timing rules (Rust, `engine.rs`, pure + unit-tested):
- `elapsed = now - lastDrinkTs - pausedAccumMs - (paused ? now - pausedSince : 0)`
- `effective = max(0, elapsed - snoozeMs)`; `fill = clamp(1 - effective / intervalMs, 0, 1)`.
- Snooze adds N min to `snoozeMs` without touching `lastDrinkTs`; cleared on drink.
- Pause: `pausedSince = now`; resume: `pausedAccumMs += now - pausedSince`. Auto-resume after 120 min (emit `nudge` kind `auto-resume` + toast).
- Sleeping (outside active hours): timer frozen exactly like paused, bar rendered as full/grey "zzz". On the transition sleeping → active: `lastDrinkTs = now`, `snoozeMs = 0`, `pausedAccumMs = 0`, no count increment.
- Session lock / system sleep: time keeps accruing. On unlock/resume with `away = now - lockedAt`:
  - `away >= 4h` → `lastDrinkTs = now`, `snoozeMs = 0` (fresh session, no count).
  - `away >= intervalMs` → emit `nudge` kind `welcome-back` (+ toast "Welcome back, drink now?"), suppress repeat nudges for 5 min.
- Day rollover unchanged.

## Events (extended)

### `nudge` — Rust → UI
```ts
interface Nudge {
  kind: 'overdue' | 'repeat' | 'welcome-back' | 'auto-resume';
  overdueMs: number;
}
```
Emitted: `overdue` once when fill first hits 0; `repeat` every `nudgeEveryMin` while still overdue, at most `nudgeMax` times; `welcome-back` / `auto-resume` as above. Never emitted while `quiet` is true (ambient only) — except `auto-resume`.
UI reaction: 300 ms wobble animation on the widget; if `soundEnabled`, play the built-in drop sound (WebAudio-synthesised, no asset file) at `soundVolume`. Respect `prefers-reduced-motion`.

### `mode-changed` — not needed; `mode` rides on `tick`.

## Commands (extended)

| Command        | Args                       | Returns | Notes |
|----------------|----------------------------|---------|-------|
| `snooze`       | `{ minutes: number }`      | `Tick`  | 1..60, clamped. Appends history `{type:'snooze', minutes}`. |
| `set_paused`   | `{ paused: boolean }`      | `Tick`  | Idempotent. History `pause` / `resume`. |
| `drink`        | unchanged; `source` may now also be `'hotkey'` or `'toast'`. |
| `reset_today`  | –                          | `Tick`  | `todayCount = 0` and the bar refills (fresh timer, snooze cleared, nudges reset). |

## Settings (extended)

```ts
interface Settings {
  // MVP
  intervalMin: number;  opacity: number;  showText: boolean;  showCount: boolean;
  // Timing
  activeStart: string;      // "HH:MM", default "08:00"
  activeEnd: string;        // default "22:00"  (activeStart == activeEnd → always active)
  quietStart: string;       // default "22:00"
  quietEnd: string;         // default "08:00"  (quietStart == quietEnd → never quiet)
  dailyGoal: number;        // 1..30, default 8 — display only ("5 / 8")
  // Behavior
  alwaysOnTop: boolean;     // default true
  clickThrough: boolean;    // default false. Whole bar ignores the mouse except a 20 px grip at the right edge.
  autostart: boolean;       // default false. Rust applies via tauri-plugin-autostart on change.
  hotkeyEnabled: boolean;   // default true
  hotkey: string;           // default "Ctrl+Alt+W" (tauri-plugin-global-shortcut syntax)
  // Alerts
  toastEnabled: boolean;    // default true — native notification on `overdue` and `welcome-back`
  nudgeEveryMin: number;    // 1..60, default 10
  nudgeMax: number;         // 0..10, default 3
  soundEnabled: boolean;    // default false
  soundVolume: number;      // 0..1, default 0.5
}
```

Rust validates all fields (clamp numbers, validate "HH:MM", fall back to default on a bad hotkey and report it in the `set_settings` return value by returning the actual applied settings).

## Click-through mechanics (Rust)

When `clickThrough` is true: `set_ignore_cursor_events(true)`, and a 100 ms poll of the cursor position toggles it to `false` while the cursor is inside the grip rectangle (rightmost 20 logical px of the widget) so the grip stays clickable/draggable. UI shows a subtle grip glyph (⋮⋮) at the right edge in this mode; click on the grip = drink, drag on the grip = move.

## Toast (Rust, tauri-plugin-notification)

Title "Tide", body "Time for water" (overdue) / "Welcome back — drink now?" (welcome-back) / "Timer resumed" (auto-resume). Action buttons are best-effort: if the plugin cannot attach actions on Windows, plain toast is acceptable and the widget click remains the way to log.

## Windows

`settings` window grows to 380×560. Settings page gets four groups: Timing, Look, Behavior, Alerts, plus a footer with **Reset today** and **Quit**.

## Widget visuals for new states (UI)

| State      | Look |
|------------|------|
| paused     | fill desaturated grey (`#9CA3AF`), ⏸ glyph before the text, text "Paused" |
| sleeping   | fill 100 % grey, text "zzz" |
| quiet      | whole pill at 40 % of configured opacity |
| snoozed    | no special look; the fill simply jumps up |
| nudge      | 300 ms horizontal wobble (±3 px, 3 cycles) |
| hover      | ✓ (drink) and ⏸/▶ (pause toggle) mini-buttons fade in at the right edge; click on them must NOT count as a bar click |
| clickThrough | 20 px grip zone with ⋮⋮ glyph at right edge; hover buttons disabled |

Right-click still opens Settings (no custom context menu in v0.2).

---

# v0.3 additions (2026-09-02)

Everything above stays valid. New Settings fields have defaults so old files load.

## Settings (extended)

```ts
interface Settings {
  // ...v0.2 fields unchanged...
  // Look
  layout: 'horizontal' | 'vertical' | 'compact';  // default 'horizontal'
  scale: number;             // 0.75..1.5, default 1.0 — multiplies the window size
  colorPreset: 'default' | 'colorblind' | 'mono';  // default 'default'
  reducedMotion: 'system' | 'on' | 'off';          // default 'system'
}
```

Window size is owned by Rust and applied on startup and whenever layout/scale change:

| layout     | logical size (before scale) | notes |
|------------|-----------------------------|-------|
| horizontal | 220 × 28                    | as today |
| vertical   | 28 × 220                    | fill anchored at the bottom, drains top-down like a glass |
| compact    | 220 × 28                    | window keeps full height; UI draws a 6 px hairline at the bottom edge and expands to the full pill while hovered (+1.5 s after leave) |

Top-left position is preserved when the size changes. The click-through grip is the rightmost 20 px (horizontal/compact) or the bottom 20 px (vertical).

## Color presets (UI only)

| preset     | stops (fill 1.0 → 0.0) | overdue |
|------------|------------------------|---------|
| default    | as MVP table            | `#EF4444` pulse |
| colorblind | `#3B82F6` → `#8B5CF6` (0.5) → `#D946EF` (0.0) | `#D946EF` pulse (hue moves blue→purple→magenta, which stays distinguishable under red-green deficiency; luminance is not monotonic across the whole range, only within each segment) |
| mono       | `#9CA3AF` flat; urgent zone adds a diagonal hatch (repeating-linear-gradient 45°, 4 px) | hatch + pulse |

## Reduced motion (UI only)

`system` → honour `prefers-reduced-motion`; `on` → no pulse/wobble/splash/expand animations; `off` → always animate. Implement as a class on `<html>` (`rm-on` / `rm-off`) so CSS can override the media query.

## Streak and stats

```ts
interface Tick { /* ...*/ streak: number; }   // consecutive days where drinks >= dailyGoal (today excluded until it rolls over)

interface DayStat {
  dayKey: string;        // "YYYY-MM-DD" (04:00 rollover)
  drinks: number;
  avgGapMin: number | null;      // mean minutes between consecutive drinks that day; null if < 2 drinks
  longestOverdueMin: number;     // max(0, gap − interval) over the day's gaps, using the current interval
  goalMet: boolean;
}
interface Stats {
  days: DayStat[];       // last 14 days, oldest first, every day present (zeros if none)
  streak: number;
  bestStreak: number;
  totalDrinks: number;
}
```

- Streak is stored in `state.json` (`streak`, `bestStreak`) and updated at day rollover: `todayCount >= dailyGoal` → `streak += 1`, else `streak = 0`; `bestStreak = max`. On first upgrade to v0.3 (fields missing) Rust rebuilds both from history.jsonl.
- `get_stats` command → `Stats`, computed from history.jsonl (drink events only) at call time. Cheap enough (file is small).
- Settings page gets a **Stats** group: streak / best / total, and a 14-day bar chart (inline SVG, no chart library, bar = drinks, a thin line at dailyGoal, bars ≥ goal in the fresh color, others grey). Refreshes on open and on every `tick` where `todayCount` changed.

## Data commands

| Command         | Args | Returns              | Notes |
|-----------------|------|----------------------|-------|
| `get_stats`     | –    | `Stats`              | |
| `open_data_dir` | –    | `void`               | Opens the data folder in Explorer. |
| `reset_all`     | –    | `Tick`               | Deletes history, resets state (count, streak, timer full), keeps settings. UI must confirm first (native `confirm()` is fine). |

Settings page gets a **Data** group: Open data folder, Reset all (danger). (CSV export was removed after release: Explorer reveal was unreliable and the feature had no real use.) Footer keeps Reset today + Quit.

## History (extended)

Unchanged shape; `reset_all` appends `{type:'reset', source:'ui'}` after truncation so the file is never empty.
