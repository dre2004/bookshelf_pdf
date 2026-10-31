#!/usr/bin/env node
// Exercises the fresh-install voice-download path over CDP: opens the
// narration panel, asserts no on-device voices, clicks Download, waits for
// the on-device voices to appear in the list, then asserts Daniela is the
// default. Usage: node scripts/verify-download.mjs [port]
const port = process.argv[2] ?? "9243";

const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === "page");
if (!page) throw new Error("no page target");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });

let nextId = 1;
const pending = new Map();
ws.onmessage = (event) => {
  const message = JSON.parse(event.data);
  if (message.id && pending.has(message.id)) {
    pending.get(message.id)(message.result?.result?.value);
    pending.delete(message.id);
  }
};
const send = (expression) =>
  new Promise((resolve) => {
    const id = nextId++;
    pending.set(id, resolve);
    ws.send(JSON.stringify({ id, method: "Runtime.evaluate", params: { expression, returnByValue: true } }));
  });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const results = [];
const check = (name, value) => {
  results.push(`${value ? "PASS" : "FAIL"}  ${name}`);
  if (!value) process.exitCode = 1;
};
const onDeviceCount = () =>
  send("document.querySelectorAll('.narration-panel optgroup[label*=On-device] option').length");
const activeLabel = () =>
  send("[...document.querySelectorAll('.narration-panel button')].find(b => b.textContent.includes('Downloading'))?.textContent");

check("reader rendered", await send("!!document.querySelector('.page-frame')"));
await send("[...document.querySelectorAll('button')].find(b => b.textContent.includes('Narrate'))?.click()");
await sleep(1000);
check("panel open", await send("!!document.querySelector('.narration-panel')"));
await sleep(1200);
check("no on-device voices initially", (await onDeviceCount()) === 0);
check(
  "download button present",
  !!(await send("[...document.querySelectorAll('.narration-panel button')].some(b => b.textContent.includes('Download on-device voices'))")),
);

await send("[...document.querySelectorAll('.narration-panel button')].find(b => b.textContent.includes('Download on-device voices'))?.click()");

// Wait until the on-device group appears (download + list refresh), or the
// button returns to idle with nothing to show. Downloads are ~240 MB.
let appeared = false;
for (let attempt = 0; attempt < 100 && !appeared; attempt++) {
  await sleep(3000);
  const label = (await activeLabel()) ?? "";
  if (label) process.stdout.write(`  ${label}\n`);
  appeared = ((await onDeviceCount()) || 0) > 0;
}
check("on-device voices appeared", appeared);

const options = (await send("[...document.querySelectorAll('.narration-panel optgroup[label*=On-device] option')].map(o => o.textContent).join(' | ')")) || "";
console.log("on-device voices:", options);
check("daniela listed", options.includes("Daniela"));
const selected = (await send("document.querySelector('.narration-panel select')?.selectedOptions[0]?.textContent")) || "";
console.log("default:", selected);
check("daniela is default", selected.includes("Daniela"));
check("no crash", !(await send("!!document.querySelector('.crash-screen')")));

console.log(results.join("\n"));
ws.close();
