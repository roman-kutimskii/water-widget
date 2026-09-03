// Bumps the app version in every file that carries it, so a release never
// ships with mismatched versions.
//
//   npm run bump -- 0.3.0        set an explicit version
//   npm run bump -- patch        0.2.0 -> 0.2.1 (also: minor, major)
//   npm run bump -- minor --tag  bump, commit, and create the v* tag
//
// Push the tag (`git push origin main vX.Y.Z`) to trigger the release build.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";

const [arg, ...flags] = process.argv.slice(2);
if (!arg) {
  console.error("usage: npm run bump -- <version|patch|minor|major> [--tag]");
  process.exit(1);
}

const pkgPath = "package.json";
const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));
const current = pkg.version;

let next;
if (/^\d+\.\d+\.\d+$/.test(arg)) {
  next = arg;
} else if (["major", "minor", "patch"].includes(arg)) {
  const [ma, mi, pa] = current.split(".").map(Number);
  next =
    arg === "major" ? `${ma + 1}.0.0` :
    arg === "minor" ? `${ma}.${mi + 1}.0` :
                      `${ma}.${mi}.${pa + 1}`;
} else {
  console.error(`invalid version or bump type: ${arg}`);
  process.exit(1);
}

// Each entry: file, and a replacer that swaps only the app's own version
// (not a dependency that happens to share the number).
const targets = [
  {
    file: "package.json",
    edit: (s) => s.replace(/("version":\s*")[^"]+(")/, `$1${next}$2`),
  },
  {
    file: "package-lock.json",
    edit: (s) => {
      const lock = JSON.parse(s);
      lock.version = next;
      lock.packages[""].version = next;
      return JSON.stringify(lock, null, 2) + "\n";
    },
  },
  {
    file: "src-tauri/tauri.conf.json",
    edit: (s) => s.replace(/("version":\s*")[^"]+(")/, `$1${next}$2`),
  },
  {
    file: "src-tauri/Cargo.toml",
    edit: (s) => s.replace(/^(version\s*=\s*")[^"]+(")/m, `$1${next}$2`),
  },
  {
    file: "src-tauri/Cargo.lock",
    edit: (s) =>
      s.replace(
        /(\[\[package\]\]\nname = "tide"\nversion = ")[^"]+(")/,
        `$1${next}$2`,
      ),
  },
];

for (const { file, edit } of targets) {
  const before = readFileSync(file, "utf8");
  const after = edit(before);
  if (after === before) throw new Error(`no version field updated in ${file}`);
  writeFileSync(file, after);
  console.log(`${file}: ${current} -> ${next}`);
}

if (flags.includes("--tag")) {
  const git = (...a) => execFileSync("git", a, { stdio: "inherit" });
  git("add", ...targets.map((t) => t.file));
  git("commit", "-m", `Bump version to ${next}`);
  git("tag", `v${next}`);
  console.log(`\nTagged v${next}. Push with: git push origin main v${next}`);
} else {
  console.log(`\nNext: git commit -am "Bump version to ${next}" && git tag v${next}`);
}
