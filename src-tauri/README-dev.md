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
| `settings.json` | `{ version, intervalMin, opacity, showText, showCount }` |
| `state.json`    | `{ version, lastDrinkTs, todayCount, dayKey, position }` |
| `history.jsonl` | one `{ ts, type: "drink", source }` per line |

Missing or corrupt files fall back to defaults; writes are atomic (temp + rename).
Deleting the directory resets the app to first-launch behaviour.

## Layout

- `src/engine.rs` — pure timer maths: `compute_tick`, `zone_for_fill`, `day_key`
  (04:00 rollover), `should_merge_drink` (60 s). No IO, fully unit-tested.
- `src/store.rs` — settings/state/history persistence + clamping.
- `src/lib.rs` — Tauri wiring: commands, 1 Hz tick loop, tray, windows.
- `capabilities/default.json` — permissions for both windows (includes
  `core:window:allow-start-dragging` for `data-tauri-drag-region`).

## Logging

`env_logger`, default level `info`. Override with `RUST_LOG=debug npm run tauri dev`.
