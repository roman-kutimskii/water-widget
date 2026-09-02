// Builds a fresh installer and (with --install) launches it.
//
// `tauri build` runs `npm run build` first (beforeBuildCommand), so dist/ is
// always regenerated from src/ — the installer can never ship a stale bundle.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { createRequire } from "node:module";
import { delimiter, join } from "node:path";

// rustup's bin dir is added to PATH at install time, so any shell opened
// before that (or one started from an app that cached the old environment)
// runs without cargo and the Rust build fails with "program not found".
const exe = process.platform === "win32" ? "cargo.exe" : "cargo";
const onPath = (process.env.PATH ?? "")
  .split(delimiter)
  .some((d) => d && existsSync(join(d, exe)));

if (!onPath) {
  const cargoBin = join(process.env.CARGO_HOME ?? join(homedir(), ".cargo"), "bin");
  if (!existsSync(join(cargoBin, exe))) {
    throw new Error(`cargo not found on PATH or in ${cargoBin} — install Rust via rustup`);
  }
  process.env.PATH = `${process.env.PATH ?? ""}${delimiter}${cargoBin}`;
  console.log(`cargo not on PATH; using ${cargoBin}`);
}

// Call the CLI's JS entry directly: spawning npm.cmd is blocked on Windows
// under Node >= 20 (EINVAL) unless a shell is involved.
const tauri = createRequire(import.meta.url).resolve("@tauri-apps/cli/tauri.js");
execFileSync(process.execPath, [tauri, "build"], { stdio: "inherit" });

const dir = join("src-tauri", "target", "release", "bundle", "nsis");
const installer = readdirSync(dir)
  .filter((f) => f.endsWith(".exe"))
  .map((f) => join(dir, f))
  .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0];

if (!installer) throw new Error(`no installer produced in ${dir}`);
console.log(`\nInstaller: ${installer}`);

if (process.argv.includes("--install")) {
  console.log("Quit Tide from the tray before continuing.");
  execFileSync(installer, { stdio: "inherit" });
}
