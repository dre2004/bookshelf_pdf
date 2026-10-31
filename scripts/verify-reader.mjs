#!/usr/bin/env node
// End-to-end verification against the live app over CDP. Usage:
//   node scripts/verify-reader.mjs [port]
// Clicks the Bookmark toolbar button (the action that used to fail with a
// FOREIGN KEY error) and exercises back/forward navigation.
const port = process.argv[2] ?? "9223";

const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === "page" && !t.url.startsWith("devtools"));
if (!page) throw new Error("no page target");

const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  ws.onopen = resolve;
  ws.onerror = reject;
});

let nextId = 1;
const pending = new Map();
ws.onmessage = (event) => {
  const message = JSON.parse(event.data);
  if (message.id && pending.has(message.id)) {
    pending.get(message.id)(message);
    pending.delete(message.id);
  }
};
const send = (method, params) =>
  new Promise((resolve) => {
    const id = nextId++;
    pending.set(id, resolve);
    ws.send(JSON.stringify({ id, method, params }));
  });

const ev = async (expression) => {
  const response = await send("Runtime.evaluate", { expression, returnByValue: true });
  if (response.result?.exceptionDetails) {
    throw new Error(`eval failed: ${expression}\n${JSON.stringify(response.result.exceptionDetails, null, 2).slice(0, 500)}`);
  }
  return response.result?.result?.value;
};
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const results = [];
const check = (name, value) => {
  results.push(`${value ? "PASS" : "FAIL"}  ${name}`);
  if (!value) process.exitCode = 1;
};

// Reader is up
check("reader rendered", await ev("!!document.querySelector('.page-frame')"));

// Click the bookmark button — the previously-broken action
const clicked = await ev(
  "(() => { const b = [...document.querySelectorAll('button')].find(b => (b.title||'').includes('Bookmark this position')); if (b) { b.click(); return true; } return false; })()",
);
check("bookmark button clicked", clicked);
await sleep(1000);

check("no error banner", (await ev("document.querySelector('.banner.error')?.innerText ?? 'none'")) === "none");
check("no crash screen", !(await ev("!!document.querySelector('.crash-screen')")));
check("sidebar opened to bookmarks", (await ev("document.querySelector('.sidebar-tab.active')?.textContent ?? ''")).trim() === "Marks");
const rows = await ev("document.querySelectorAll('.bookmark-row').length");
check("bookmark row present", rows > 0);

// Back/forward: jump via thumbnail-less path — use End key then Back button
await ev(
  "(() => { const b = [...document.querySelectorAll('button')].find(b => (b.title||'').includes('Back to where')); if (b && !b.disabled) { b.click(); return true; } return false; })()",
);
await sleep(600);
check("no crash after navigation", !(await ev("!!document.querySelector('.crash-screen')")));

console.log(results.join("\n"));
console.log(await ev("document.querySelector('.bookmark-row')?.innerText.replace(/\\n/g,' | ') ?? 'no rows'"));
ws.close();
