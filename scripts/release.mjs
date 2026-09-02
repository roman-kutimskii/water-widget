// Builds a fresh installer and (with --install) launches it.
//
// `tauri build` runs `npm run build` first (beforeBuildCommand), so dist/ is
// always regenerated from src/ — the installer can never ship a stale bundle.
import { execFileSync } from "node:child_process";
import { readdirSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";

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
