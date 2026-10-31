#!/usr/bin/env node
// Rasterizes assets/icon.svg to a 1024px PNG that `tauri icon` turns into
// the full icon set (icns, ico, PNGs). Run: npm run icon
import sharp from "sharp";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const svg = resolve(root, "assets/icon.svg");
const png = resolve(root, "assets/app-icon.png");

await sharp(svg, { density: 300 })
  .resize(1024, 1024)
  .png()
  .toFile(png);

console.log(`wrote ${png}`);
