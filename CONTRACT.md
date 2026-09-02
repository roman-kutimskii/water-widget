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
| `drink`          | `{ source: 'click'\|'hotkey'\|'toast' }` | `Tick`      | Resets `lastDrinkTs = now`, `todayCount += 1`. Events within 60 s of the previous drink are merged: state updates but count does not increment. Appends to history. Emits `tick`. |
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

Icon + menu: **Drink now**, **Settings…**, separator, **Quit**. Left-click on tray icon = Drink now.
