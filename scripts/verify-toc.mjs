#!/usr/bin/env node
// Repro/instrumentation for: on first open, sidebar outline clicks don't
// navigate until the page content is scrolled or a couple of entries are
// clicked. Samples scrollTop + visible page on a timeline after each click.
//
// Usage: node scripts/verify-toc.mjs [port]
const port = process.argv[2] ?? "9230";

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
    throw new Error(`eval failed: ${expression}\n${JSON.stringify(response.result.exceptionDetails).slice(0, 400)}`);
  }
  return response.result?.result?.value;
};
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// Wait for the reader and the outline to be live — the earliest moment a
// human could click a chapter link.
for (let i = 0; i < 100; i++) {
  const ready = await ev(
    "!!document.querySelector('.page-frame') && document.querySelectorAll('.outline-item:not([disabled])').length > 0",
  );
  if (ready) break;
  if (i === 0) await ev("(() => { const b = [...document.querySelectorAll('button')].find(b => (b.title||'').includes('Toggle sidebar')); if (b) b.click(); return true; })()");
  await sleep(150);
}

const probe = (target) => ev(`(() => {
  const c = document.querySelector('.reader.scroll-mode');
  if (!c) return { err: 'no container' };
  const pages = [...document.querySelectorAll('.scroll-pages > .lazy-page')];
  const cr = c.getBoundingClientRect();
  const midY = cr.top + cr.height * 0.35;
  let vis = -1;
  pages.forEach((el, i) => { const r = el.getBoundingClientRect(); if (r.top <= midY && r.bottom > midY) vis = i; });
  const t = pages[${target}] ? pages[${target}].getBoundingClientRect() : null;
  return {
    scrollTop: Math.round(c.scrollTop),
    vis,
    tgtTop: t ? Math.round(t.top - cr.top) : null,
    tgtHeight: t ? Math.round(t.height) : null,
  };
})()`);

// Outline entries as [{title, page}] from the DOM.
const outline = await ev(`[...document.querySelectorAll('.outline-item:not([disabled])')].map(b => ({
  title: b.querySelector('.outline-title')?.textContent ?? '',
  page: Number(b.querySelector('.outline-page')?.textContent ?? '-1') - 1,
})).filter(o => o.page >= 0)`);
console.log(`outline entries: ${outline.length}`);

// Two far-apart chapters, mirroring the reported sequence.
const sorted = [...outline].sort((a, b) => b.page - a.page);
const first = sorted[0];
const second = sorted[Math.floor(sorted.length / 2)];
console.log(`click 1 -> page ${first.page + 1}: ${first.title}`);
console.log(`click 2 -> page ${second.page + 1}: ${second.title}`);

const clickEntry = (entry) =>
  ev(`(() => {
    const b = [...document.querySelectorAll('.outline-item:not([disabled])')].find(
      x => Number(x.querySelector('.outline-page')?.textContent ?? '-1') - 1 === ${entry.page});
    if (!b) return false; b.click(); return true;
  })()`);

const timeline = async (label, entry) => {
  if (!(await clickEntry(entry))) { console.log(`${label}: entry not found`); return; }
  const t0 = Date.now();
  let settled = -2;
  for (let i = 0; i < 24; i++) {
    const s = await probe(entry.page);
    console.log(`${label} +${Date.now() - t0}ms  scrollTop=${s.scrollTop} visible=${s.vis} tgtTop=${s.tgtTop}`);
    if (i === 23) settled = s.vis;
    await sleep(120);
  }
  console.log(`${label} settled on visible page ${settled} (target ${entry.page}) -> ${settled === entry.page ? "ARRIVED" : "DID NOT ARRIVE"}`);
};

await timeline("click1", first);
await timeline("click2", second);
ws.close();
