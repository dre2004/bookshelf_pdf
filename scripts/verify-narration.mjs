#!/usr/bin/env node
// Narration end-to-end checks over CDP against a running app instance.
// Usage: node scripts/verify-narration.mjs [port]
const port = process.argv[2] ?? "9234";

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
const send = (expression, awaitPromise = false) =>
  new Promise((resolve) => {
    const id = nextId++;
    pending.set(id, resolve);
    ws.send(JSON.stringify({
      id,
      method: "Runtime.evaluate",
      params: { expression, awaitPromise, returnByValue: true },
    }));
  });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const results = [];
const check = (name, value) => {
  results.push(`${value ? "PASS" : "FAIL"}  ${name}`);
  if (!value) process.exitCode = 1;
};

check("reader rendered", await send("!!document.querySelector('.page-frame')"));

// Open the narration panel.
await send("[...document.querySelectorAll('button')].find(b => b.textContent.includes('Narrate'))?.click()");
await sleep(600);
check("panel open", await send("!!document.querySelector('.narration-panel')"));
await sleep(1200);

const options = (await send("[...document.querySelectorAll('.narration-panel option')].map(o => o.textContent).join(' | ')")) || "";
console.log("voices:", options);
check("daniela listed", options.includes("Daniela"));
const selected = (await send("document.querySelector('.narration-panel select')?.selectedOptions[0]?.textContent")) || "";
console.log("default:", selected);
check("daniela is default", selected.includes("Daniela"));

await send("[...document.querySelectorAll('.narration-panel button')].find(b => b.textContent.includes('Read from page'))?.click()");
// First on-device synthesis loads the model, then synthesizes chunks.
await sleep(15000);
const status = (await send("document.querySelector('.narration-status')?.textContent")) || "";
console.log("status:", status);
check("narration active", status.includes("Reading page"));
// The spoken-word highlight appears while a chunk is playing; poll for it
// across chunk/page boundaries.
let highlighted = false;
for (let attempt = 0; attempt < 10 && !highlighted; attempt++) {
  highlighted = ((await send("document.querySelectorAll('.overlay.narration-hit .overlay-box').length")) || 0) > 0;
  if (!highlighted) await sleep(2000);
}
check("highlight rendered", highlighted);

// Pause / resume / stop.
await send("[...document.querySelectorAll('.narration-panel button')].find(b => b.textContent.includes('Pause'))?.click()");
await sleep(400);
check("pause works", (await send("[...document.querySelectorAll('.narration-panel button')].some(b => b.textContent.includes('Resume'))")) === true);
await send("[...document.querySelectorAll('.narration-panel button')].find(b => b.textContent.includes('Resume'))?.click()");
await sleep(300);
await send("[...document.querySelectorAll('.narration-panel button')].find(b => b.textContent.includes('Stop'))?.click()");
await sleep(600);
check("stopped", !(await send("!!document.querySelector('.narration-status')")));
check("no crash", !(await send("!!document.querySelector('.crash-screen')")));

await send("window.__un && window.__un()");
console.log(results.join("\n"));
ws.close();
