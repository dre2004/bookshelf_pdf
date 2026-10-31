#!/usr/bin/env node
// Launches a verification instance that reproduces the FRESH-INSTALL
// experience: cwd is a temp dir, so the repo's engine/voices is not
// visible and only app-data/voices (normally empty) can provide voices.
// Use with scripts/verify-narration.mjs to exercise the voice downloader.
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync, existsSync, rmSync, openSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const pdf = process.argv[2] ?? "C:\\Users\\Barre\\Desktop\\Black-Hand-Gang-obooko-tya0157.pdf";
const port = process.argv[3] ?? "9238";
const pidFile = resolve(root, "src-tauri/target/verify-instance.pid");

if (process.argv[2] === "--stop") {
  if (existsSync(pidFile)) {
    const pid = Number(readFileSync(pidFile, "utf8").trim());
    try {
      process.kill(pid);
      console.log(`stopped verify instance ${pid}`);
    } catch {
      console.log("verify instance already exited");
    }
    writeFileSync(pidFile, "");
  }
  process.exit(0);
}

const cwd = resolve(tmpdir(), "bs-verify-cwd");
mkdirSync(cwd, { recursive: true });

const logFile = openSync(resolve(root, "src-tauri/target/verify-instance.log"), "w");
const child = spawn(resolve(root, "src-tauri/target/debug/bookshelf-pdf.exe"), [pdf], {
  detached: true,
  stdio: ["ignore", logFile, logFile],
  cwd,
  env: {
    ...process.env,
    BOOKSHELF_VERIFY_INSTANCE: "1",
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
  },
});
child.unref();
mkdirSync(resolve(root, "src-tauri/target"), { recursive: true });
writeFileSync(pidFile, String(child.pid));
console.log(`fresh-install verify instance pid=${child.pid} cwd=${cwd} cdp=http://127.0.0.1:${port}/json`);
