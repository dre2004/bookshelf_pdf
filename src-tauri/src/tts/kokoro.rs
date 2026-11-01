//! Kokoro-82M engine: natural neural voices computed on-device through
//! ONNX Runtime. Pack layout (app-data `kokoro/` or dev `engine/kokoro/`):
//! `model.onnx`, `tokenizer.json`, `voices/<name>.bin` (510×256 float32
//! style embeddings).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

use ort::{
    environment::Environment,
    session::{Session, builder::GraphOptimizationLevel},
    value::Tensor,
};
use piper_plus_g2p::english::EnglishPhonemizer;
use serde::Serialize;
use serde_json::Value;
use tauri::Emitter;

/// The process-wide ONNX Runtime environment. ort allows only one
/// `Environment` per process — a second `init().build()` fails even after
/// the first is dropped — so every session shares this one.
static ORT_ENV: Mutex<Option<Environment>> = Mutex::new(None);

fn ort_environment() -> Result<Environment, String> {
    let mut slot = ORT_ENV
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(env) = slot.as_ref() {
        return Ok(env.clone());
    }
    let env = ort::init()
        .build()
        .map_err(|e| format!("ONNX Runtime environment: {e}"))?;
    *slot = Some(env.clone());
    Ok(env)
}

/// The model's synthesis window: token sequences are capped at this length
/// (matching the reference implementations' MAX_PHONEME_LENGTH).
const MAX_TOKENS: usize = 510;
pub const SAMPLE_RATE: u32 = 24_000;

/// Voices shipped by the in-app pack download. Kokoro has 54 presets; this
/// is a curated female-forward set (per project preference).
pub const KOKORO_VOICE_SPECS: &[(&str, &str, &str, &str)] = &[
    // (id, display name, gender, accent)
    ("af_heart", "Heart", "female", "American"),
    ("af_bella", "Bella", "female", "American"),
    ("af_nicole", "Nicole", "female", "American"),
    ("af_nova", "Nova", "female", "American"),
    ("af_sky", "Sky", "female", "American"),
    ("bf_emma", "Emma", "female", "British"),
    ("bf_isabella", "Isabella", "female", "British"),
    ("am_adam", "Adam", "male", "American"),
    ("am_michael", "Michael", "male", "American"),
];

const HF_BASE: &str = "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/resolve/main";

/// Trims leading/trailing near-silence, keeping a small margin so onset
/// consonants are not clipped.
fn trim_silence(samples: &mut Vec<f32>, sample_rate: u32) {
    const THRESHOLD: f32 = 0.005;
    let margin = (sample_rate as usize * 30 / 1000).min(samples.len() / 4);
    let Some(first) = samples.iter().position(|s| s.abs() > THRESHOLD) else {
        samples.clear();
        return;
    };
    let Some(last) = samples.iter().rposition(|s| s.abs() > THRESHOLD) else {
        samples.clear();
        return;
    };
    if first >= last {
        samples.clear();
        return;
    }
    let start = first.saturating_sub(margin);
    let end = (last + 1 + margin).min(samples.len());
    samples.drain(end..);
    samples.drain(..start);
}

/// Maps typographic variants and PDFium extraction artifacts to their
/// ASCII equivalents for G2P. The dictionary rejects exotic characters
/// and falls back to spelling words letter by letter, so this is where
/// "cross<U+0002>references" (PDFium's placeholder for a glyph missing
/// from the font's ToUnicode map) and "cross<U+2011>references" (LaTeX's
/// non-breaking hyphen) become speakable.
fn normalize_typographic(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{2019}' | '\u{2018}' | '\u{02BC}' => '\'',
            '\u{201C}' | '\u{201D}' => '"',
            '\u{2013}' | '\u{2014}' | '\u{2011}' | '\u{2010}' | '\u{FE63}' | '\u{FF0D}' => '-',
            // Control characters survive normalization on purpose: they
            // are PDFium's stand-in for glyphs without a Unicode mapping,
            // and whether such a glyph is a hyphen or invisible spacing
            // is decided per word later (see `resolve_control_artifacts`).
            // \t \r \n are whitespace, so they never end up inside a
            // word token.
            other => other,
        })
        .collect()
}

/// Decides how a control character inside a word token reads. Between
/// letters it is usually a font-switch spacing mark ("assign<ctrl>ment"
/// is the single word "assignment"), but a hyphen glyph can extract the
/// same way ("cross<ctrl>references"). The letter-name fallback tells
/// them apart: "assignment" is a dictionary word and reads with one
/// stress, while "crossreferences" letter-spells — a stress on nearly
/// every letter. A control char not wedged between two letters always
/// takes the hyphen reading.
fn resolve_control_artifacts(token: &str, phonemizer: &EnglishPhonemizer) -> String {
    use piper_plus_g2p::Phonemizer as _;
    let chars: Vec<char> = token.chars().collect();
    if !chars.iter().any(|c| c.is_control()) {
        return token.to_string();
    }
    let hyphenated: String = chars
        .iter()
        .map(|c| if c.is_control() { '-' } else { *c })
        .collect();
    let between_letters = |i: usize| {
        chars
            .get(i.wrapping_sub(1))
            .is_some_and(|c| c.is_alphabetic())
            && chars.get(i + 1).is_some_and(|c| c.is_alphabetic())
    };
    if !chars
        .iter()
        .enumerate()
        .all(|(i, c)| !c.is_control() || between_letters(i))
    {
        return hyphenated;
    }
    let joined: String = chars.iter().filter(|c| !c.is_control()).collect();
    let bare: String = joined
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '\'')
        .collect();
    let joined_reads_cleanly = !bare.is_empty()
        && phonemizer
            .phonemize_with_prosody(&bare)
            .is_ok_and(|(tokens, _)| {
                !crate::tts::lexicon::spells_letter_by_letter(&bare, &tokens.concat())
            });
    if joined_reads_cleanly {
        joined
    } else {
        hyphenated
    }
}

/// Char-level phoneme tokenizer parsed straight from `tokenizer.json`
/// (115 single-character tokens; unknown characters are dropped, exactly
/// what the reference normalizer achieves by stripping them).
struct PhonemeTokenizer {
    vocab: HashMap<char, i64>,
}

impl PhonemeTokenizer {
    fn load(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("reading tokenizer: {e}"))?;
        let config: Value =
            serde_json::from_str(&raw).map_err(|e| format!("parsing tokenizer: {e}"))?;
        let vocab_json = config["model"]["vocab"]
            .as_object()
            .ok_or("tokenizer.json has no model.vocab")?;
        let mut vocab = HashMap::with_capacity(vocab_json.len());
        for (token, id) in vocab_json {
            let mut chars = token.chars();
            if let (Some(c), None, Some(id)) = (chars.next(), chars.next(), id.as_i64()) {
                vocab.insert(c, id);
            }
            // Multi-char tokens do not exist in this vocab; skip defensively.
        }
        Ok(Self { vocab })
    }

    fn encode(&self, text: &str) -> Vec<i64> {
        text.chars()
            .filter_map(|c| self.vocab.get(&c).copied())
            .collect()
    }
}

pub struct KokoroModel {
    session: Mutex<Session>,
    tokenizer: PhonemeTokenizer,
    phonemizer: EnglishPhonemizer,
    /// "tokens" (v1.0 exports) or "input_ids" (newer).
    tokens_input: String,
    /// Whether the style input takes the full (510, 256) embedding or a
    /// 256-dim average — read from the graph instead of assumed.
    style_is_full: bool,
    /// Supplemental-lexicon resolutions, cached per word (None = the
    /// lexicon can't help; the dictionary's fallback stands). One entry
    /// per distinct word ever narrated.
    lexicon_cache: Mutex<HashMap<String, Option<String>>>,
    /// Resolved control-char artifacts ("assign<U+0002>ment"), cached per
    /// raw token.
    artifact_cache: Mutex<HashMap<String, String>>,
}

/// A stretch of words to hand the phonemizer in one piece.
///
/// Words are phonemized as whole runs so the engine hears connected
/// prosody — per-word phonemization made every word an isolated utterance
/// ("If - You're - New - Here"). Runs only break where a homograph or
/// lexicon override supplies raw phonemes that must not be re-phonemized
/// as text.
enum G2pRun<'a> {
    Sentence(Vec<&'a str>),
    Phonemes(&'static str),
    /// A lexicon-derived pronunciation (owned: built at runtime).
    LexiconPhonemes(String),
}

/// Joins runs of single-letter tokens ("i t a l i c" -> "italic").
///
/// Italic and small-caps faces often extract one glyph per "word"; every
/// letter is then an unknown word to the dictionary and the narration
/// spells it. A run of 3+ single-letter alphabetic tokens is virtually
/// always extraction damage. Word gaps survive as the empty tokens that
/// `split(' ')` keeps for repeated spaces, so "t h e   q u i c k"
/// rejoins as two words, not one.
fn repair_letter_spaced(text: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut run: Vec<char> = Vec::new();

    fn end_run(run: &mut Vec<char>, words: &mut Vec<String>) {
        if run.len() >= 3 {
            let joined: String = run.drain(..).collect();
            words.push(joined);
        } else {
            for c in run.drain(..) {
                words.push(c.to_string());
            }
        }
    }

    for token in text.split(' ') {
        if token.is_empty() {
            end_run(&mut run, &mut words);
            continue;
        }
        // Only a bare single alphabetic character joins a letter-run:
        // "J." carries its period (initials), so it passes through.
        let mut chars = token.chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && c.is_alphabetic()
        {
            run.push(c);
            continue;
        }
        end_run(&mut run, &mut words);
        words.push(token.to_string());
    }
    end_run(&mut run, &mut words);
    words
}

fn g2p_runs<'a>(tokens: &[&'a str], lexicon_overrides: &HashMap<usize, String>) -> Vec<G2pRun<'a>> {
    let mut runs = Vec::new();
    let mut batch: Vec<&str> = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        // Lexicon-derived pronunciations win first: the dictionary
        // letter-spells these words, so no other path can help.
        if let Some(ipa) = lexicon_overrides.get(&index) {
            if !batch.is_empty() {
                runs.push(G2pRun::Sentence(std::mem::take(&mut batch)));
            }
            runs.push(G2pRun::LexiconPhonemes(ipa.clone()));
            continue;
        }
        match crate::tts::homograph::resolve(tokens, index) {
            // Raw phonemes splice in as-is; flush the pending sentence so
            // the override never passes through the dictionary.
            Some(crate::tts::homograph::Pronunciation::Phonemes(phonemes)) => {
                if !batch.is_empty() {
                    runs.push(G2pRun::Sentence(std::mem::take(&mut batch)));
                }
                runs.push(G2pRun::Phonemes(phonemes));
            }
            // A respelling is dictionary text ("read" -> "reed"): it joins
            // the run in place, keeping the sentence connected.
            Some(crate::tts::homograph::Pronunciation::Respell(spoken)) => {
                batch.push(spoken);
            }
            None => batch.push(token),
        }
    }
    if !batch.is_empty() {
        runs.push(G2pRun::Sentence(batch));
    }
    runs
}

impl KokoroModel {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let model_path = dir.join("model.onnx");
        if !model_path.is_file() {
            return Err("Kokoro model is not installed".into());
        }
        let env = ort_environment()?;
        let session = Session::builder(&env)
            .map_err(|e| format!("session builder: {e}"))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| format!("optimization level: {e}"))?
            .commit_from_file(&model_path)
            .map_err(|e| format!("loading Kokoro model: {e}"))?;

        let mut tokens_input = None;
        let mut style_is_full = false;
        for input in session.inputs() {
            let name = input.name();
            if name == "input_ids" || name == "tokens" {
                tokens_input = Some(name.to_string());
            }
            if let ("style", ort::value::ValueType::Tensor { shape, .. }) = (name, input.dtype()) {
                // Exports declaring a fixed first dim of 1 expect the
                // 256-dim averaged style vector; [N, 256] exports take
                // the full (510, 256) embedding table.
                style_is_full = shape.len() == 2 && shape.first() != Some(&1);
            }
        }
        let tokens_input = tokens_input.ok_or("Kokoro model has no tokens/input_ids input")?;

        Ok(Self {
            session: Mutex::new(session),
            tokenizer: PhonemeTokenizer::load(&dir.join("tokenizer.json"))?,
            phonemizer: EnglishPhonemizer::new_bundled()
                .map_err(|e| format!("english phonemizer: {e}"))?,
            tokens_input,
            style_is_full,
            lexicon_cache: Mutex::new(HashMap::new()),
            artifact_cache: Mutex::new(HashMap::new()),
        })
    }

    /// Cached `resolve_control_artifacts` — the hot path (words without
    /// artifacts) skips both the lock and the phonemizer.
    fn resolve_token_artifacts(&self, token: String) -> String {
        if !token.chars().any(|c| c.is_control()) {
            return token;
        }
        {
            let cache = self.artifact_cache.lock().unwrap();
            if let Some(hit) = cache.get(&token) {
                return hit.clone();
            }
        }
        let resolved = resolve_control_artifacts(&token, &self.phonemizer);
        self.artifact_cache
            .lock()
            .unwrap()
            .insert(token, resolved.clone());
        resolved
    }

    /// A lexicon-derived pronunciation for `word`, or None. Only words
    /// the primary phonemizer would spell letter by letter are worth
    /// deriving (dictionary words must keep their entries), so the word
    /// is checked against the phonemizer once and the outcome cached.
    fn word_lexicon_override(&self, word: &str) -> Option<String> {
        use piper_plus_g2p::Phonemizer as _;
        {
            let cache = self.lexicon_cache.lock().unwrap();
            if let Some(hit) = cache.get(word) {
                return hit.clone();
            }
        }
        let resolved = (|| {
            let (tokens, _) = self.phonemizer.phonemize_with_prosody(word).ok()?;
            let ipa = tokens.concat();
            if !crate::tts::lexicon::spells_letter_by_letter(word, &ipa) {
                // The dictionary knows this word: no override, and no
                // need to re-check it ever.
                return None;
            }
            crate::tts::lexicon::resolve_with_phonemizer(word, &self.phonemizer)
        })();
        self.lexicon_cache
            .lock()
            .unwrap()
            .insert(word.to_string(), resolved.clone());
        resolved
    }

    /// Loads a voice's style embedding. Files are raw float32 (510, 256);
    /// averaged to 256 dims when the graph expects that form.
    fn style_vector(&self, dir: &Path, voice: &str) -> Result<Vec<f32>, String> {
        let path = dir.join("voices").join(format!("{voice}.bin"));
        let bytes = std::fs::read(&path).map_err(|e| format!("voice '{voice}': {e}"))?;
        let floats: Vec<f32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| f32::from_le_bytes(*chunk))
            .collect();
        if self.style_is_full {
            return Ok(floats);
        }
        let rows = floats.len() / 256;
        if rows == 0 {
            return Ok(floats);
        }
        let mut averaged = vec![0f32; 256];
        for (index, value) in floats.iter().enumerate() {
            averaged[index % 256] += value / rows as f32;
        }
        Ok(averaged)
    }

    /// English text → IPA (word boundaries preserved, sentence punctuation
    /// kept for natural pausing).
    fn phonemize(&self, text: &str) -> Result<String, String> {
        // Nothing-to-say input must stop here: normalization maps control
        // chars to '-' (exotic-hyphen repair), so a bare "\r\n" would
        // otherwise speak as dashes, and lone punctuation as a tick.
        if !text.chars().any(|c| c.is_alphanumeric()) {
            return Ok(String::new());
        }
        // Typographic punctuation must become ASCII before G2P: publishers
        // use U+2019 etc., which breaks dictionary lookups and makes the
        // fallback spell words letter by letter ("you<U+2019>re" is heard
        // as "y-o-u-r-e").
        let text = normalize_typographic(text);
        let repaired: Vec<String> = repair_letter_spaced(&text)
            .into_iter()
            .map(|token| self.resolve_token_artifacts(token))
            .collect();
        let tokens: Vec<&str> = repaired.iter().map(String::as_str).collect();
        use piper_plus_g2p::Phonemizer as _;
        // Words the dictionary would letter-spell get a lexicon-derived
        // pronunciation (curated table or suffix rules); everything else
        // flows through the phonemizer for sentence-level prosody.
        let lexicon_overrides: HashMap<usize, String> = tokens
            .iter()
            .enumerate()
            .filter_map(|(index, word)| self.word_lexicon_override(word).map(|ipa| (index, ipa)))
            .collect();
        let mut parts = Vec::new();
        for run in g2p_runs(&tokens, &lexicon_overrides) {
            match run {
                // Words are phonemized as whole sentences so the engine
                // hears connected prosody — per-word phonemization made
                // every word an isolated utterance ("If - You're - New -
                // Here").
                G2pRun::Sentence(words) => {
                    let (tokens, _) = self
                        .phonemizer
                        .phonemize_with_prosody(&words.join(" "))
                        .map_err(|e| format!("phonemize: {e}"))?;
                    parts.push(tokens.concat());
                }
                // A raw-phoneme homograph override splices in as-is; its
                // IPA must not be re-phonemized as text.
                G2pRun::Phonemes(phonemes) => parts.push(phonemes.to_string()),
                // Same, for pronunciations derived at runtime.
                G2pRun::LexiconPhonemes(ipa) => parts.push(ipa),
            }
        }
        let trailing: String = text
            .chars()
            .rev()
            .take_while(|c| ".,!?;:".contains(*c))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let spoken = parts.join(" ");
        // Nothing to say must produce no phonemes: a bare space is in the
        // tokenizer vocab (id 16), so "format!("{ } { }")" on empties made
        // the model synthesize a one-token blip for empty pieces.
        if spoken.trim().is_empty() {
            return Ok(String::new());
        }
        Ok(format!("{spoken} {trailing}"))
    }

    /// Text → audio samples at 24 kHz. Long windows split at token 0-pad
    /// boundaries like the reference chunker.
    pub fn synthesize(
        &self,
        dir: &Path,
        text: &str,
        voice: &str,
        speed: f32,
    ) -> Result<(Vec<f32>, u32), String> {
        let ipa = self.phonemize(text)?;
        let ids = self.tokenizer.encode(&ipa);
        if ids.is_empty() {
            return Ok((Vec::new(), SAMPLE_RATE));
        }
        let style = self.style_vector(dir, voice)?;

        // Window [0, *ids, 0] over MAX_TOKENS, splitting on overflow.
        let mut audio: Vec<f32> = Vec::new();
        let mut cursor = 0usize;
        loop {
            let remaining = ids.len() - cursor;
            if remaining == 0 {
                break;
            }
            let take = remaining.min(MAX_TOKENS - 2);
            let mut window = Vec::with_capacity(take + 2);
            window.push(0i64);
            window.extend_from_slice(&ids[cursor..cursor + take]);
            window.push(0);
            cursor += take;

            let len = window.len();
            let tokens_tensor = Tensor::from_array(([1_usize, len], window.into_boxed_slice()))
                .map_err(|e| format!("tokens tensor: {e}"))?;
            let style_len = style.len();
            let full_shape = [style_len / 256, 256];
            let flat_shape = [1_usize, style_len];
            let style_shape: &[usize] = if self.style_is_full {
                &full_shape
            } else {
                &flat_shape
            };
            let style_tensor = Tensor::from_array((style_shape, style.clone().into_boxed_slice()))
                .map_err(|e| format!("style tensor: {e}"))?;
            let speed_tensor = Tensor::from_array(([1_usize], vec![speed].into_boxed_slice()))
                .map_err(|e| format!("speed tensor: {e}"))?;

            let inputs: Vec<(
                std::borrow::Cow<'_, str>,
                ort::session::SessionInputValue<'_>,
            )> = vec![
                (self.tokens_input.as_str().into(), (&tokens_tensor).into()),
                ("style".into(), (&style_tensor).into()),
                ("speed".into(), (&speed_tensor).into()),
            ];

            let mut session = self.session.lock().unwrap();
            let outputs = session.run(inputs).map_err(|e| format!("inference: {e}"))?;
            let (_, samples) = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| format!("audio output: {e}"))?;
            // Kokoro pads each window with silence; sentence-sized chunks
            // would stack that dead air at every chunk boundary. Trim the
            // edges (keeping a small margin) so narration flows.
            let mut window = samples.to_vec();
            trim_silence(&mut window, SAMPLE_RATE);
            audio.extend_from_slice(&window);
        }
        Ok((audio, SAMPLE_RATE))
    }
}

/// Resolves the Kokoro pack directory: app-data first, dev checkout second.
pub fn kokoro_dirs(app: &tauri::AppHandle) -> Vec<PathBuf> {
    use tauri::Manager;
    let mut candidates = Vec::new();
    if let Ok(data) = app.path().app_data_dir() {
        candidates.push(data.join("kokoro"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("engine").join("kokoro"));
        candidates.push(cwd.join("..").join("engine").join("kokoro"));
    }
    candidates.into_iter().filter(|dir| dir.is_dir()).collect()
}

/// Downloads the Kokoro pack (model + tokenizer + curated voices) into
/// `dir`, emitting `kokoro-download` events. Reuses the Piper downloader's
/// file fetcher; already-present files are skipped.
pub fn download_kokoro_pack(app: &tauri::AppHandle, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir.join("voices")).map_err(|e| format!("pack dir: {e}"))?;

    #[derive(Serialize, Clone)]
    #[serde(rename_all = "camelCase", tag = "kind")]
    enum Event {
        Started {
            #[serde(rename = "totalBytes")]
            total: u64,
        },
        Progress {
            #[serde(rename = "downloadedBytes")]
            downloaded: u64,
        },
        Done,
        Failed {
            reason: String,
        },
    }
    let emit = |event: Event| {
        let _ = app.emit("kokoro-download", event);
    };

    let mut plans: Vec<(String, PathBuf, u64)> = Vec::new();
    for (url, path) in [
        (
            format!("{HF_BASE}/onnx/model_q8f16.onnx"),
            dir.join("model.onnx"),
        ),
        (
            format!("{HF_BASE}/tokenizer.json"),
            dir.join("tokenizer.json"),
        ),
    ] {
        if path.is_file() {
            continue;
        }
        let size = head_content_length(&url);
        plans.push((url, path, size));
    }

    if plans.is_empty() {
        emit(Event::Done);
        return Ok(());
    }
    let total: u64 = plans.iter().map(|(_, _, size)| *size).sum();
    emit(Event::Started { total });

    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(1200)))
            .build(),
    );
    let mut downloaded = 0u64;
    let mut last_reported = 0u64;
    for (url, path, expected) in plans {
        // Progress events fire per completed file; download_file's own
        // byte-level callback is unused here.
        let mut report = |_bytes: u64| {};
        if let Err(e) = download_file(
            &agent,
            &url,
            &path,
            expected,
            &mut downloaded,
            &mut last_reported,
            &mut report,
        ) {
            let _ = std::fs::remove_file(path.with_extension("part"));
            let reason = format!("downloading {url}: {e}");
            emit(Event::Failed {
                reason: reason.clone(),
            });
            return Err(reason);
        }
        emit(Event::Progress { downloaded });
    }
    emit(Event::Done);
    Ok(())
}

/// Downloads one voice's style file on demand (~0.5 MB). Emits the same
/// `kokoro-download` events so the UI can reflect progress.
pub fn download_voice(app: &tauri::AppHandle, dir: &Path, voice: &str) -> Result<(), String> {
    if !KOKORO_VOICE_SPECS.iter().any(|(id, ..)| *id == voice) {
        return Err(format!("unknown voice '{voice}'"));
    }
    #[derive(Serialize, Clone)]
    #[serde(rename_all = "camelCase", tag = "kind")]
    enum Event {
        VoiceStarted { voice: String },
        VoiceDone { voice: String },
        Failed { reason: String },
    }
    let emit = |event: Event| {
        let _ = app.emit("kokoro-download", event);
    };
    let path = dir.join("voices").join(format!("{voice}.bin"));
    if path.is_file() {
        // Report presence so a setup UI waiting on this voice can settle.
        emit(Event::VoiceDone {
            voice: voice.to_string(),
        });
        return Ok(());
    }
    std::fs::create_dir_all(dir.join("voices")).map_err(|e| format!("voices dir: {e}"))?;
    emit(Event::VoiceStarted {
        voice: voice.to_string(),
    });

    let url = format!("{HF_BASE}/voices/{voice}.bin");
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(120)))
            .build(),
    );
    let expected = head_content_length(&url);
    let mut downloaded = 0u64;
    let mut last_reported = 0u64;
    let mut report = |_bytes: u64| {};
    match download_file(
        &agent,
        &url,
        &path,
        expected,
        &mut downloaded,
        &mut last_reported,
        &mut report,
    ) {
        Ok(()) => {
            emit(Event::VoiceDone {
                voice: voice.to_string(),
            });
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(path.with_extension("part"));
            let reason = format!("downloading voice '{voice}': {e}");
            emit(Event::Failed {
                reason: reason.clone(),
            });
            Err(reason)
        }
    }
}

/// Downloads every curated voice (~0.5 MB each), emitting per-voice
/// `kokoro-download` events and a final `done` so a setup screen can
/// walk the list. Missing files are skipped silently via `download_voice`'s
/// already-present fast path, which still reports each voice.
pub fn download_all_voices(app: &tauri::AppHandle, dir: &Path) -> Result<(), String> {
    for (id, ..) in KOKORO_VOICE_SPECS {
        download_voice(app, dir, id)?;
    }
    #[derive(Serialize, Clone)]
    #[serde(rename_all = "camelCase", tag = "kind")]
    enum Event {
        Done,
    }
    let _ = app.emit("kokoro-download", Event::Done);
    Ok(())
}

/// The sentence spoken when sampling a voice from settings.
pub const SAMPLE_SENTENCE: &str = "This is a sample of how this voice reads your books.";

/// Synthesizes the sample sentence in the given voice and plays it. Used by
/// the settings screen's per-voice preview button.
pub fn play_voice_sample(dir: &Path, voice: &str, speed: f32) -> Result<(), String> {
    let model = KokoroModel::load(dir)?;
    let (samples, sample_rate) = model.synthesize(dir, SAMPLE_SENTENCE, voice, speed)?;
    if samples.is_empty() {
        return Err("voice produced no audio".into());
    }
    // Sample playback is fire-and-forget on its own audio player; the
    // narration engine is untouched.
    std::thread::spawn(move || {
        let Ok(_env) = ort_environment() else {
            return;
        };
        let Ok(sink) = rodio::DeviceSinkBuilder::open_default_sink() else {
            return;
        };
        let player = rodio::Player::connect_new(sink.mixer());
        let (channels, rate) = (
            rodio::ChannelCount::new(1),
            rodio::SampleRate::new(sample_rate),
        );
        if let (Some(channels), Some(rate)) = (channels, rate) {
            player.append(rodio::buffer::SamplesBuffer::new(channels, rate, samples));
            player.sleep_until_end();
        }
    });
    Ok(())
}

fn head_content_length(url: &str) -> u64 {
    ureq::Agent::new_with_defaults()
        .head(url)
        .call()
        .ok()
        .and_then(|response| {
            response
                .headers()
                .get("content-length")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(0)
}

/// Streams `url` to `path` via a `.part` temp file, verifying the expected
/// size when known. `downloaded`/`last_reported` carry cumulative progress
/// across sequential files; `report` fires roughly every 1 MB.
#[allow(clippy::too_many_arguments)]
fn download_file(
    agent: &ureq::Agent,
    url: &str,
    path: &Path,
    expected_size: u64,
    downloaded: &mut u64,
    last_reported: &mut u64,
    report: &mut dyn FnMut(u64),
) -> Result<(), String> {
    use std::io::{Read, Write};

    let mut reader = agent
        .get(url)
        .call()
        .map_err(|e| format!("request failed: {e}"))?
        .into_body()
        .into_reader();

    let temp_path = path.with_extension("part");
    let mut file = std::fs::File::create(&temp_path).map_err(|e| format!("create: {e}"))?;
    let mut buffer = vec![0u8; 256 * 1024];
    let mut written = 0u64;
    loop {
        let read = reader.read(&mut buffer).map_err(|e| format!("read: {e}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|e| format!("write: {e}"))?;
        written += read as u64;
        *downloaded += read as u64;
        // Throttle progress to ~1 MB steps.
        if *downloaded - *last_reported >= 1024 * 1024 {
            *last_reported = *downloaded;
            report(*downloaded);
        }
    }
    drop(file);
    if expected_size > 0 && written != expected_size {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!(
            "size mismatch: got {written}, expected {expected_size}"
        ));
    }
    std::fs::rename(&temp_path, path).map_err(|e| format!("rename: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real-model smoke test; skipped when the pack is not installed (CI).
    #[test]
    fn synthesize_with_installed_pack() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../engine/kokoro");
        let Ok(model) = KokoroModel::load(&dir) else {
            eprintln!("skipping: no Kokoro pack installed");
            return;
        };
        let (audio, sample_rate) = model
            .synthesize(&dir, "Hello world, from Kokoro.", "af_heart", 1.0)
            .unwrap_or_else(|e| panic!("synthesis failed: {e}"));
        assert_eq!(sample_rate, SAMPLE_RATE);
        // ~1.5 s of speech at 24 kHz.
        assert!(
            audio.len() > 20_000,
            "suspiciously little audio: {} samples",
            audio.len()
        );
        assert!(audio.iter().any(|s| s.abs() > 0.01), "audio is silent");
    }
}

#[cfg(test)]
mod g2p_tests {
    use super::*;
    use piper_plus_g2p::Phonemizer as _;

    #[test]
    fn empty_and_whitespace_text_synthesize_nothing() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../engine/kokoro");
        let Ok(model) = KokoroModel::load(&dir) else {
            eprintln!("skipping: no Kokoro pack installed");
            return;
        };
        // Empty pieces used to synthesize a one-token blip: the space is a
        // real vocab entry (id 16), so a " " phoneme string produced audio.
        for text in ["", " ", "\r\n", ". "] {
            let ipa = model.phonemize(text).expect("phonemize");
            assert!(ipa.is_empty(), "{text:?} -> {ipa:?}");
            let (samples, _) = model
                .synthesize(&dir, text, "af_heart", 1.0)
                .expect("synthesize");
            assert!(
                samples.is_empty(),
                "{text:?} produced {} samples",
                samples.len()
            );
        }
    }

    #[test]
    fn contractions_phonemize_correctly() {
        let p = EnglishPhonemizer::new_bundled().expect("phonemizer");
        for word in ["you're", "don't", "it's", "won't"] {
            let (tokens, _) = p.phonemize_with_prosody(word).expect("g2p");
            let joined: String = tokens.concat();
            // Letter-name fallback produces long spelled-out IPA;
            // dictionary hits are short.
            assert!(joined.chars().count() <= 8, "{word} -> {joined}");
        }
    }

    #[test]
    fn typographic_punctuation_is_normalized() {
        assert_eq!(normalize_typographic("you\u{2019}re"), "you're");
        assert_eq!(normalize_typographic("don\u{2019}t"), "don't");
        assert_eq!(
            normalize_typographic("\u{201C}hi\u{201D} \u{2014} ok"),
            "\"hi\" - ok"
        );
    }

    #[test]
    fn trim_silence_removes_dead_air_but_keeps_margin() {
        let mut samples = vec![0.0; 1000];
        for value in samples[400..600].iter_mut() {
            *value = 0.5;
        }
        trim_silence(&mut samples, 24_000);
        // margin = min(30ms, len/4) = min(720, 250) = 250, so the 200-sample
        // burst keeps a 250-sample margin on each side.
        assert_eq!(samples.len(), 700, "unexpected length: {}", samples.len());
        assert!(samples.iter().any(|s| s.abs() > 0.1));

        let mut silent = vec![0.0; 500];
        trim_silence(&mut silent, 24_000);
        assert!(silent.is_empty());
    }

    /// The homograph rules moved to tts::homograph (table-driven, many
    /// words). This keeps the phonemizer-side fact they depend on: the
    /// dictionary's bare "read" is the past tense, so a present-tense
    /// substitution to "reed" audibly matters.
    #[test]
    fn dictionary_read_is_past_tense_so_substitution_matters() {
        use piper_plus_g2p::Phonemizer as _;
        let p = EnglishPhonemizer::new_bundled().expect("phonemizer");
        let (reed, _) = p.phonemize_with_prosody("reed").expect("g2p");
        let (read, _) = p.phonemize_with_prosody("read").expect("g2p");
        assert_eq!(read.concat(), "ɹˈɛd");
        assert_ne!(reed.concat(), read.concat());
    }

    /// The original report: "If You're new here" with a typographic
    /// apostrophe used to spell out letter names.
    #[test]
    fn curly_apostrophe_contraction_phonemizes_like_ascii() {
        let p = EnglishPhonemizer::new_bundled().expect("phonemizer");
        let curly = format!("you{}re", '\u{2019}');
        let normalized = normalize_typographic(&curly);
        let (a, _) = p.phonemize_with_prosody(&normalized).expect("g2p");
        let (b, _) = p.phonemize_with_prosody("you're").expect("g2p");
        assert_eq!(a.concat(), b.concat());
    }

    /// Italic faces often extract one glyph per word; the letters must
    /// rejoin into words before G2P sees them, with widened spaces kept
    /// as word boundaries.
    #[test]
    fn letter_spaced_runs_rejoin_into_words() {
        // Single spaces separate glyphs; the wider gap separates words.
        assert_eq!(
            repair_letter_spaced("t h e   q u i c k   fox"),
            vec!["the".to_string(), "quick".to_string(), "fox".to_string()]
        );
        // Real single-letter words ("a I") never stack three deep.
        assert_eq!(
            repair_letter_spaced("a I saw"),
            vec!["a".to_string(), "I".to_string(), "saw".to_string()]
        );
        // Initials keep their periods (not bare letters) and pass through.
        assert_eq!(repair_letter_spaced("J. R. R. Tolkien").len(), 4);
    }

    /// The reported "c-r-o-s-s-r-e-f-e-r-e-n-c-e-s": the book's hyphen
    /// glyph extracts from PDFium as control character U+0002, the
    /// dictionary rejects the word, and it letter-spells. The non-breaking
    /// hyphen family and the modifier-letter apostrophe fail the same way.
    #[test]
    fn pdfium_placeholder_and_hyphen_variants_phonemize_like_ascii() {
        let p = EnglishPhonemizer::new_bundled().expect("phonemizer");
        let (ascii, _) = p.phonemize_with_prosody("cross-references").expect("g2p");
        let ascii = ascii.concat();
        for variant in [
            format!("cross{}references", '\u{0002}'),
            format!("cross{}references", '\u{2011}'),
            format!("cross{}references", '\u{2010}'),
            format!("cross{}references", '\u{FF0D}'),
        ] {
            let normalized = normalize_typographic(&variant);
            // The U+0002 form goes through the artifact resolver (the
            // joined "crossreferences" letter-spells, so it hyphenates);
            // the typographic hyphens normalize to ASCII directly.
            let resolved = resolve_control_artifacts(&normalized, &p);
            let (got, _) = p.phonemize_with_prosody(&resolved).expect("g2p");
            assert_eq!(
                got.concat(),
                ascii,
                "variant {variant:?} should read like a plain hyphen"
            );
        }
        // And the modifier-letter apostrophe reads like a plain one.
        let (a, _) = p
            .phonemize_with_prosody(&normalize_typographic(&format!("you{}re", '\u{02BC}')))
            .expect("g2p");
        let (b, _) = p.phonemize_with_prosody("you're").expect("g2p");
        assert_eq!(a.concat(), b.concat());
    }

    /// The other face of the artifact: a font-switch spacing mark inside
    /// a word ("assign<U+0002>ment" on book page 12) used to read as
    /// "assign e-m-e-n-t-i-e". The joined form is a dictionary word, so
    /// it joins.
    #[test]
    fn mid_word_control_artifact_joins_when_the_word_is_real() {
        let p = EnglishPhonemizer::new_bundled().expect("phonemizer");
        let raw = format!("In C, assign{}ment is an expression", '\u{0002}');
        let normalized = normalize_typographic(&raw);
        let tokens: Vec<String> = normalized.split(' ').map(str::to_string).collect();
        let resolved: Vec<String> = tokens
            .iter()
            .map(|token| resolve_control_artifacts(token, &p))
            .collect();
        assert!(resolved.contains(&"assignment".to_string()), "{resolved:?}");
        let (a, _) = p.phonemize_with_prosody(&resolved.join(" ")).expect("g2p");
        let (b, _) = p
            .phonemize_with_prosody("In C, assignment is an expression")
            .expect("g2p");
        assert_eq!(a.concat(), b.concat());
    }

    /// A control char that is not wedged between two letters (edge of a
    /// token, next to a digit) always takes the hyphen reading.
    #[test]
    fn control_artifact_outside_letters_hyphenates() {
        let p = EnglishPhonemizer::new_bundled().expect("phonemizer");
        assert_eq!(
            resolve_control_artifacts(&format!("12{}34", '\u{0002}'), &p),
            format!("12-34")
        );
        assert_eq!(
            resolve_control_artifacts(&format!("word{}", '\u{0002}'), &p),
            "word-"
        );
    }

    /// Joined forms the dictionary doesn't know must not be joined: the
    /// stress-per-letter fallback is the decider, not blind joining.
    #[test]
    fn joined_gibberish_is_never_preferred() {
        let p = EnglishPhonemizer::new_bundled().expect("phonemizer");
        let joined_garbage = format!("xyzz{}qwump", '\u{0002}');
        let resolved = resolve_control_artifacts(&joined_garbage, &p);
        assert_eq!(resolved, "xyzz-qwump", "unknown compound must hyphenate");
    }
}
