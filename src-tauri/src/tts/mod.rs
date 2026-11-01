//! On-device narration: the Kokoro-82M engine (see [`kokoro`]) synthesizes
//! each page chunk through ONNX Runtime, phonemization is pure-Rust G2P
//! (`piper_plus_g2p`, no espeak-ng, no Python), playback runs through rodio
//! with prefetch so chunks flow without gaps.

mod chunker;
pub mod homograph;
pub mod kokoro;

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::Serialize;
use tauri::Emitter;

use chunker::split_for_narration;
use kokoro::KokoroModel;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsVoiceInfo {
    /// Selection key: `kokoro:<voice>`.
    pub id: String,
    /// Human-friendly label.
    pub label: String,
    /// Locale, e.g. `es_AR` / `en-US`.
    pub lang: String,
    /// Which engine synthesizes this voice.
    pub engine: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum TtsEvent {
    /// A chunk starting at char `start` (of the page text) is now playing.
    Progress {
        page: i32,
        start: i64,
        end: i64,
    },
    /// All chunks of the page have been queued.
    PageDone {
        page: i32,
    },
    Error {
        reason: String,
    },
}

pub struct TtsEngine {
    app: tauri::AppHandle,
    kokoro: Mutex<Option<(PathBuf, Arc<KokoroModel>)>>,
    playback: Mutex<PlaybackControl>,
    device: Mutex<Option<rodio::MixerDeviceSink>>,
}

struct PlaybackControl {
    stop: Arc<AtomicBool>,
    player: Option<Arc<rodio::Player>>,
    /// Wall-clock reference for scheduling highlight events at the moment
    /// their audio plays, while synthesis runs ahead (prefetch). Pausing
    /// the player also pauses this clock.
    clock: Arc<Mutex<Option<PlaybackClock>>>,
}

/// Playback time that excludes pauses: `start` is when the first chunk's
/// audio began, `paused_total` accumulates completed pauses, and
/// `pause_started` marks an in-flight pause.
#[derive(Clone, Copy)]
struct PlaybackClock {
    start: std::time::Instant,
    paused_total: std::time::Duration,
    pause_started: Option<std::time::Instant>,
}

impl PlaybackClock {
    fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
            paused_total: std::time::Duration::ZERO,
            pause_started: None,
        }
    }

    fn playing_elapsed(&self) -> std::time::Duration {
        let in_pause = self
            .pause_started
            .map(|began| began.elapsed())
            .unwrap_or_default();
        self.start.elapsed() - self.paused_total - in_pause
    }

    fn pause(&mut self) {
        if self.pause_started.is_none() {
            self.pause_started = Some(std::time::Instant::now());
        }
    }

    fn resume(&mut self) {
        if let Some(began) = self.pause_started.take() {
            self.paused_total += began.elapsed();
        }
    }
}

/// A heading lead recognized at the start of a chunk, split into one or
/// two heading pieces ("Chapter 0" kicker line, then the title line) with
/// the pause each piece should play after it.
pub(crate) struct HeadingLead {
    pieces: Vec<((usize, usize), u32, String)>,
}

/// True for lines like "Chapter 0", "Part II", "Appendix B", "Section 4".
fn is_chapter_kicker(line: &str) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.len() != 2 {
        return false;
    }
    matches!(
        words[0].to_ascii_lowercase().as_str(),
        "chapter" | "part" | "appendix" | "section"
    ) && !words[1].is_empty()
}

/// How far into a chunk the heading-lead rules scan. Heading leads sit at
/// the start of a chunk; the chunker's groups run up to ~400 chars, so the
/// scan must be bounded by its own window — a whole-chunk length cap
/// silently disabled heading detection on every real book.
const LEAD_WINDOW_CHARS: usize = 160;

/// A standalone heading line inside a chunk: an outline title, or a
/// section-number lead with a capitalized title after it and no sentence
/// punctuation. Wrapped body lines never match these rules.
const HEADING_LINE_MAX_CHARS: usize = 60;

/// Cap on heading pieces within one chunk — the pause-storm guard.
const MAX_HEADING_PIECES_PER_CHUNK: usize = 2;

/// The beat played after an em/en dash that sets off a parenthetical
/// ("end to end — an hour — because..."). Shorter than the 0.4s sentence
/// gap: a dash is a breath inside a sentence, not a boundary.
const DASH_PAUSE_MS: u32 = 300;

/// The beat after a semicolon, which joins two independent clauses:
/// longer than a dash's breath, shorter than the sentence gap. Without
/// this, semicolons are silently dropped by the phoneme tokenizer and
/// contribute no pause at all.
const SEMICOLON_PAUSE_MS: u32 = 350;

/// The beat after a colon, which introduces a list or an explanation
/// ("it shows: the typing is not a ritual"). Same length as a semicolon;
/// kept separate so it can be tuned independently.
const COLON_PAUSE_MS: u32 = 350;

/// Minimum non-whitespace text on each side of a beat marker for it to
/// earn a pause; markers glued between short fragments stay inline.
const BEAT_SPLIT_MIN_CHARS: usize = 6;

/// Splits a body piece at its in-sentence beat markers — parenthetical
/// em/en dashes, semicolons and colons — each becoming a short pause on
/// the piece before it. Numeric ranges ("3–7") and times ("12:30") never
/// split, and hyphenated compounds (U+002D) never split. The final
/// sub-piece keeps the pause the body carried (sentence/paragraph).
fn split_at_beats(chars: &[char], range: (usize, usize), final_pause: u32) -> Vec<Piece> {
    let (start, end) = range;
    let mut cuts: Vec<(usize, u32)> = Vec::new();
    let mut last_cut = start;
    for i in start..end {
        let pause = match chars[i] {
            '\u{2014}' | '\u{2013}' => {
                let prev_digit = i > 0 && chars[i - 1].is_ascii_digit();
                let next_digit = chars.get(i + 1).is_some_and(|c| c.is_ascii_digit());
                if prev_digit && next_digit {
                    // Numeric range "3–7", not a parenthetical.
                    continue;
                }
                DASH_PAUSE_MS
            }
            ':' => {
                let prev_digit = i > 0 && chars[i - 1].is_ascii_digit();
                let next_digit = chars.get(i + 1).is_some_and(|c| c.is_ascii_digit());
                if prev_digit && next_digit {
                    // Time or ratio "12:30", not an introduction.
                    continue;
                }
                COLON_PAUSE_MS
            }
            ';' => SEMICOLON_PAUSE_MS,
            _ => continue,
        };
        let before = chars[last_cut..i]
            .iter()
            .filter(|c| !c.is_whitespace())
            .count();
        let after = chars[i + 1..end]
            .iter()
            .filter(|c| !c.is_whitespace())
            .count();
        if before >= BEAT_SPLIT_MIN_CHARS && after >= BEAT_SPLIT_MIN_CHARS {
            cuts.push((i, pause));
            last_cut = i;
        }
    }
    if cuts.is_empty() {
        let text: String = chars[start..end].iter().collect();
        return vec![(range, final_pause, text)];
    }
    let mut pieces = Vec::with_capacity(cuts.len() + 1);
    let mut piece_start = start;
    for (cut, pause) in cuts
        .iter()
        .copied()
        .chain(std::iter::once((end, final_pause)))
    {
        let text: String = chars[piece_start..cut].iter().collect();
        pieces.push(((piece_start, cut), pause, text));
        piece_start = cut + 1;
    }
    pieces
}

pub(crate) fn split_heading_lead(
    chars: &[char],
    range: (usize, usize),
    outline_titles: &[String],
) -> Option<HeadingLead> {
    let (start, end) = range;
    if end - start < 3 {
        return None;
    }
    let text: String = chars[start..end].iter().collect();

    // Chapter-opening block: a kicker line ("Chapter 0") over the title
    // line, with the title matching an outline title or standing as a
    // short capitalized line on its own. All offsets below are char
    // indices into `chars`, never byte offsets into `text`.
    let first_newline = chars[start..end]
        .iter()
        .position(|c| *c == '\n' || *c == '\r');
    if let Some(nl) = first_newline {
        let line1: String = chars[start..start + nl]
            .iter()
            .collect::<String>()
            .trim_end()
            .to_string();
        if is_chapter_kicker(&line1) {
            // Cursor stays inside the chunk: a paragraph break can land
            // between the \r and \n of a split CRLF, and scanning past
            // `end` would panic (and pull the next chunk's text in).
            let mut cursor = (start + nl).min(end);
            while cursor < end
                && matches!(chars.get(cursor), Some(c) if c.is_whitespace() && *c != '\n' && *c != '\r')
            {
                cursor += 1;
            }
            // skip consecutive newline whitespace
            while cursor < end && matches!(chars.get(cursor), Some('\n') | Some('\r')) {
                cursor += 1;
            }
            // Title line runs to the next line break.
            let title_end = chars[cursor..end]
                .iter()
                .position(|c| *c == '\n' || *c == '\r')
                .map(|k| cursor + k)
                .unwrap_or(end);
            let title_text: String = chars[cursor..title_end].iter().collect();
            let title_trim = title_text.trim_end();
            let title_is_heading = is_heading_like(title_trim, outline_titles)
                || outline_titles
                    .iter()
                    .any(|t| normalize_for_match(t) == normalize_for_match(title_trim));
            let mut pieces = vec![((start, cursor), HEADING_KICKER_PAUSE_MS, line1)];
            if !title_trim.is_empty() && title_is_heading {
                pieces.push(((cursor, title_end), HEADING_PAUSE_MS, title_text));
            }
            return Some(HeadingLead { pieces });
        }
    }

    // Outline title at the very start of the chunk (e.g. "Preface" over
    // the body "C has outlived..."). Split right after the title's
    // matching characters.
    for title in outline_titles {
        let normalized_title = normalize_for_match(title);
        if normalized_title.is_empty() {
            continue;
        }
        let mut ti = title.chars().peekable();
        let mut consumed_to = None;
        for (idx, c) in text.chars().enumerate().take(LEAD_WINDOW_CHARS) {
            match ti.peek() {
                Some(&tc) if c.is_whitespace() && tc.is_whitespace() => {
                    ti.next();
                    consumed_to = Some(idx + 1);
                }
                Some(&tc) if c == tc => {
                    ti.next();
                    consumed_to = Some(idx + 1);
                }
                _ => break,
            }
        }
        if ti.peek().is_none() && consumed_to.unwrap_or(0) < text.chars().count() {
            let split = start + consumed_to.unwrap_or(0);
            let heading: String = chars[start..split].iter().collect();
            return Some(HeadingLead {
                pieces: vec![((start, split), HEADING_PAUSE_MS, heading)],
            });
        }
    }

    // Numbered lead ("0.1 How to read this book", "Two tracks...").
    let first_word = text
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
    let number_core = first_word.trim_end_matches('.');
    let numbered = !number_core.is_empty()
        && number_core.chars().all(|c| c.is_ascii_digit() || c == '.')
        && number_core
            .chars()
            .last()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false);
    if !numbered {
        return None;
    }
    for (idx, c) in text.chars().enumerate().take(LEAD_WINDOW_CHARS) {
        if idx == 0 {
            continue;
        }
        let terminator = matches!(c, '!' | '?' | ':' | '\n' | '\r')
            || (c == '.' && {
                let next = text.chars().nth(idx + 1);
                !matches!(next, Some(n) if n.is_ascii_digit())
            });
        if !terminator {
            continue;
        }
        let split = start + idx + 1;
        if split >= end {
            return None;
        }
        let lead: String = chars[start..split].iter().collect();
        let lead_trim = lead
            .trim_end()
            .trim_end_matches(['.', '!', '?', ':'])
            .trim_end();
        if is_heading_like(lead_trim, outline_titles) {
            return Some(HeadingLead {
                pieces: vec![((start, split), HEADING_PAUSE_MS, lead)],
            });
        }
        return None;
    }
    None
}

/// Whether a standalone line is a heading: an outline title, or a
/// section-number lead ("0.1 How to read this book"). Body lines that
/// merely start with a decimal ("12.5 percent of cases") don't match:
/// the word after the number must be capitalized and the line must carry
/// no sentence punctuation beyond the number itself.
fn is_heading_line(trimmed: &str, outline_titles: &[String]) -> bool {
    if trimmed.is_empty() || trimmed.chars().count() > HEADING_LINE_MAX_CHARS {
        return false;
    }
    if outline_titles
        .iter()
        .any(|t| normalize_for_match(t) == normalize_for_match(trimmed))
    {
        return true;
    }
    let Some(first) = trimmed.split_whitespace().next() else {
        return false;
    };
    let number = first.trim_end_matches('.');
    let numbered = !number.is_empty()
        && number.chars().next().is_some_and(|c| c.is_ascii_digit())
        && number.chars().last().is_some_and(|c| c.is_ascii_digit())
        && number.chars().all(|c| c.is_ascii_digit() || c == '.');
    if !numbered {
        return false;
    }
    let after_number = trimmed[first.len()..].trim_start();
    let starts_title = after_number
        .chars()
        .next()
        .is_some_and(|c| c.is_uppercase());
    starts_title
        && !after_number.contains('.')
        && !after_number.contains('!')
        && !after_number.contains('?')
}

/// Appends one playback piece: samples, a trailing pause (in ms, skipped
/// on the page's final piece), and the highlight scheduled at its
/// projected play offset. Shared by the announcement paths.
#[allow(clippy::too_many_arguments)] // one arg per playback concern
fn append_piece(
    player: &Arc<rodio::Player>,
    schedule: Schedule<'_>,
    page: i32,
    range: (usize, usize),
    samples: Vec<f32>,
    sample_rate: u32,
    pause_ms: u32,
    last_chunk: (usize, usize),
) {
    let sample_rate = std::num::NonZero::new(sample_rate).expect("nonzero rate");
    let mut buffered = samples;
    if range.1 != last_chunk.1 {
        buffered.extend(vec![
            0.0;
            sample_rate.get() as usize * pause_ms as usize / 1000
        ]);
    }
    player.append(rodio::buffer::SamplesBuffer::new(
        rodio::ChannelCount::new(1).expect("nonzero channels"),
        sample_rate,
        buffered.clone(),
    ));
    schedule(buffered, sample_rate, page, Some(range));
}

/// Everything the speak pipeline needs, collected so the entry point stays
/// at one argument.
pub struct SpeakRequest<'a> {
    pub text: &'a str,
    pub page: i32,
    pub voice_id: &'a str,
    pub rate: f32,
    pub paragraph_breaks: Vec<usize>,
    pub outline_titles: Vec<String>,
    pub code_blocks: Vec<(usize, usize)>,
    pub code_handling: String,
}

/// A playback piece: char range, trailing pause in ms, spoken text.
type Piece = ((usize, usize), u32, String);

/// The speak loops' highlight scheduler.
type Schedule<'a> = &'a dyn Fn(Vec<f32>, std::num::NonZero<u32>, i32, Option<(usize, usize)>);

/// Marks each narration chunk as code when >= 60% of its characters fall
/// inside a detected code block.
fn classify_code_chunks(chunks: &[(usize, usize)], code_blocks: &[(usize, usize)]) -> Vec<bool> {
    chunks
        .iter()
        .map(|(start, end)| {
            let length = end.saturating_sub(*start).max(1);
            let covered: usize = code_blocks
                .iter()
                .map(|(bs, be)| {
                    let lo = (*start).max(*bs);
                    let hi = (*end).min(*be);
                    hi.saturating_sub(lo)
                })
                .sum();
            covered * 10 >= length * 6
        })
        .collect()
}

/// Playback pieces for one chunk: a heading lead at the chunk start, and
/// standalone heading lines anywhere after it, become their own pieces
/// carrying the heading pause; body text between them carries the
/// sentence/paragraph pause. Nothing else is ever split.
fn heading_aware_pieces(
    chars: &[char],
    chunk: (usize, usize),
    outline_titles: &[String],
    paragraph_breaks: &[usize],
) -> Vec<Piece> {
    let (start, end) = chunk;
    // Pure boundary classification: whether a paragraph break sits at this
    // chunk's end. The speak loops separately suppress the pause on the
    // page's final piece, so "runs to end of text" is not a paragraph.
    let ends_paragraph =
        paragraph_breaks.contains(&end.saturating_sub(1)) || paragraph_breaks.contains(&end);
    let final_pause = if ends_paragraph {
        PARAGRAPH_PAUSE_MS
    } else {
        SENTENCE_PAUSE_MS
    };

    let lead_pieces = split_heading_lead(chars, chunk, outline_titles)
        .map(|lead| lead.pieces)
        .unwrap_or_default();
    let mut heading_count = lead_pieces.len();
    let mut body_start = lead_pieces.last().map(|(r, _, _)| r.1).unwrap_or(start);
    let mut pieces: Vec<((usize, usize), u32, String)> = Vec::new();

    // Walk the remaining lines; a standalone heading line closes the body
    // piece before it and becomes its own piece. Piece ranges include the
    // line break so the next piece starts clean.
    let mut cursor = body_start;
    while cursor < end && heading_count < MAX_HEADING_PIECES_PER_CHUNK {
        let line_end = chars[cursor..end]
            .iter()
            .position(|c| *c == '\n' || *c == '\r')
            .map(|k| cursor + k)
            .unwrap_or(end);
        let next_start = chars[line_end..end]
            .iter()
            .position(|c| *c != '\n' && *c != '\r')
            .map(|k| line_end + k)
            .unwrap_or(end);
        let line: String = chars[cursor..line_end].iter().collect();
        let trimmed = line.trim();
        if !trimmed.is_empty() && is_heading_line(trimmed, outline_titles) {
            if chars[body_start..cursor].iter().any(|c| !c.is_whitespace()) {
                pieces.extend(split_at_beats(
                    chars,
                    (body_start, cursor),
                    PARAGRAPH_PAUSE_MS,
                ));
            }
            pieces.push(((cursor, next_start), HEADING_PAUSE_MS, trimmed.to_string()));
            heading_count += 1;
            body_start = next_start;
        }
        cursor = next_start;
    }

    let mut all = lead_pieces;
    all.append(&mut pieces);
    // A heading lead can consume the whole chunk (its title lives in the
    // next one); the leftover range is then whitespace only and must not
    // become an empty piece.
    if chars[body_start..end].iter().any(|c| !c.is_whitespace()) {
        all.extend(split_at_beats(chars, (body_start, end), final_pause));
    }
    all
}

const HEADING_PAUSE_MS: u32 = 1000;
const HEADING_KICKER_PAUSE_MS: u32 = 1000;
const PARAGRAPH_PAUSE_MS: u32 = 800;
const SENTENCE_PAUSE_MS: u32 = 400;

/// Heading detection, in order of confidence:
/// 1. The chunk matches a PDF outline title (the sidebar's own TOC) —
///    exact after normalizing case/whitespace/typographic punctuation.
/// 2. Structural cues: short, no sentence-ending punctuation, and either
///    section-numbered, colon-ended, or every word capitalized.
fn is_heading_like(trimmed: &str, outline_titles: &[String]) -> bool {
    let normalized = normalize_for_match(trimmed);
    if outline_titles
        .iter()
        .any(|title| normalize_for_match(title) == normalized)
    {
        return true;
    }
    let words = trimmed.split_whitespace().count();
    if words == 0 || words > 10 || trimmed.chars().count() > 70 {
        return false;
    }
    if trimmed.ends_with('.') || trimmed.ends_with('!') || trimmed.ends_with('?') {
        return false;
    }
    let starts_numbered = trimmed
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false);
    if trimmed.ends_with(':') {
        return true;
    }
    if starts_numbered {
        // A section-number lead is a heading only when a capitalized
        // title follows ("0.1 How to read this book"); lowercase prose
        // after the number ("12.5 percent of cases") is a sentence that
        // happens to start with a decimal.
        let number_len = trimmed.split_whitespace().next().map(str::len).unwrap_or(0);
        let after_number = trimmed[number_len..].trim_start();
        return after_number
            .chars()
            .next()
            .is_some_and(|c| c.is_uppercase());
    }
    // Title Case: every word starts uppercase (headings, not sentences).
    words > 1
        && trimmed.split_whitespace().all(|word| {
            word.chars()
                .next()
                .map(|c| c.is_uppercase())
                .unwrap_or(false)
        })
}

fn normalize_for_match(text: &str) -> String {
    let curly_apostrophe = char::from_u32(0x2019).unwrap();
    let curly_open = char::from_u32(0x2018).unwrap();
    let curly_dquote_a = char::from_u32(0x201C).unwrap();
    let curly_dquote_b = char::from_u32(0x201D).unwrap();
    let en_dash = char::from_u32(0x2013).unwrap();
    let em_dash = char::from_u32(0x2014).unwrap();
    text.chars()
        .map(|c| match c {
            c if c == curly_apostrophe || c == curly_open => '\'',
            c if c == curly_dquote_a || c == curly_dquote_b => '"',
            c if c == en_dash || c == em_dash => '-',
            other => other,
        })
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

impl TtsEngine {
    pub fn app_handle(&self) -> &tauri::AppHandle {
        &self.app
    }

    pub fn new(app: tauri::AppHandle) -> Self {
        Self {
            app,
            kokoro: Mutex::new(None),
            playback: Mutex::new(PlaybackControl {
                stop: Arc::new(AtomicBool::new(false)),
                player: None,
                clock: Arc::new(Mutex::new(None)),
            }),
            device: Mutex::new(None),
        }
    }

    /// The narration voice list: the full Kokoro English catalog; voice
    /// style files download on demand with the engine pack.
    pub fn list_voices(&self) -> Vec<TtsVoiceInfo> {
        let mut voices: Vec<TtsVoiceInfo> = kokoro::KOKORO_VOICE_SPECS
            .iter()
            .map(|(id, name, gender, accent)| TtsVoiceInfo {
                id: id.to_string(),
                label: format!("{name} ({accent}, {gender})"),
                lang: "en-US".into(),
                engine: "kokoro".into(),
            })
            .collect();
        voices.sort_by(|a, b| a.label.cmp(&b.label));
        voices
    }

    /// Whether the Kokoro engine pack (model + tokenizer) is present in any
    /// known location or installable location.
    pub fn kokoro_pack_installed(&self) -> bool {
        kokoro::kokoro_dirs(&self.app)
            .iter()
            .any(|dir| dir.join("model.onnx").is_file())
    }

    /// Where the Kokoro pack lives (existing location, else the install
    /// target in app data).
    pub fn kokoro_dir(&self) -> Result<PathBuf, String> {
        if let Some(dir) = kokoro::kokoro_dirs(&self.app).into_iter().next() {
            return Ok(dir);
        }
        self.install_kokoro_dir()
    }

    fn install_kokoro_dir(&self) -> Result<PathBuf, String> {
        use tauri::Manager;
        Ok(self
            .app
            .path()
            .app_data_dir()
            .map_err(|e| format!("app data dir: {e}"))?
            .join("kokoro"))
    }

    /// Plays the settings-screen sample sentence in the given voice,
    /// downloading the voice file (and the pack, if missing) first.
    pub fn preview_voice(
        &self,
        app: &tauri::AppHandle,
        voice: &str,
        speed: f32,
    ) -> Result<(), String> {
        let dir = self.kokoro_dir()?;
        if !dir.join("model.onnx").is_file() {
            kokoro::download_kokoro_pack(app, &dir)?;
        }
        kokoro::download_voice(app, &dir, voice)?;
        // A fresh voice file must not be hidden by a cached model handle.
        self.kokoro.lock().unwrap().take();
        kokoro::play_voice_sample(&dir, voice, speed)
    }

    fn ensure_device(&self) -> Result<(), String> {
        let mut guard = self.device.lock().unwrap();
        if guard.is_none() {
            let sink = rodio::DeviceSinkBuilder::open_default_sink()
                .map_err(|e| format!("no audio output device: {e}"))?;
            *guard = Some(sink);
        }
        Ok(())
    }

    fn player(&self) -> Result<Arc<rodio::Player>, String> {
        self.ensure_device()?;
        let guard = self.device.lock().unwrap();
        let sink = guard.as_ref().expect("device initialized");
        Ok(Arc::new(rodio::Player::connect_new(sink.mixer())))
    }

    fn kokoro_model(&self) -> Result<(PathBuf, Arc<KokoroModel>), String> {
        if let Some(pair) = self.kokoro.lock().unwrap().clone() {
            return Ok(pair);
        }
        let dir = kokoro::kokoro_dirs(&self.app)
            .into_iter()
            .find(|dir| dir.join("model.onnx").is_file())
            .ok_or("the Kokoro voice pack is not installed")?;
        let model = Arc::new(KokoroModel::load(&dir)?);
        let pair = (dir, model);
        *self.kokoro.lock().unwrap() = Some(pair.clone());
        Ok(pair)
    }

    pub fn stop(&self) {
        let mut playback = self.playback.lock().unwrap();
        playback.stop.store(true, Ordering::Relaxed);
        *playback.clock.lock().unwrap() = None;
        if let Some(player) = playback.player.take() {
            player.clear();
            player.stop();
        }
    }

    pub fn pause(&self) {
        let playback = self.playback.lock().unwrap();
        if let Some(clock) = playback.clock.lock().unwrap().as_mut() {
            clock.pause();
        }
        if let Some(player) = playback.player.as_ref() {
            player.pause();
        }
    }

    pub fn resume(&self) {
        let playback = self.playback.lock().unwrap();
        if let Some(clock) = playback.clock.lock().unwrap().as_mut() {
            clock.resume();
        }
        if let Some(player) = playback.player.as_ref() {
            player.play();
        }
    }

    /// Speaks `text` (a whole page) chunk by chunk on a worker thread,
    /// emitting progress events as each chunk starts playing. Returns once
    /// the worker is launched; playback continues in the background.
    pub fn speak(&self, request: SpeakRequest<'_>) -> Result<(), String> {
        self.stop();

        let SpeakRequest {
            text: request_text,
            page,
            voice_id,
            rate,
            paragraph_breaks,
            outline_titles,
            code_blocks,
            code_handling,
        } = request;
        let text = request_text.to_string();
        let voice_id = voice_id.to_string();
        let player = self.player()?;
        let app = self.app.clone();

        let (stop, clock) = {
            let mut playback = self.playback.lock().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let clock = Arc::new(Mutex::new(None::<PlaybackClock>));
            playback.stop = stop.clone();
            playback.clock = clock.clone();
            playback.player = Some(player.clone());
            (stop, clock)
        };
        // Audio queued so far (seconds); chunk highlights are scheduled at
        // their projected play offset while synthesis runs ahead.
        let queued = Arc::new(Mutex::new(0f64));

        // Appends a chunk and schedules its highlight (and, at the final
        // offset, PageDone) against the playback clock.
        let schedule = {
            let clock = clock.clone();
            let queued = queued.clone();
            let stop = stop.clone();
            let app = app.clone();
            move |samples: Vec<f32>,
                  sample_rate: std::num::NonZero<u32>,
                  page: i32,
                  range: Option<(usize, usize)>| {
                let sample_rate = sample_rate.get();
                let duration = samples.len() as f64 / sample_rate as f64;
                let offset_secs;
                {
                    let mut queued = queued.lock().unwrap();
                    offset_secs = *queued;
                    *queued += duration;
                }
                let offset = std::time::Duration::from_secs_f64(offset_secs.max(0.0));
                {
                    let mut clock = clock.lock().unwrap();
                    if clock.is_none() {
                        *clock = Some(PlaybackClock::new());
                    }
                }
                let event_app = app.clone();
                let event_clock = clock.clone();
                let event_stop = stop.clone();
                std::thread::spawn(move || {
                    loop {
                        if event_stop.load(Ordering::Relaxed) {
                            return;
                        }
                        let due = {
                            let guard = event_clock.lock().unwrap();
                            match guard.as_ref() {
                                Some(clock) => clock.playing_elapsed() >= offset,
                                // Stopped while scheduling: clock cleared.
                                None => return,
                            }
                        };
                        if due {
                            let event = match range {
                                Some((start, end)) => TtsEvent::Progress {
                                    page,
                                    start: start as i64,
                                    end: end as i64,
                                },
                                None => TtsEvent::PageDone { page },
                            };
                            let _ = event_app.emit("tts-event", event);
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                });
            }
        };

        // Panel speed is calibrated: 1.0x plays at 0.95 of raw engine
        // speed per user tuning; the slider stays honest about ratios.
        let rate = rate.clamp(0.5, 2.0) * SPEED_CALIBRATION;
        const SPEED_CALIBRATION: f32 = 0.95;
        let text_chars: Vec<char> = text.chars().collect();
        let chunks = split_for_narration(&text, &paragraph_breaks);
        // Code-block handling per the narration setting: chunk overlap
        // with a detected block decides membership; announcements collapse
        // consecutive code chunks into one "Code listing." piece.
        let announce_code = code_handling != "skip";
        let code_chunks = classify_code_chunks(&chunks, &code_blocks);
        let last_chunk = chunks.last().copied().unwrap_or((0, 0));

        if let Some(kokoro_voice) = voice_id.strip_prefix("kokoro:").map(str::to_string) {
            let (dir, model) = self.kokoro_model()?;
            std::thread::spawn(move || {
                let mut announce_pending = false;
                for (index, (start, end)) in chunks.into_iter().enumerate() {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    if code_chunks[index] {
                        // Code listing: synthesize nothing from its text —
                        // either a single announcement or silence.
                        if announce_code && !announce_pending {
                            announce_pending = true;
                            if let Ok((samples, piece_rate)) =
                                model.synthesize(&dir, "Code listing.", &kokoro_voice, rate)
                                && !samples.is_empty()
                            {
                                append_piece(
                                    &player,
                                    &schedule,
                                    page,
                                    (start, end),
                                    samples,
                                    piece_rate,
                                    1500,
                                    last_chunk,
                                );
                            }
                        } else if !announce_code {
                            // Skip mode: still advance the highlight briefly
                            // so the selection tracks past the listing.
                            schedule(
                                Vec::new(),
                                24_000u32.try_into().expect("rate"),
                                page,
                                Some((start, end)),
                            );
                        }
                        continue;
                    }
                    announce_pending = false;
                    // A heading glued to body text becomes two pieces:
                    // the heading (1s pause after), then the body.
                    let pieces = heading_aware_pieces(
                        &text_chars,
                        (start, end),
                        &outline_titles,
                        &paragraph_breaks,
                    );
                    for (piece, pause_ms, piece_text) in pieces {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        let (samples, piece_rate) =
                            match model.synthesize(&dir, &piece_text, &kokoro_voice, rate) {
                                Ok(audio) => audio,
                                Err(e) => {
                                    let _ = app.emit("tts-event", TtsEvent::Error { reason: e });
                                    return;
                                }
                            };
                        if samples.is_empty() {
                            continue;
                        }
                        let sample_rate =
                            rodio::SampleRate::new(piece_rate).expect("nonzero sample rate");
                        let mut samples = samples;
                        // Each piece carries its own trailing pause; the
                        // page's final piece ends clean so the next page
                        // starts immediately.
                        if piece.1 != last_chunk.1 {
                            samples.extend(vec![
                                0.0;
                                sample_rate.get() as usize * pause_ms as usize
                                    / 1000
                            ]);
                        }
                        player.append(rodio::buffer::SamplesBuffer::new(
                            rodio::ChannelCount::new(1).expect("nonzero channels"),
                            sample_rate,
                            samples.clone(),
                        ));
                        // Highlight is scheduled at this piece's projected
                        // play time; synthesis runs ahead so narration flows.
                        schedule(samples, sample_rate, page, Some(piece));
                    }
                }
                schedule(
                    Vec::new(),
                    std::num::NonZero::new(24_000).expect("rate"),
                    page,
                    None,
                );
            });
            return Ok(());
        }

        Err(format!(
            "unknown voice '{voice_id}': only Kokoro voices are available"
        ))
    }
}

#[cfg(test)]
mod heading_lead_tests {
    use super::*;

    fn chars(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    fn pieces(text: &str, outline: &[&str]) -> Vec<((usize, usize), u32, String)> {
        let outline: Vec<String> = outline.iter().map(|s| s.to_string()).collect();
        heading_aware_pieces(&chars(text), (0, text.chars().count()), &outline, &[])
    }

    #[test]
    fn chapter_block_splits_into_kicker_title_and_body() {
        // The real opening of page 11 of the test book.
        let text = "Chapter 0\nIf You\u{2019}re New Here\nThis book begins with programs, and it begins quickly. Here is the first one.";
        let outline = ["If You\u{2019}re New Here"];
        let pieces = pieces(text, &outline);
        assert_eq!(
            pieces.len(),
            3,
            "expected kicker + title + body: {pieces:?}"
        );
        assert_eq!(pieces[0].2.trim(), "Chapter 0");
        assert_eq!(pieces[0].1, HEADING_KICKER_PAUSE_MS);
        assert_eq!(pieces[1].2.trim(), "If You\u{2019}re New Here");
        assert_eq!(pieces[1].1, HEADING_PAUSE_MS);
        assert!(
            pieces[2].2.trim_start().starts_with("This book begins"),
            "body wrong: {:?}",
            pieces[2].2
        );
        assert_eq!(pieces[2].1, SENTENCE_PAUSE_MS);
    }

    #[test]
    fn kicker_without_title_still_pauses() {
        // "Chapter 3" over plain prose: no title line, so the kicker gets
        // its beat and the rest reads as body.
        let text = "Chapter 3\nOnce upon a time there was a language. It compiled fast.";
        let pieces = pieces(text, &[]);
        assert_eq!(pieces.len(), 2, "expected kicker + body: {pieces:?}");
        assert_eq!(pieces[0].2.trim(), "Chapter 3");
        assert_eq!(pieces[0].1, HEADING_KICKER_PAUSE_MS);
        assert!(pieces[1].2.trim_start().starts_with("Once upon a time"));
    }

    #[test]
    fn numbered_heading_splits_from_body_with_pause() {
        let text = "0.1 How to read this book. You will read this book carefully.";
        let pieces = pieces(text, &[]);
        assert_eq!(pieces.len(), 2, "expected heading + body: {pieces:?}");
        let (_, heading_pause, heading_text) = &pieces[0];
        assert_eq!(*heading_pause, HEADING_PAUSE_MS);
        assert!(
            heading_text.contains("How to read this book"),
            "heading incomplete: {heading_text:?}"
        );
        assert!(
            pieces[1].2.starts_with(" You"),
            "body wrong: {:?}",
            pieces[1].2
        );
    }

    #[test]
    fn outline_title_heading_without_period_splits() {
        let text = "If You Are New Here you should read chapter one first. It covers the basics.";
        let outline = ["If You Are New Here"];
        let pieces = pieces(text, &outline);
        assert_eq!(pieces.len(), 2, "expected split: {pieces:?}");
        assert_eq!(pieces[0].1, HEADING_PAUSE_MS);
        // The heading ends at the outline title boundary, before "you should".
        assert!(pieces[0].2.contains("New Here"));
    }

    #[test]
    fn kicker_chunk_ending_between_crlf_halves_stays_whole() {
        // A paragraph break can land between the \r and \n of a split
        // CRLF, so the kicker chunk carries only the \r and the title
        // lives in the next chunk. The kicker must still read whole (and
        // the cursor must not scan past the chunk end), with no empty
        // tail piece. Shape traced from page 11 of the test book after
        // the paragraph-break fix.
        let text = "Chapter 0\r\nIf You\u{2019}re New Here";
        let chars = chars(text);
        let pieces = heading_aware_pieces(&chars, (0, 10), &[], &[]);
        assert_eq!(pieces.len(), 1, "kicker only, no empty tail: {pieces:?}");
        assert_eq!(pieces[0].2.trim(), "Chapter 0");
        assert_eq!(pieces[0].1, HEADING_KICKER_PAUSE_MS);
    }

    #[test]
    fn whitespace_only_remainder_makes_no_piece() {
        // A chunk that is entirely consumed by its heading (title included)
        // must not emit an empty body piece — those synthesized as a blip.
        let text = "If You Are New Here\r\n";
        let outline = ["If You Are New Here"];
        let pieces = pieces(text, &outline);
        assert_eq!(pieces.len(), 1, "heading only: {pieces:?}");
        assert!(pieces[0].2.trim().contains("New Here"));
    }

    #[test]
    fn plain_body_never_splits() {
        let text = "The rain fell for days and the river rose over its banks. Fields flooded.";
        let pieces = pieces(text, &[]);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].1, SENTENCE_PAUSE_MS);
    }

    // The four tests below use the exact chunk shapes the real pipeline
    // produced for the opening page of the C book (traced through
    // page_text → geometry breaks → chunker): a >300-char chapter-opener
    // chunk, a mid-chunk section heading, and wrapped body lines.

    #[test]
    fn long_chunk_with_chapter_opener_still_splits() {
        let text = "Chapter 0\r\nIf You\u{2019}re New Here\r\nThis book begins with programs, and it begins quickly. That is the right way to learn C and\r\nthe wrong way to be surprised by it: the first chapter\u{2019}s loops lean on facts \u{2014} about expressions,\r\ndeclarations, and truth \u{2014} that deserve a floor under them before the running starts. Part 0 is\r\nthat floor. ";
        assert!(
            text.chars().count() > 200,
            "test must exercise a long chunk"
        );
        let outline = ["If You\u{2019}re New Here"];
        let pieces = pieces(text, &outline);
        assert_eq!(
            pieces.len(),
            6,
            "expected kicker + title + beat-split body: {pieces:?}"
        );
        let pauses: Vec<u32> = pieces.iter().map(|(_, p, _)| *p).collect();
        assert_eq!(
            pauses,
            vec![
                HEADING_KICKER_PAUSE_MS,
                HEADING_PAUSE_MS,
                COLON_PAUSE_MS,
                DASH_PAUSE_MS,
                DASH_PAUSE_MS,
                SENTENCE_PAUSE_MS
            ]
        );
        assert_eq!(pieces[0].2.trim(), "Chapter 0");
        assert!(pieces[1].2.contains("New Here"));
        assert!(pieces[2].2.trim_end().ends_with("surprised by it"));
        assert!(pieces[5].2.trim_start().starts_with("that deserve"));
    }

    #[test]
    fn parenthetical_dashes_get_a_beat() {
        // The sentence that prompted the feature.
        let text = "Read Part 0 once, end to end \u{2014} an hour \u{2014} because it concentrates exactly the material that trips the experienced. ";
        let pieces = pieces(text, &[]);
        assert_eq!(pieces.len(), 3, "{pieces:?}");
        let pauses: Vec<u32> = pieces.iter().map(|(_, p, _)| *p).collect();
        assert_eq!(
            pauses,
            vec![DASH_PAUSE_MS, DASH_PAUSE_MS, SENTENCE_PAUSE_MS]
        );
        assert!(pieces[0].2.trim_end().ends_with("end to end"));
        assert_eq!(pieces[1].2.trim(), "an hour");
        assert!(pieces[2].2.trim_start().starts_with("because it"));
    }

    #[test]
    fn numeric_ranges_and_hyphens_do_not_split() {
        // En dash between digits is a range; a hyphen-minus is a compound.
        let text = "See pages 3\u{2013}7 for the tables. The read-only buffer held up fine. ";
        let pieces = pieces(text, &[]);
        assert_eq!(pieces.len(), 1, "{pieces:?}");
    }

    #[test]
    fn short_dash_fragments_stay_inline() {
        // Trailing fragment too short to stand alone: no cut at all.
        let text = "It was over \u{2014} soon. ";
        assert_eq!(pieces(text, &[]).len(), 1);
        // Middle fragment too short: only the first dash earns a beat.
        let text = "Everything \u{2014} or so \u{2014} continued as before. ";
        let pieces = pieces(text, &[]);
        assert_eq!(pieces.len(), 2, "{pieces:?}");
    }

    #[test]
    fn semicolons_get_a_beat() {
        // Semicolons used to be dropped by the phoneme tokenizer with no
        // pause at all; they now join two clauses with a 350ms beat.
        let text = "It was quiet for a while; nothing moved after that. The end came slowly. ";
        let pieces = pieces(text, &[]);
        assert_eq!(pieces.len(), 2, "{pieces:?}");
        assert_eq!(pieces[0].1, SEMICOLON_PAUSE_MS);
        assert!(pieces[0].2.trim_end().ends_with("while"));
        assert_eq!(pieces[1].1, SENTENCE_PAUSE_MS);
    }

    #[test]
    fn semicolon_beats_combine_with_dash_beats() {
        let text =
            "First comes the dash \u{2014} a breath; then the semicolon settles it. The end. ";
        let pieces = pieces(text, &[]);
        assert_eq!(pieces.len(), 3, "{pieces:?}");
        let pauses: Vec<u32> = pieces.iter().map(|(_, p, _)| *p).collect();
        assert_eq!(
            pauses,
            vec![DASH_PAUSE_MS, SEMICOLON_PAUSE_MS, SENTENCE_PAUSE_MS]
        );
    }

    #[test]
    fn colons_get_a_beat() {
        // The reported sentence: the colon used to read as one run-on.
        let text = "Finish Part 0, then start Chapter 1 and type in every program it shows: the typing is not a ritual, it is the course. ";
        let pieces = pieces(text, &[]);
        assert_eq!(pieces.len(), 2, "{pieces:?}");
        assert_eq!(pieces[0].1, COLON_PAUSE_MS);
        assert!(pieces[0].2.trim_end().ends_with("it shows"));
        assert!(pieces[1].2.trim_start().starts_with("the typing"));
    }

    #[test]
    fn times_do_not_split_at_colons() {
        let text = "The meeting ran 12:30 to 2:00 and overran badly. Nothing else happened. ";
        let pieces = pieces(text, &[]);
        assert_eq!(pieces.len(), 1, "{pieces:?}");
    }

    #[test]
    fn short_semicolon_fragments_stay_inline() {
        let text = "It stopped; soon. ";
        assert_eq!(pieces(text, &[]).len(), 1);
    }

    #[test]
    fn numbered_heading_line_mid_chunk_splits() {
        let text = "It assumes nothing, it moves slowly by this book\u{2019}s standards, and it exists so that\r\nevery reader \u{2014} newcomer or arriving polyglot \u{2014} starts Chapter 1 on the same ground.\r\n0.1 How to read this book\r\nTwo tracks lead through this book; take the one that fits.\r\nNew to C, or new to programming. ";
        let pieces = pieces(text, &[]);
        // The first body's parenthetical dashes each earn a beat before the
        // heading line splits off with its full pause; the trailing body
        // also splits at its semicolon.
        assert_eq!(
            pieces.len(),
            6,
            "expected dash-split body + heading + semicolon-split body: {pieces:?}"
        );
        let pauses: Vec<u32> = pieces.iter().map(|(_, p, _)| *p).collect();
        assert_eq!(
            pauses,
            vec![
                DASH_PAUSE_MS,
                DASH_PAUSE_MS,
                PARAGRAPH_PAUSE_MS,
                HEADING_PAUSE_MS,
                SEMICOLON_PAUSE_MS,
                SENTENCE_PAUSE_MS
            ]
        );
        assert_eq!(pieces[3].2, "0.1 How to read this book");
        assert!(pieces[4].2.trim_end().ends_with("book"));
        assert!(pieces[5].2.trim_start().starts_with("take the one"));
    }

    #[test]
    fn wrapped_body_lines_never_become_headings() {
        let text = "Read linearly. Finish Part 0, then start Chapter 1\r\nand type in every program it shows \u{2014} the typing is not a ritual, it is the course. The chapters\r\nascend in power, each concept appears before it is used, and every chapter ends with exercises\r\nthat are the other half of its teaching. ";
        let pieces = pieces(text, &[]);
        // The em dash splits the body with a beat; no heading pauses appear.
        assert_eq!(pieces.len(), 2, "body split: {pieces:?}");
        assert_eq!(
            pieces[0].1, DASH_PAUSE_MS,
            "first piece should be a dash beat, not a heading: {pieces:?}"
        );
        assert_eq!(pieces[1].1, SENTENCE_PAUSE_MS);
    }

    #[test]
    fn decimal_lead_line_is_not_a_heading() {
        // A body line starting with a decimal: the follower is lowercase
        // prose, not a capitalized section title.
        let text = "12.5 percent of cases\r\nwere recorded elsewhere in the study. ";
        let pieces = pieces(text, &[]);
        assert_eq!(pieces.len(), 1, "decimal line split off: {pieces:?}");
    }
}

#[cfg(test)]
mod pause_tests {
    use super::*;

    #[test]
    fn pause_durations_match_narration_standards() {
        // Each playback piece carries the pause that follows it: 1.0 s
        // after a heading (chapter kicker or title), 0.8 s after a
        // paragraph, 0.4 s between sentence groups.
        let chars: Vec<char> = "A paragraph of text that ends the block.".chars().collect();
        let n = chars.len();
        let pieces = heading_aware_pieces(&chars, (0, n), &[], &[n]);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].1, PARAGRAPH_PAUSE_MS);

        let chars: Vec<char> = "One sentence here. Another one follows.".chars().collect();
        let n = chars.len();
        let pieces = heading_aware_pieces(&chars, (0, n), &[], &[]);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].1, SENTENCE_PAUSE_MS);
    }

    #[test]
    fn heading_detection_prefers_outline_titles() {
        let outline = vec!["Getting Started".to_string()];
        assert!(is_heading_like("getting started", &outline));
        assert!(!is_heading_like("This is a full sentence.", &outline));
        // Title Case alone is a structural heading cue.
        assert!(is_heading_like("If You're New Here", &outline));
        assert!(!is_heading_like("just some words here", &outline));
        assert!(is_heading_like(
            "If You're New Here",
            &["If You're New Here".to_string()]
        ));
        assert!(is_heading_like("0.1 How to read this book", &outline));
        assert!(is_heading_like("Part 3:", &outline));
    }
}

/// Diagnostic that runs the exact narration pipeline — page text,
/// geometry paragraph breaks, outline titles, sentence chunking, heading
/// split — against a real book and prints what the narrator would play.
/// Opt-in: set BOOKSHELF_TRACE_BOOK (path to a PDF) and optionally
/// BOOKSHELF_TRACE_PAGE (0-based, default 10). CI never sets them.
#[cfg(test)]
mod book_trace_tests {
    use super::*;

    fn escape(text: &str) -> String {
        text.chars()
            .flat_map(|c| match c {
                '\r' => "\r".chars().collect::<Vec<_>>(),
                '\n' => "\n".chars().collect::<Vec<_>>(),
                '\t' => "\t".chars().collect::<Vec<_>>(),
                other => vec![other],
            })
            .collect()
    }

    #[test]
    fn trace_real_book_page() {
        let Ok(book) = std::env::var("BOOKSHELF_TRACE_BOOK") else {
            return;
        };
        let page: i32 = std::env::var("BOOKSHELF_TRACE_PAGE")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(10);
        let library = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/pdfium/pdfium.dll");
        let engine = crate::pdfium_engine::Engine::with_library_path(&library)
            .expect("bind pdfium (run npm run fetch:pdfium)");
        let info = engine.open_document(&book, None).expect("open book");
        let data = engine.page_text(info.doc_id, page).expect("read page text");
        let text = data.chars;
        let breaks = engine
            .paragraph_breaks(info.doc_id, page, text.chars().count())
            .unwrap_or_default();
        let titles = engine.outline_titles_for_page(info.doc_id, page);

        println!("== page {page} ({} chars) ==", text.chars().count());
        println!(
            "first 200 chars: {:?}",
            escape(&text.chars().take(200).collect::<String>())
        );
        println!("outline titles: {titles:?}");
        println!("paragraph breaks: {breaks:?}");

        let chars: Vec<char> = text.chars().collect();
        let chunks = split_for_narration(&text, &breaks);
        println!("== {} chunks ===", chunks.len());
        for (i, (start, end)) in chunks.iter().copied().take(5).enumerate() {
            println!(
                "-- chunk {i} [{start}..{end}] raw: {:?}",
                escape(&chars[start..end].iter().collect::<String>())
            );
            for ((ps, pe), pause, ptext) in
                heading_aware_pieces(&chars, (start, end), &titles, &breaks)
            {
                println!(
                    "   piece [{ps}..{pe}] pause={pause}ms text={:?}",
                    escape(&ptext)
                );
            }
        }

        // Acceptance checks are data-driven so the trace can be pointed at
        // any page of any book: the kicker/title assertions only apply on
        // pages that actually open with a chapter block.
        let first_line: String = chars[..chars
            .iter()
            .position(|c| *c == '\r' || *c == '\n')
            .unwrap_or(chars.len())]
            .iter()
            .collect();
        if is_chapter_kicker(first_line.trim_end()) {
            let first = heading_aware_pieces(&chars, chunks[0], &titles, &breaks);
            assert!(
                first.len() >= 3 && first[0].1 == HEADING_KICKER_PAUSE_MS,
                "chapter kicker not split off: {first:?}"
            );
            assert!(
                first[1].1 == HEADING_PAUSE_MS,
                "chapter title not split off: {first:?}"
            );
        }
        if text.contains("How to read") {
            assert!(
                chunks.iter().any(|&(s, e)| {
                    heading_aware_pieces(&chars, (s, e), &titles, &breaks)
                        .iter()
                        .any(|(_, pause, text)| {
                            *pause == HEADING_PAUSE_MS && text.contains("How to read")
                        })
                }),
                "0.1 heading line not split off"
            );
        }
        // Parenthetical dashes on the page each earn a beat (the book's
        // text changes between builds, so this is data-driven, not pinned
        // to a specific chunk).
        let dash_candidates = chars
            .iter()
            .filter(|c| **c == '\u{2014}' || **c == '\u{2013}')
            .count() as u32;
        let beats: u32 = chunks
            .iter()
            .map(|&(s, e)| {
                heading_aware_pieces(&chars, (s, e), &titles, &breaks)
                    .iter()
                    .filter(|(_, p, _)| *p == DASH_PAUSE_MS)
                    .count() as u32
            })
            .sum();
        println!("dash candidates on page: {dash_candidates}, beats scheduled: {beats}");
        assert!(
            beats >= dash_candidates / 2,
            "expected dash beats for roughly every parenthetical dash (candidates {dash_candidates}, beats {beats})"
        );

        // Homograph audit: every ambiguous word on the page and what the
        // disambiguation table decides for it (None = dictionary default).
        println!("== homograph audit ==");
        let tokens: Vec<&str> = text.split_whitespace().collect();
        for (index, token) in tokens.iter().enumerate() {
            let cleaned: String = token
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
                .to_ascii_lowercase();
            if homograph::ENTRIES.iter().any(|e| e.word == cleaned) {
                let decision = match homograph::resolve(&tokens, index) {
                    Some(homograph::Pronunciation::Respell(r)) => format!("respell {r:?}"),
                    Some(homograph::Pronunciation::Phonemes(p)) => format!("phonemes {p:?}"),
                    None => "default".to_string(),
                };
                println!(
                    "  {token:>14} <- {:?}: {decision}",
                    tokens[index.saturating_sub(1)]
                );
            }
        }
    }
}

#[cfg(test)]
mod code_handling_tests {
    use super::*;

    #[test]
    fn chunks_inside_blocks_classify_as_code() {
        let chunks = vec![(0, 10), (10, 20), (20, 30)];
        let blocks = vec![(10, 20)];
        assert_eq!(
            classify_code_chunks(&chunks, &blocks),
            vec![false, true, false]
        );
    }

    #[test]
    fn partial_overlap_under_sixty_percent_is_prose() {
        let chunks = vec![(0, 100)];
        let blocks = vec![(0, 50)];
        assert_eq!(classify_code_chunks(&chunks, &blocks), vec![false]);
    }

    #[test]
    fn overlapping_blocks_sum_coverage() {
        let chunks = vec![(0, 20)];
        let blocks = vec![(0, 8), (12, 20)];
        // 16/20 = 80% covered
        assert_eq!(classify_code_chunks(&chunks, &blocks), vec![true]);
    }
}

#[cfg(test)]
mod trace_probe {
    use super::*;

    /// Dev probe: runs the exact speak-path text derivation (page text →
    /// paragraph breaks → chunks → heading pieces) on a real book page and
    /// prints every piece, so extraction damage is visible end to end.
    /// Skipped unless BOOKSHELF_TRACE_PDF is set:
    ///   BOOKSHELF_TRACE_PDF="book.pdf" BOOKSHELF_TRACE_PAGE=11 \
    ///     cargo test --lib trace_real_page -- --nocapture
    #[test]
    fn trace_real_page_pieces() {
        let Ok(path) = std::env::var("BOOKSHELF_TRACE_PDF") else {
            eprintln!("skipping: BOOKSHELF_TRACE_PDF not set");
            return;
        };
        let page: i32 = std::env::var("BOOKSHELF_TRACE_PAGE")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(0);
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let lib = [
            manifest.join("resources/pdfium/pdfium.dll"),
            manifest.join("resources/pdfium/libpdfium.dylib"),
            manifest.join("resources/pdfium/libpdfium.so"),
        ]
        .into_iter()
        .find(|p| p.is_file())
        .expect("pdfium library");
        let engine = crate::pdfium_engine::Engine::with_library_path(&lib).unwrap();
        let doc = engine.open_document(&path, None).unwrap();
        let data = engine.page_text(doc.doc_id, page).unwrap();
        let text: String = data.chars.clone();
        let char_count = text.chars().count();
        let breaks = engine
            .paragraph_breaks(doc.doc_id, page, char_count)
            .unwrap_or_default();
        let outline = engine.outline_titles_for_page(doc.doc_id, page);
        eprintln!(
            "page {page}: {char_count} chars, {} breaks, outline={outline:?}",
            breaks.len()
        );
        eprintln!("breaks={breaks:?}");
        let chunks = split_for_narration(&text, &breaks);
        let code_chunks = classify_code_chunks(&chunks, &data.code_blocks);
        let text_chars: Vec<char> = text.chars().collect();
        let preview = |text: &str, max: usize| text.chars().take(max).collect::<String>();
        for (index, (start, end)) in chunks.iter().copied().enumerate() {
            let chunk: String = text_chars[start..end].iter().collect();
            eprintln!(
                "chunk {index} [{start},{end}) code={} {:?}",
                code_chunks[index],
                preview(&chunk, 90)
            );
            for (piece, pause, piece_text) in
                heading_aware_pieces(&text_chars, (start, end), &outline, &breaks)
            {
                eprintln!(
                    "    piece {pause:4}ms [{},{}): {:?}",
                    piece.0,
                    piece.1,
                    preview(&piece_text, 70)
                );
            }
        }
    }
}
