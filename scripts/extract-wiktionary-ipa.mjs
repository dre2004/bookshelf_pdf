#!/usr/bin/env node
// Distills kaikki.org's machine-readable English Wiktionary extract into
// the bundled pronunciation table (src-tauri/resources/pronunciation/
// en-US.tsv.gz). Run by the lexicon workflow (monthly or on demand);
// downloads ~534 MB and streams it — never load it into memory.
//
// Output lines are `word<TAB>IPA`, IPA normalized to the symbol set the
// Kokoro tokenizer accepts (what the curated lexicon uses): no slashes,
// syllable dots, tie bars, non-syllabic breves or parentheses; r -> ɹ,
// ASCII g -> ɡ. US-tagged pronunciations win; entries whose word contains
// a space are skipped (the narrator looks words up one token at a time).
import { createReadStream, createWriteStream, writeFileSync } from 'node:fs';
import { createGunzip, gunzipSync, gzipSync } from 'node:zlib';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { pipeline } from 'node:stream/promises';
import { once } from 'node:events';
import readline from 'node:readline';

const SOURCE =
  'https://kaikki.org/dictionary/English/kaikki.org-dictionary-English.jsonl.gz';
const OUT = join(
  dirname(fileURLToPath(import.meta.url)),
  '../src-tauri/resources/pronunciation/en-US.tsv.gz',
);
const STAGING = join(tmpdir(), 'kaikki-english.jsonl.gz');

const res = await fetch(SOURCE, { headers: { 'User-Agent': 'bookshelf-pdf-lexicon' } });
if (!res.ok) throw new Error(`kaikki download failed: HTTP ${res.status}`);
console.log(`downloading ${SOURCE} ...`);
await pipeline(res.body, createWriteStream(STAGING));
console.log('download complete; extracting...');

function normalizeIpa(raw) {
  return raw
    .replace(/[/]/g, '')
    .replace(/[\u0361\u032F]/g, '') // tie bars, non-syllabic breves
    .replace(/[.\s()]/g, '') // syllable dots, spaces, optional-sound parens
    .replace(/r/g, 'ɹ')
    .replace(/g/g, 'ɡ')
    .replace(/^[ˌˈ]+|[ˌˈ]+$/g, '') // leading/trailing stray stress marks
    .replace(/(.)\1{2,}/g, '$1$1'); // runaway repeats from template damage
}

const ACCENT_RANK = (tags) => {
  const t = tags ?? [];
  if (t.includes('US') || t.includes('GenAm')) return 0;
  if (!t.some((x) => /RP|UK|CanE|AuE|NZE|ScE|InE/.test(x))) return 1; // untagged = general
  return 2;
};

const WORD_OK = /^[A-Za-z0-9'’-]{1,40}$/;

const best = new Map(); // word -> {rank, ipa}
let seen = 0;
let kept = 0;

const rl = readline.createInterface({
  input: createReadStream(STAGING).pipe(createGunzip()),
  crlfDelay: Infinity,
});
rl.on('line', (line) => {
  seen++;
  let rec;
  try {
    rec = JSON.parse(line);
  } catch {
    return;
  }
  if (rec.lang_code !== 'en' || !Array.isArray(rec.sounds)) return;
  const word = typeof rec.word === 'string' ? rec.word.replace(/’/g, "'") : '';
  if (!WORD_OK.test(word) || !/[A-Za-z]/.test(word)) return;
  let winner;
  for (const sound of rec.sounds) {
    if (typeof sound.ipa !== 'string' || !sound.ipa.includes('/')) continue;
    const ipa = normalizeIpa(sound.ipa);
    if (ipa.length < 1 || ipa.length > 80) continue;
    const rank = ACCENT_RANK(sound.tags);
    if (!winner || rank < winner.rank) winner = { rank, ipa };
  }
  if (!winner) return;
  const prev = best.get(word);
  // Lower rank wins; first record wins ties (Wiktionary lists the most
  // central sense first).
  if (!prev || winner.rank < prev.rank) {
    best.set(word, winner);
    kept++;
  }
});
await once(rl, 'close');
console.log(`${seen.toLocaleString()} records scanned, ${kept.toLocaleString()} words kept`);

const lines = [...best.keys()].sort().map((w) => `${w}\t${best.get(w).ipa}\n`);
if (lines.length === 0) throw new Error('no entries extracted — refusing to write');
writeFileSync(OUT, gzipSync(lines.join('')));
console.log(`wrote ${OUT} (${lines.length.toLocaleString()} entries)`);

// Sanity: the table must decompress and contain the entries the narrator
// pins in tests.
const roundTrip = gunzipSync(gzipSync(lines.join(''))).toString();
if (!roundTrip.includes('AI\t') || !roundTrip.includes('LLM\t')) {
  throw new Error('sanity check failed: AI/LLM missing from the table');
}
