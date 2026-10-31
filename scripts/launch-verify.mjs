#!/usr/bin/env node
// Launches an ISOLATED Bookshelf PDF instance for end-to-end verification.
//
// Isolation: the app identifier is overridden via TAURI_CONFIG, so this
// instance gets its own single-instance namespace (it will never forward
// files into a copy of the app someone is actively reading in) and its own
// app-data directory (its own database — it can't touch real reading
// progress). Only the PID recorded here is ever stopped, never a blanket
// process-name kill.
//
// Usage:
//   node scripts/launch-verify.mjs <pdf-path> [debug-port]
//   node scripts/launch-verify.mjs --stop
import { spawn } from "node:child_process";
import { existsSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const pidFile = resolve(root, "target", "verify-instance.pid");

if (process.argv[2] === "--stop") {
  if (existsSync(pidFile)) {
    const pid = Number(readFileSync(pidFile, "utf8").trim());
    if (Number.isFinite(pid)) {
      try {
        process.kill(pid);
        console.log(`stopped verify instance ${pid}`);
      } catch {
        console.log("verify instance already exited");
      }
    }
    writeFileSync(pidFile, "");
  } else {
    console.log("no verify instance recorded");
  }
  process.exit(0);
}

const pdf = process.argv[2];
const port = process.argv[3] ?? "9230";
if (!pdf || !existsSync(resolve(pdf))) {
  console.error("usage: node scripts/launch-verify.mjs <pdf-path> [debug-port]");
  process.exit(1);
}

const config = {
  identifier: "com.bookshelfpdf.app.verify",
  productName: "Bookshelf PDF (verify)",
};

const child = spawn(
  resolve(root, "src-tauri/target/debug/bookshelf-pdf.exe"),
  [resolve(pdf)],
  {
    detached: true,
    stdio: "ignore",
    env: {
      ...process.env,
      // Runtime isolation, enforced by the app itself: no single-instance
      // forwarding, no window-state writes, separate database. (The config
      // override above is belt-and-braces; the env var is what the app reads.)
      BOOKSHELF_VERIFY_INSTANCE: "1",
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
      TAURI_CONFIG: JSON.stringify(config),
    },
  },
);
child.unref();
mkdirSync(resolve(root, "target"), { recursive: true });
writeFileSync(pidFile, String(child.pid));
console.log(`verify instance pid=${child.pid} cdp=http://127.0.0.1:${port}/json`);
