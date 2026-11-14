#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const defaults = { darwin: `${process.arch === "arm64" ? "aarch64" : "x86_64"}-apple-darwin`,
  linux: `${process.arch === "arm64" ? "aarch64" : "x86_64"}-unknown-linux-gnu`, win32: "x86_64-pc-windows-msvc" };
const target = process.env.TAURI_ENV_TARGET_TRIPLE || defaults[process.platform];
if (!target || !/^[A-Za-z0-9][A-Za-z0-9_-]{1,99}$/.test(target)) throw Error("Unsupported desktop runtime target");
const candidates = process.platform === "win32" ? [["python", []], ["py", ["-3"]]] : [["python3", []], ["python", []]];
let result;
for (const [python, prefix] of candidates) {
  const args = [...prefix, join(root, "scripts/stage-decider-runtime.py"), "--stage", "--build", "--target", target];
  if (process.env.CI === "true" && process.env.GITHUB_ACTIONS === "true") args.push("--fetch-source");
  result = spawnSync(python, args, { cwd: root, stdio: "inherit", env: process.env });
  if (result.error?.code === "ENOENT") continue;
  if (result.error) throw Error("The pinned runtime staging command could not start");
  process.exit(result.status ?? 1);
}
throw Error("Python 3 is required to prepare desktop runtime resources");
