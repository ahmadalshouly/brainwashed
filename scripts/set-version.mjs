#!/usr/bin/env node
// Sets the release version everywhere it appears:
//   node scripts/set-version.mjs 0.1.0
import { readFileSync, writeFileSync } from "node:fs";

const version = process.argv[2];
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version ?? "")) {
  console.error("usage: node scripts/set-version.mjs <major.minor.patch>");
  process.exit(1);
}

const edit = (path, fn) => {
  const before = readFileSync(path, "utf8");
  const after = fn(before);
  if (after === before) throw new Error(`${path}: version not found`);
  writeFileSync(path, after);
  console.log(`${path} -> ${version}`);
};

for (const path of [
  "apps/host/package.json",
  "apps/web/package.json",
  "packages/api/package.json",
  "apps/host/src-tauri/tauri.conf.json",
]) {
  edit(path, (s) => s.replace(/("version":\s*")[^"]+(")/, `$1${version}$2`));
}
for (const path of ["apps/host/src-tauri/Cargo.toml", "crates/relay/Cargo.toml"]) {
  edit(path, (s) => s.replace(/^version = "[^"]+"/m, `version = "${version}"`));
}
console.log("Now run `cargo check` to refresh Cargo.lock, commit, and tag v" + version + ".");
