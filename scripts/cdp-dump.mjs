#!/usr/bin/env node
// Reads page text and console errors from a WebView2 instance started with
// --remote-debugging-port=9222. Usage: node scripts/cdp-dump.mjs
const port = 9222;

const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === "page" && !t.url.startsWith("devtools"));
if (!page) {
  console.error("no page target found; targets:", targets.map((t) => `${t.type}:${t.url}`).join(", "));
  process.exit(1);
}
console.log("target:", page.url, page.title);

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
function send(method, params = {}) {
  return new Promise((resolve) => {
    const id = nextId++;
    pending.set(id, resolve);
    ws.send(JSON.stringify({ id, method, params }));
  });
}

await send("Runtime.enable");

const evaluate = async (expression) => {
  const result = await send("Runtime.evaluate", { expression, returnByValue: true });
  return result.result?.result?.value;
};

const text = await evaluate("document.body.innerText.slice(0, 1200)");
console.log("--- page text ---");
console.log(text);

const errors = await evaluate(
  "window.__errors ? window.__errors.join('\\n---\\n') : '(window.onerror not captured)'",
);
console.log("--- window errors ---");
console.log(errors);

ws.close();
