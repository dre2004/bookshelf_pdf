//! Kokoro-82M engine: natural neural voices computed on-device through the
//! same ONNX Runtime as the Piper engine. Pack layout (app-data `kokoro/` or
//! dev `engine/kokoro/`): `model.onnx`, `tokenizer.json`, `voices/<name>.bin`
//! (510×256 float32 style embeddings).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

use ort::{
    session::{Session, builder::GraphOptimizationLevel},
    value::Tensor,
};
use piper_plus_g2p::english::EnglishPhonemizer;
use serde::Serialize;
use serde_json::Value;
use tauri::Emitter;

use super::voice_downloads::download_file;

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

/// Maps smart quotes/apostrophes/dashes to their ASCII equivalents for
/// G2P; unknown characters are dropped by the tokenizer anyway.
fn normalize_typographic(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{2019}' | '\u{2018}' => '\'',
            '\u{201C}' | '\u{201D}' => '"',
            '\u{2013}' | '\u{2014}' => '-',
            other => other,
        })
        .collect()
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
}

impl KokoroModel {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let model_path = dir.join("model.onnx");
        if !model_path.is_file() {
            return Err("Kokoro model is not installed".into());
        }
        let env =
            piper_plus::ort_env::environment().map_err(|e| format!("ort environment: {e}"))?;
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
        })
    }

    /// Loads a voice's style embedding. Files are raw float32 (510, 256);
    /// averaged to 256 dims when the graph expects that form.
    fn style_vector(&self, dir: &Path, voice: &str) -> Result<Vec<f32>, String> {
        let path = dir.join("voices").join(format!("{voice}.bin"));
        let bytes = std::fs::read(&path).map_err(|e| format!("voice '{voice}': {e}"))?;
        let floats: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
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
        // Typographic punctuation must become ASCII before G2P: publishers
        // use U+2019 etc., which breaks dictionary lookups and makes the
        // fallback spell words letter by letter ("you<U+2019>re" is heard
        // as "y-o-u-r-e").
        let text = normalize_typographic(text);
        let mut words = Vec::new();
        for word in text.split_whitespace() {
            use piper_plus_g2p::Phonemizer as _;
            let (tokens, _) = self
                .phonemizer
                .phonemize_with_prosody(word)
                .map_err(|e| format!("phonemize: {e}"))?;
            words.push(tokens.concat());
        }
        let trailing: String = text
            .chars()
            .rev()
            .take_while(|c| ".,!?;:".contains(*c))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        Ok(format!("{} {}", words.join(" "), trailing))
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
            audio.extend_from_slice(samples);
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
    ]
    .into_iter()
    .chain(KOKORO_VOICE_SPECS.iter().map(|(id, ..)| {
        (
            format!("{HF_BASE}/voices/{id}.bin"),
            dir.join("voices").join(format!("{id}.bin")),
        )
    })) {
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
}
