# Tide — Desktop Water Reminder Widget

**Status:** spec v0.2, decisions resolved · MVP and v0.2 roadmap items implemented 2026-09-02 (see §13; IPC details in CONTRACT.md)
**Platform:** Windows 11 first (macOS/Linux later)

---

## 1. One-line summary

A tiny always-visible desktop widget shaped like a **reverse progress bar**: a bar that starts full right after you drink, slowly *drains* over time, shifts color from calm blue to alarming red, and nudges you to drink again. One click refills it.

---

## 2. Problem and goals

**Problem.** People forget to drink water while working. Calendar/phone reminders are easy to dismiss and interrupt flow; a passive, ambient cue works better than a pop-up.

**Goals**
- Ambient, glanceable: the state of the bar tells you "how overdue" you are without reading anything.
- Zero-friction logging: one click or one hotkey = "I drank".
- Escalation that is noticeable but never blocking.
- Light on resources (it lives on screen all day): < 50 MB RAM, ~0 % CPU when idle.
- Respect the user's context: quiet hours, sleep/lock, meetings, full-screen apps.

**Non-goals (v1)**
- No hydration science / body-weight calculators. Fixed user-chosen interval.
- No accounts, cloud sync, or mobile companion.
- No gamification beyond a simple daily count and streak.

---

## 3. Core concept: the reverse progress bar

The bar represents **time remaining until you should drink**, not water consumed.

```
Just drank                       Halfway                          Interval elapsed
|████████████████████| 100 %  →  |██████████░░░░░░░░░░| 50 %  →  |░░░░░░░░░░░░░░░░░░░░| 0 % (overdue)
        blue                          yellow                        red, pulsing
```

- `fill = clamp(1 - elapsed / interval, 0, 1)`
- `elapsed` = wall-clock time since the last "drank" event, minus paused time (see §7).
- Bar drains **continuously** (state updated every 1 s, animated smoothly), not in steps.

### 3.1 Color model

Color is a function of fill, interpolated in a perceptual space (OKLCH) so it does not pass through muddy tones.

| Fill          | Zone    | Color (light theme)                  | Meaning       |
|---------------|---------|--------------------------------------|---------------|
| 100 – 60 %    | Fresh   | `#3B82F6` blue                       | You are fine  |
| 60 – 30 %     | Fading  | `#22C55E` green → `#EAB308` yellow   | Getting there |
| 30 – 0 %      | Urgent  | `#F97316` orange → `#EF4444` red     | Drink soon    |
| 0 % (overdue) | Overdue | `#EF4444` red, slow pulse (1.5 s)    | You missed it |

Thresholds are constants in config, editable in Settings → Advanced. Provide a **colorblind-safe** preset (blue → purple → magenta) and a **monochrome** preset where urgency is shown by hatching and pulse instead of hue.

### 3.2 Overdue behavior

When fill hits 0 % the bar does not just sit empty:
1. Bar background pulses red (opacity 60 → 100 %).
2. Small overdue counter appears inside the bar: `+12 min`.
3. Optional escalation (see §6).
4. The bar never auto-refills. Only a user action resets it.

---

## 4. Visual design

### 4.1 Form factor

- Frameless, transparent, always-on-top window. Default size **220 × 28 px**: a thin pill.
- Two layouts, switchable:
  - **Horizontal pill** (default): drains right-to-left.
  - **Vertical tube**: tall thin bar (28 × 220), drains top-to-bottom, reads like a glass emptying.
- Rounded corners (14 px radius), 1 px semi-transparent border for contrast on any wallpaper.
- Optional water-droplet icon at the leading edge; icon tilts/empties with the bar (nice-to-have).

### 4.2 Content inside the bar

Left-aligned, single line, 12 px system font, auto-contrast text (white on dark fills, dark on light fills):
- Fresh/Fading: remaining time `42 min`
- Urgent: `8 min`, bold
- Overdue: `+12 min`
- Optional secondary: today's count `· 5 / 8`

**Hover** reveals a tooltip: "Last drink 14:05 · Next 14:50 · Today 5 glasses".

### 4.3 States

| State       | Look |
|-------------|------|
| Normal      | Bar as described |
| Hover       | Slight lift (shadow), reveals small ✓ (drink) and ⏸ (snooze) buttons at the trailing edge |
| Paused      | Bar desaturated to grey, ⏸ glyph, fill frozen |
| Quiet hours | Bar at 40 % opacity, no escalation, still draining |
| Compact     | Bar collapsed to a 6 px hairline showing only color. Expands on hover. |

### 4.4 Theming

- Follows OS light/dark. Backgrounds: dark `rgba(20,20,25,0.75)`, light `rgba(255,255,255,0.8)` with backdrop blur (Mica/Acrylic on Windows 11).
- Opacity slider 30–100 %.
- **Click-through** mode: window ignores the mouse except for a small grip handle, so it can sit over apps without stealing clicks.

---

## 5. Interaction

| Action | Effect |
|--------|--------|
| **Left click** on bar | Log a drink: bar refills to 100 % with a 300 ms "splash" animation, count +1 |
| **Right click** | Context menu: Drink, Snooze 5/10/15 min, Pause, Layout, Settings, Quit |
| **Drag** the bar (or grip) | Move widget; position remembered per monitor |
| **Global hotkey** (default `Ctrl+Alt+W`) | Log a drink without touching the mouse |
| **Double click** | Open Settings |
| **Scroll wheel** over bar | Adjust today's interval ±5 min (optional, off by default) |
| Undo | After a click, a 5-second toast "Logged · Undo" for misclicks |

**Snooze** adds N minutes to the deadline *without* refilling: the bar jumps up a little and keeps draining.

---

## 6. Notifications and escalation

Layered, each individually toggleable:

1. **Ambient (always on):** color + pulse on the widget itself.
2. **Toast at 0 %:** native Windows notification "Time for water" with action buttons *Drank* / *Snooze 10 min*.
3. **Gentle nudge every N min while overdue** (default 10 min, max 3 repeats): widget does a brief wobble; optional built-in water-drop sound (~300 ms, off by default, respects system mute).
4. **Escalated (off by default):** after X min overdue, widget grows 1.5× or briefly slides to screen center and returns.

Suppression rules:
- Quiet hours (default 22:00–08:00): ambient only.
- Focus Assist / Do Not Disturb on: ambient only.
- Full-screen app detected (game, video, presentation): ambient only, widget optionally hides.
- Session locked or system asleep: no toasts; on resume see §7.

---

## 7. Time and lifecycle rules

- **Interval** default **45 min**; range 10–180 min.
- **Daily goal** default 8 drinks (display only; does not affect the bar).
- **Active hours** default 08:00–22:00. Outside them the bar shows "zzz" and does not count time; the bar is full at the start of active hours.
- **Sleep / lock:** while the session is locked or the PC sleeps, elapsed time still accrues (you did not drink while away), but no escalation fires. On resume:
  - away < interval: continue normally.
  - away ≥ interval: show overdue state with a **grace toast** "Welcome back, drink now?" and no repeated nudges for 5 min.
  - away ≥ 4 h: treat as a new session, reset bar to full, do not count overdue.
- **Pause:** manual; freezes elapsed. Auto-unpause after 2 h with a toast.
- **Day rollover** at 04:00 local: reset today's count, compute streak (goal met → streak +1).
- **Clock changes / timezone jumps:** use monotonic time for elapsed within a session; on a system-clock jump > 5 min, re-anchor without triggering overdue.

---

## 8. Settings (single window, five groups)

- **Timing:** interval, active hours, quiet hours, daily goal.
- **Look:** layout (horizontal / vertical / compact), size, opacity, theme (auto / light / dark), color preset, show text, show count.
- **Behavior:** always on top, click-through, start with Windows, position lock, hide on full-screen.
- **Alerts:** toast on/off, repeat nudges (interval, max), sound + volume, escalated mode.
- **Data:** export history (CSV), reset today, reset all.

Settings changes preview live on the widget.

---

## 9. Data

Local only, under `%APPDATA%/dev.kutimskii.tide/` (Tauri names the folder after the app identifier):
- `settings.json`: schema-versioned.
- `history.jsonl` (SQLite later): one row per event `{ts, type: drink|snooze|pause|resume|reset, source: click|hotkey|toast|auto}`.
- `state.json`: `lastDrinkTs`, `pausedAccumMs`, `snoozeUntil`, `todayCount`, `streak`, window position per monitor ID.

On launch, recompute fill from `lastDrinkTs` so a crash or restart never loses the timer.

v2: weekly stats page (drinks per day, average gap, longest overdue).

---

## 10. Technical design

### 10.1 Stack (recommended)

**Tauri 2 + Rust backend + TypeScript/Svelte frontend.**
Rationale: transparent frameless always-on-top windows, tray, global hotkeys, autostart and notifications are all built-in plugins; binary ~10 MB, RAM ~30–40 MB. Smooth CSS animation for the bar is trivial in a webview.

Alternatives considered:
- *Electron*: simplest, but 150 MB+ RAM for a 220 px bar. Rejected.
- *WinUI 3 / WPF (C#)*: best native look on Windows, but Windows-only. Viable if cross-platform is dropped.
- *Python + PyQt*: quick prototype only.

### 10.2 Architecture

```
+---------------- Tauri app ----------------+
| Rust core                                 |
|  - Timer engine (monotonic + wall clock)  |
|  - State store (settings/state/history)   |
|  - OS hooks: lock/sleep/resume, DND,      |
|    full-screen detection, hotkey, tray    |
|  - Notifications                          |
|         ^ events / commands (IPC)         |
| Webview UI                                |
|  - Widget window (bar, animations)        |
|  - Settings window                        |
+-------------------------------------------+
```

- Core emits `tick {fill, zone, remainingMs, overdueMs, todayCount}` every 1 s; the UI is a pure render of that payload. All timing logic lives in Rust so the UI can be swapped.
- Two windows: `widget` (frameless, transparent, no taskbar entry, always-on-top) and `settings` (normal, lazily created).
- Tray icon mirrors the zone color; tray menu = right-click menu.

### 10.3 Windows platform details

- Always-on-top: `HWND_TOPMOST`; re-assert on `WM_DISPLAYCHANGE`.
- Click-through: `WS_EX_TRANSPARENT | WS_EX_LAYERED`, toggled at runtime.
- Session events: `WTSRegisterSessionNotification` (lock/unlock), `WM_POWERBROADCAST` (sleep/resume).
- Full-screen and Focus Assist detection: `SHQueryUserNotificationState`.
- Multi-monitor: store position relative to monitor; if the monitor disappears, snap to primary bottom-right.
- Autostart via the Tauri autostart plugin (registry `Run` key).

### 10.4 Performance budget

- Idle CPU < 0.5 % (1 Hz tick; CSS transitions only on value change).
- RAM < 60 MB with settings window closed.
- Cold start < 1 s.

---

## 11. Accessibility

- Text label always available (never color-only); accessible name "Water reminder, 42 minutes remaining".
- Keyboard: hotkey to drink; settings fully keyboard navigable.
- Colorblind and monochrome presets; minimum 4.5:1 contrast for in-bar text.
- Reduced-motion setting: no pulse/wobble, static border instead.

---

## 12. Edge cases checklist

- [ ] Launch when already overdue from the previous session (crash recovery).
- [ ] User clicks "drink" 3× in 10 s: merge clicks within 10 s of the last counted drink, count once (never merged against a reset).
- [ ] Interval changed while draining: recompute from the same `lastDrinkTs`.
- [ ] Widget dragged off-screen / monitor unplugged.
- [ ] Two instances launched: single-instance lock, focus the existing one.
- [ ] System clock set back an hour.
- [ ] Notification permission denied: ambient only, note in settings.
- [ ] Widget covered by a full-screen app, then the app closes: widget reappears.
- [ ] High-DPI / mixed-DPI monitors: bar stays crisp, hit targets ≥ 24 px.

---

## 13. Roadmap

**MVP (v0.1)**, 1–2 weeks
- Horizontal bar, drain + color, click to drink, drag to move, tray with Quit.
- Interval and opacity in a minimal settings window.
- Persist state across restarts.

**v0.2**
- Toast at 0 %, snooze, global hotkey, pause, active/quiet hours.
- Lock/sleep handling, autostart, click-through.

**v0.3**
- Vertical and compact layouts, color presets, reduced motion.
- History, simple stats, streak.

**Later ideas**
- Multiple bars for other habits (stand up, eye rest) on the same engine.
- macOS/Linux builds.
- Drink size (cup/bottle) to show volume instead of count.
- Adaptive interval: shorten by 10 % when behind the daily goal after 14:00 (see decision #2).

---

## 14. Decisions log

| # | Question | Decision | Rationale |
|---|----------|----------|-----------|
| 1 | Accrue elapsed time during lock/sleep, or freeze? | **Accrue**, with the grace rules in §7 (grace toast, no nudges for 5 min, full reset after ≥ 4 h away). | The widget is a reminder, not a tracker. Your body did not pause while you were away, so the bar should reflect real time; the grace rules prevent an angry red bar the moment you unlock. |
| 2 | Fixed interval vs. adaptive interval? | **Fixed** interval for v1. Adaptive mode (shorten by 10 % when behind daily goal after 14:00) goes to the "Later ideas" list. | Predictability makes the bar readable at a glance. Adaptive behavior is a surprise the user has to learn. |
| 3 | Ship a default sound? | **Ship one built-in sound, off by default.** A single soft water-drop (~300 ms, -12 dB). Enabled via Settings → Alerts. | An ambient widget should not make noise unasked. Bundling the sound means turning it on is one toggle, not a file hunt. |
| 4 | Name | **Tide.** App ID `dev.kutimskii.tide`, tray tooltip "Tide", data folder `%APPDATA%/dev.kutimskii.tide/` (Tauri names the folder after the app identifier). | Short, evokes a water level falling and rising, no obvious conflicts among desktop apps. |

---

## 15. Acceptance criteria (MVP)

- Bar visibly drains from full to empty over the configured interval and turns from blue to red.
- One left click refills the bar and increments today's count within 100 ms.
- Widget stays on top of normal windows and remembers its position after restart.
- Restarting the app mid-interval shows the correct remaining time (±2 s).
- Idle resource usage within the budget in §10.4.
