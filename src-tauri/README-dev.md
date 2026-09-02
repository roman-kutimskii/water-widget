# Tide — backend dev notes

## Prerequisites

- Rust (stable) + the MSVC toolchain (`rustup default stable-x86_64-pc-windows-msvc`)
- Visual Studio Build Tools with the "Desktop development with C++" workload
- Node 18+ / npm
- WebView2 runtime (preinstalled on Windows 11)

## Run

```powershell
npm install          # once, at the project root
npm run tauri dev    # starts Vite on :1420 and the Tauri shell
```

`npm run tauri dev` builds the Rust crate, launches Vite (`beforeDevCommand`), and
opens the `widget` window. The `settings` window is created lazily by the
`open_settings` command; closing it hides it instead of destroying it.

Production build:

```powershell
npm run tauri build
```

## Tests

```powershell
cd src-tauri
cargo test           # engine + store unit tests
cargo check
cargo clippy --all-targets
```

`store` tests write to a per-process directory under `%TEMP%\tide-test-*` and
clean up after themselves.

## Data files

Everything lives in the Tauri `app_config_dir`, which on Windows is
`%APPDATA%\dev.kutimskii.tide\` (i.e. `C:\Users\<you>\AppData\Roaming\dev.kutimskii.tide\`).

| File            | Contents |
|-----------------|----------|
| `settings.json` | MVP fields, the v0.2 ones (active/quiet hours, dailyGoal, alwaysOnTop, clickThrough, autostart, hotkey*, toastEnabled, nudge*, sound*) and the v0.3 Look ones (`layout`, `scale`, `colorPreset`, `reducedMotion`) |
| `state.json`    | `{ version, lastDrinkTs, todayCount, dayKey, position, pausedAccumMs, pausedSince, snoozeMs, lastMode, streak, bestStreak }` |
| `history.jsonl` | one `{ ts, type: drink|snooze|pause|resume|reset, source, minutes? }` per line |

Files written by the MVP still load: every v0.2 and v0.3 field has a serde
default. `STATE_VERSION` is **2** since v0.3; a state file still at version 1
has no streak data, so the streak and the best streak are rebuilt once from
`history.jsonl` on startup and the file is rewritten at version 2. An unknown
value for one of the three Look enums (say `"layout": "diagonal"`) falls back to
that field's default instead of invalidating the whole file.

Missing or corrupt files fall back to defaults; writes are atomic (temp + rename).
Deleting the directory resets the app to first-launch behaviour.

## Layout

- `src/engine.rs` — all timing maths, pure and unit-tested: `TimerState`
  (elapsed / pausedAccum / snooze / sleeping), `compute_tick_full`,
  `zone_for_fill`, `parse_hhmm` + `is_active_at` / `is_quiet_at`
  (midnight-crossing windows), `NudgeState` (overdue once, repeats capped,
  quiet suppression, 5 min welcome-back grace), `away_outcome`, `day_key`
  (04:00 rollover), `should_merge_drink` (60 s). No IO, no Tauri.
- `src/store.rs` — settings/state/history persistence, clamping and validation
  (HH:MM strings, hotkey shape, `scale` clamped to 0.75..1.5), the `Layout` /
  `ColorPreset` / `ReducedMotion` enums, `load_history` / `truncate_history`.
- `src/stats.rs` — pure and unit-tested: `rolled_over_streak` (the day-rollover
  rule), `rebuild_streaks` (recover both counters from history) and
  `compute_stats` (the 14-day summary). No IO, no Tauri.
- `src/platform.rs` — the only Win32 code: `session_locked`
  (`OpenInputDesktop` + `GetUserObjectInformationW`), `notifications_suppressed`
  (`SHQueryUserNotificationState`), `cursor_position` (`GetCursorPos`). Each has
  a no-op fallback on non-Windows targets.
- `src/lib.rs` — Tauri wiring only: commands, the 1 Hz tick loop that drives all
  automatic transitions, the 2 s platform poll, the 100 ms click-through poll,
  tray, windows, hotkey/autostart/always-on-top application, plus the v0.3
  window-size owner (`widget_size` / `apply_widget_size`) and the CSV export.
- `capabilities/default.json` — permissions for both windows (includes
  `core:window:allow-start-dragging` for `data-tauri-drag-region` and the
  `notification` / `global-shortcut` / `autostart` plugin defaults).

## Logging

`env_logger`, default level `info`. Override with `RUST_LOG=debug npm run tauri dev`.

## v0.2 features and how to test them by hand

Set `RUST_LOG=debug` and watch the console; every transition below logs a line.

| Feature | How to exercise it |
|---|---|
| **Snooze** | Tray → *Snooze 10 min*, or `snooze` from the UI. The bar jumps up, `lastDrinkTs` does not move, and `history.jsonl` gets `{"type":"snooze","minutes":10}`. Drinking clears the accumulated snooze. |
| **Pause / resume** | Tray → *Pause* (the label flips to *Resume*). The bar freezes; on resume the frozen span lands in `pausedAccumMs`. |
| **Auto-resume** | Pause and wait 120 min, or temporarily lower `AUTO_RESUME_MS` in `engine.rs`. Expect a `nudge` event of kind `auto-resume` plus a toast, even during quiet hours. |
| **Active hours** | Set `activeStart`/`activeEnd` in Settings to a window that has just ended: within a second the tick reports `mode: "sleeping"` and the timer freezes. Set them so the window starts again and the bar resets to full without incrementing the count. Setting the two equal means "always active". Windows may cross midnight (e.g. 22:00 → 06:00). |
| **Quiet hours / DND / full-screen** | Put the clock inside `quietStart`–`quietEnd`, or turn on Focus Assist, or start a full-screen game/video. Within 2 s `tick.quiet` becomes true and nudges/toasts stop (ambient only). |
| **Nudges** | Set `intervalMin` to 10, `nudgeEveryMin` to 1, `nudgeMax` to 3 and wait: one `overdue` nudge at fill 0, then at most 3 `repeat` nudges one minute apart. |
| **Session lock** | Win+L, wait, unlock. Away ≥ the interval → a `welcome-back` nudge + toast and no repeats for 5 min. Away ≥ 4 h → the bar comes back full with the day's count untouched. |
| **System sleep** | Suspend the machine and resume. The tick loop notices the wall-clock gap (> 60 s) and applies the same away rules. |
| **Global hotkey** | Default `Ctrl+Alt+W` logs a drink with source `hotkey`. Change it in Settings; the new accelerator is registered immediately. An accelerator the OS rejects (already taken) falls back to `Ctrl+Alt+W`, logs a warning, and `set_settings` returns the accelerator that was actually applied — check the returned value, not what you typed. |
| **Autostart** | Toggle *Start with Windows*; verify `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` gains/loses the `Tide` value. The setting is the source of truth and is reconciled on every startup. |
| **Always on top** | Toggle it and raise another window over the widget. |
| **Click-through** | Turn it on: clicks pass through the bar to whatever is behind it, except the rightmost 20 logical px, where a 100 ms cursor poll re-enables input so the grip stays clickable and draggable. Turning it off stops the poll thread (a generation counter makes sure an old thread never fights the new state). |
| **Toasts** | With `toastEnabled`, `overdue` / `welcome-back` / `auto-resume` show a native "Tide" notification. Repeat nudges are event-only. Notifications require the app to be installed (`npm run tauri build`) on some Windows configurations; in `dev` they may be silently dropped. |

Do not run two instances: `tauri-plugin-single-instance` focuses the existing
widget instead.

## v0.3 features and how to test them by hand

### Window size is owned by Rust

`widget_size(layout, scale)` is the single source of truth:

| layout | logical size (before `scale`) |
|---|---|
| `horizontal` | 220 × 28 |
| `compact` | 220 × 28 (the window keeps the full pill; the UI draws the hairline) |
| `vertical` | 28 × 220 |

`scale` multiplies both axes and is clamped to 0.75..1.5. The size is applied on
startup (`place_widget`) and from `set_settings` whenever `layout` or `scale`
changes; `apply_widget_size` restores the outer top-left corner afterwards, so
the bar grows to the right/down instead of wandering. The default bottom-right
placement measures the real window, so a vertical or scaled widget still lands
24 px off the work-area corner.

After every resize (startup and `set_settings` alike) the widget is clamped back
into the work area of the monitor holding its top-left corner, falling back to
the primary monitor: growing from `horizontal` to a scaled `vertical` at the
bottom-right corner would otherwise leave the taller window hanging below the
screen. The rect math is the pure, tested `clamp_rect_into(area, pos, size)` —
no margin, and a window bigger than the work area is aligned to its top-left
corner. A clamped position is written straight back to `state.json`.

The click-through grip follows the layout: the rightmost `20 × scale` logical px
for `horizontal` / `compact`, the bottom `20 × scale` for `vertical`. The 100 ms
poll reads the current layout and scale from settings, so changing either while
click-through is on takes effect immediately.

### Streak semantics

- A streak counts **completed** days whose drink count reached `dailyGoal`.
  Today is never included: it only counts once the 04:00 rollover happens.
- At the rollover the day that just ended is folded in: `todayCount >= dailyGoal`
  → `streak += 1`, otherwise `streak = 0`. `bestStreak` is the running maximum
  and is never lowered — `reset_all` keeps it.
- A day with no drinks at all breaks the streak, including days the app was not
  running: the rebuild walks every calendar day from the first recorded drink.
- `streak` rides on every `tick`; `streak` / `bestStreak` also come back from
  `get_stats`, recomputed from history so the view cannot drift from the file.
- To exercise it, set `dailyGoal` to 1, log a drink, then move the system clock
  past the next 04:00: the tick loop logs `day rollover … streak 0 -> 1`.

### New commands

| Command | Returns | Notes |
|---|---|---|
| `get_stats` | `Stats` | 14 days, oldest first, empty days zeroed. `avgGapMin` is `null` below two drinks that day; `longestOverdueMin` is `max(0, gap − interval)` over that day's gaps only, so the overnight span never counts. `totalDrinks` spans the whole file, not the window. |
| `open_data_dir` | – | `explorer <data dir>`; best effort. |
| `reset_all` | `Tick` | Truncates `history.jsonl` and appends one `{"type":"reset","source":"ui"}` line so the file is never empty; `todayCount` and `streak` go to 0, the timer goes back to full, the nudge schedule is cleared. `bestStreak`, settings and the widget position survive. The UI confirms first. |

Malformed lines in `history.jsonl` are skipped with a warning rather than
failing a stats call or an export.

## Release build

```
npm run tauri build
```

Produces `src-tauri/target/release/bundle/nsis/Tide_<version>_x64-setup.exe` (per-user NSIS installer, no admin needed; `/S` for silent). It installs to `%LOCALAPPDATA%\Tide\tide.exe`. Autostart re-registers on every launch when the setting is on, so the Run key always follows the executable that is actually running (dev build vs. installed).
