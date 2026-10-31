#!/usr/bin/env node
// Downloads Piper voice models (.onnx + .onnx.json) from the
// rhasspy/piper-voices collection into engine/voices/. Each voice model
// carries its own license from that project — check before redistributing.
//
// Usage: node scripts/fetch-voices.mjs [lang/name/quality ...]
//   e.g. node scripts/fetch-voices.mjs es_AR/daniela/high en_US/amy/high
// Without arguments, installs the default set below.
import { createWriteStream } from "node:fs";
import { mkdirSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { pipeline } from "node:stream/promises";

const DEFAULT_VOICES = ["es_AR/daniela/high", "en_US/amy/high", "en_GB/alba/medium"];
const BASE = "https://huggingface.co/rhasspy/piper-voices/resolve/main";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = resolve(root, "engine/voices");

const specs = process.argv.slice(2).filter((a) => !a.startsWith("--"));
const voices = specs.length > 0 ? specs : DEFAULT_VOICES;
mkdirSync(outDir, { recursive: true });

for (const spec of voices) {
  const [lang, name, quality] = spec.split("/");
  if (!lang || !name || !quality) {
    console.error(`skipping malformed spec "${spec}" (want lang/name/quality)`);
    continue;
  }
  const stem = `${lang}-${name}-${quality}`;
  for (const ext of ["onnx", "onnx.json"]) {
    const url = `${BASE}/${lang}/${name}/${quality}/${stem}.${ext}`;
    const dest = resolve(outDir, `${stem}.${ext}`);
    console.log(`fetching ${stem}.${ext}`);
    const response = await fetch(url);
    if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
    await pipeline(response.body, createWriteStream(dest));
  }
  console.log(`installed ${stem}`);
}
