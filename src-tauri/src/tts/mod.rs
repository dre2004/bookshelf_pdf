//! On-device text-to-speech for classic Piper voices (rhasspy/piper-voices
//! `.onnx` + `.onnx.json` pairs): ONNX inference through piper-plus's
//! engine, phonemization through its pure-Rust G2P (no espeak-ng, no
//! Python), playback through rodio. Voices load lazily on first use.
//!
//! piper-plus's high-level `PiperVoice` rejects classic voices
//! (`phoneme_type: espeak`), so this module drives its public low-level
//! pieces directly and replicates classic Piper's phoneme-id construction:
//! ids from the model's own map, a pad after every phoneme, BOS prefix and
//! EOS suffix (`^`, `_`, `$` tokens in the map).

mod chunker;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use piper_plus::{OnnxEngine, SynthesisRequest, VoiceConfig};
use serde::Serialize;
use serde_json::Value;
use tauri::{Emitter, Manager};

use chunker::split_for_tts;

/// Character-count ceiling per synthesis call; keeps first-audio latency
/// low and matches how the models were trained (sentence-ish units).
const MAX_CHUNK_CHARS: usize = 220;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsVoiceInfo {
    /// File stem, e.g. `es_AR-daniela-high`.
    pub id: String,
    /// Human-friendly label, e.g. `Daniela — Spanish (Argentina)`.
    pub label: String,
    /// eSpeak locale from the model config, e.g. `es_AR`.
    pub lang: String,
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

struct LoadedVoice {
    engine: Mutex<OnnxEngine>,
    phonemizer: Box<dyn piper_plus_g2p::Phonemizer>,
    id_map: piper_plus_g2p::PhonemeIdMap,
    noise_scale: f32,
    length_scale: f32,
    noise_w: f32,
}

pub struct TtsEngine {
    app: tauri::AppHandle,
    voices_dir: PathBuf,
    loaded: Mutex<HashMap<String, Arc<LoadedVoice>>>,
    playback: Mutex<PlaybackControl>,
    device: Mutex<Option<rodio::MixerDeviceSink>>,
}

struct PlaybackControl {
    stop: Arc<AtomicBool>,
    player: Option<Arc<rodio::Player>>,
}

impl TtsEngine {
    pub fn new(app: tauri::AppHandle) -> Self {
        let voices_dir = Self::resolve_voices_dir(&app);
        Self {
            app,
            voices_dir,
            loaded: Mutex::new(HashMap::new()),
            playback: Mutex::new(PlaybackControl {
                stop: Arc::new(AtomicBool::new(false)),
                player: None,
            }),
            device: Mutex::new(None),
        }
    }

    fn resolve_voices_dir(app: &tauri::AppHandle) -> PathBuf {
        let mut candidates = Vec::new();
        if let Ok(data) = app.path().app_data_dir() {
            candidates.push(data.join("voices"));
        }
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join("engine").join("voices"));
            candidates.push(cwd.join("..").join("engine").join("voices"));
        }
        candidates
            .into_iter()
            .find(|dir| dir.is_dir())
            .unwrap_or_else(|| PathBuf::from("engine/voices"))
    }

    /// Lists installed voice pairs in the voices directory.
    pub fn list_voices(&self) -> Vec<TtsVoiceInfo> {
        let Ok(entries) = std::fs::read_dir(&self.voices_dir) else {
            return Vec::new();
        };
        let mut voices = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(id) = file_stem_of_model(&path) else {
                continue;
            };
            voices.push(voice_info(&id, &path));
        }
        voices.sort_by(|a, b| a.id.cmp(&b.id));
        voices
    }

    fn voice(&self, id: &str) -> Result<Arc<LoadedVoice>, String> {
        if let Some(voice) = self.loaded.lock().unwrap().get(id) {
            return Ok(voice.clone());
        }
        let model = self.voices_dir.join(format!("{id}.onnx"));
        if !model.is_file() {
            return Err(format!("voice '{id}' is not installed"));
        }
        let voice = Arc::new(load_voice(&model)?);
        self.loaded
            .lock()
            .unwrap()
            .insert(id.to_string(), voice.clone());
        Ok(voice)
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

    pub fn stop(&self) {
        let mut playback = self.playback.lock().unwrap();
        playback.stop.store(true, Ordering::Relaxed);
        if let Some(player) = playback.player.take() {
            player.clear();
            player.stop();
        }
    }

    pub fn pause(&self) {
        if let Some(player) = self.playback.lock().unwrap().player.as_ref() {
            player.pause();
        }
    }

    pub fn resume(&self) {
        if let Some(player) = self.playback.lock().unwrap().player.as_ref() {
            player.play();
        }
    }

    /// Speaks `text` (a whole page) chunk by chunk on a worker thread,
    /// emitting progress events as each chunk starts playing. Returns once
    /// the worker is launched; playback continues in the background.
    pub fn speak(&self, text: &str, page: i32, voice_id: &str, rate: f32) -> Result<(), String> {
        self.stop();

        let text = text.to_string();
        let player = self.player()?;

        let stop = {
            let mut playback = self.playback.lock().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            playback.stop = stop.clone();
            playback.player = Some(player.clone());
            stop
        };

        let voice = self.voice(voice_id)?;
        let chunks = split_for_tts(&text, MAX_CHUNK_CHARS);
        let rate = rate.clamp(0.5, 2.0);
        let app = self.app.clone();

        std::thread::spawn(move || {
            for (start, end) in chunks {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let chunk_text: String = text.chars().skip(start).take(end - start).collect();
                let synthesized = {
                    let mut engine = voice.engine.lock().unwrap();
                    synthesize_with_config(&mut engine, &voice, &chunk_text, rate)
                };
                let (samples, sample_rate) = match synthesized {
                    Ok(audio) => audio,
                    Err(e) => {
                        let _ = app.emit(
                            "tts-event",
                            TtsEvent::Error {
                                reason: e.to_string(),
                            },
                        );
                        return;
                    }
                };
                if samples.is_empty() {
                    continue;
                }
                // Piper yields i16 PCM; rodio's Sample type is f32.
                let samples: Vec<f32> = samples.iter().map(|&s| s as f32 / 32768.0).collect();
                let channels = rodio::ChannelCount::new(1).expect("nonzero channels");
                let sample_rate = rodio::SampleRate::new(sample_rate).expect("nonzero sample rate");
                player.append(rodio::buffer::SamplesBuffer::new(
                    channels,
                    sample_rate,
                    samples,
                ));
                let _ = app.emit(
                    "tts-event",
                    TtsEvent::Progress {
                        page,
                        start: start as i64,
                        end: end as i64,
                    },
                );
            }
            let _ = app.emit("tts-event", TtsEvent::PageDone { page });
        });
        Ok(())
    }
}

/// Classic-Piper synthesis: text → G2P tokens → model id map (skipping
/// unknown phonemes, as classic Piper does) → pad/BOS/EOS wrapping → ONNX.
fn synthesize_with_config(
    engine: &mut OnnxEngine,
    voice: &LoadedVoice,
    text: &str,
    rate: f32,
) -> Result<(Vec<i16>, u32), String> {
    let (tokens, _) = voice
        .phonemizer
        .phonemize_with_prosody(text)
        .map_err(|e| format!("phonemize: {e}"))?;

    let mut ids: Vec<i64> = Vec::with_capacity(tokens.len() * 2);
    for token in &tokens {
        if let Some(token_ids) = voice.id_map.get(token) {
            ids.extend_from_slice(token_ids);
        }
    }
    if ids.is_empty() {
        return Ok((Vec::new(), 0));
    }

    let id_count = ids.len();
    let (ids, _) = piper_plus_g2p::multilingual::default_post_process_ids(
        ids,
        vec![None; id_count],
        &voice.id_map,
        "$",
    );

    let request = SynthesisRequest {
        phoneme_ids: ids,
        noise_scale: voice.noise_scale,
        // length_scale multiplies phoneme duration, so it is the inverse of
        // the playback rate.
        length_scale: voice.length_scale / rate,
        noise_w: voice.noise_w,
        ..Default::default()
    };
    let result = engine
        .synthesize(&request)
        .map_err(|e| format!("inference: {e}"))?;
    Ok((result.audio, result.sample_rate))
}

/// Loads a classic Piper voice: ONNX session + language phonemizer chosen
/// by the config's espeak locale, plus its synthesis scales.
fn load_voice(model: &Path) -> Result<LoadedVoice, String> {
    let json_path = model.with_file_name(format!(
        "{}.json",
        model.file_name().unwrap_or_default().to_string_lossy()
    ));
    let raw: Value = serde_json::from_str(
        &std::fs::read_to_string(&json_path)
            .map_err(|e| format!("reading {}: {e}", json_path.display()))?,
    )
    .map_err(|e| format!("parsing {}: {e}", json_path.display()))?;

    let espeak_voice = raw["espeak"]["voice"].as_str().unwrap_or("en");
    let lang = espeak_voice.split(['-', '_']).next().unwrap_or("en");
    let phonemizer: Box<dyn piper_plus_g2p::Phonemizer> = match lang {
        "en" => Box::new(
            piper_plus_g2p::english::EnglishPhonemizer::new_bundled()
                .map_err(|e| format!("english phonemizer: {e}"))?,
        ),
        "es" => Box::new(piper_plus_g2p::spanish::SpanishPhonemizer),
        "fr" => Box::new(piper_plus_g2p::french::FrenchPhonemizer::new()),
        "pt" => Box::new(piper_plus_g2p::portuguese::PortuguesePhonemizer::new()),
        "sv" => Box::new(piper_plus_g2p::swedish::SwedishPhonemizer::new()),
        other => {
            return Err(format!(
                "on-device narration doesn't support language '{other}' yet"
            ));
        }
    };

    let config = VoiceConfig::load(&json_path).map_err(|e| format!("voice config: {e}"))?;
    let engine = OnnxEngine::load(model, &config, "cpu")
        .map_err(|e| format!("loading {}: {e}", model.display()))?;

    // Classic Piper's defaults when the config omits the inference block.
    let inference = &raw["inference"];
    Ok(LoadedVoice {
        engine: Mutex::new(engine),
        phonemizer,
        id_map: config.phoneme_id_map,
        noise_scale: inference["noise_scale"].as_f64().unwrap_or(0.667) as f32,
        length_scale: inference["length_scale"].as_f64().unwrap_or(1.0) as f32,
        noise_w: inference["noise_w"].as_f64().unwrap_or(0.8) as f32,
    })
}

/// Model files only: a `.onnx` whose `.onnx.json` config sits beside it
/// (piper-plus drops optimized copies without configs — ignore those).
fn file_stem_of_model(path: &Path) -> Option<String> {
    if path.extension().and_then(|e| e.to_str()) != Some("onnx") {
        return None;
    }
    let name = path.file_name()?.to_str()?;
    if !path.with_file_name(format!("{name}.json")).is_file() {
        return None;
    }
    Some(name.strip_suffix(".onnx")?.to_string())
}

/// Friendly metadata for a voice file stem like `es_AR-daniela-high`.
fn voice_info(id: &str, model_path: &Path) -> TtsVoiceInfo {
    let json_path = model_path.with_file_name(format!("{id}.onnx.json"));
    let mut lang = id.split('-').next().unwrap_or("??").to_string();
    if let Some(espeak) = std::fs::read_to_string(&json_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|config| config["espeak"]["voice"].as_str().map(str::to_string))
    {
        lang = espeak.replace('-', "_");
    }
    let parts: Vec<&str> = id.split('-').collect();
    let label = if parts.len() >= 2 {
        format!(
            "{} — {}{}",
            capitalize(parts[1]),
            language_name(&lang),
            parts
                .last()
                .filter(|q| !q.is_empty())
                .map(|q| format!(" ({q})"))
                .unwrap_or_default()
        )
    } else {
        id.to_string()
    };
    TtsVoiceInfo {
        id: id.to_string(),
        label,
        lang,
    }
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn language_name(locale: &str) -> String {
    match locale.split('_').next().unwrap_or(locale) {
        "en" => "English".into(),
        "es" => "Spanish".into(),
        "pt" => "Portuguese".into(),
        "fr" => "French".into(),
        "de" => "German".into(),
        "it" => "Italian".into(),
        "sv" => "Swedish".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real-model smoke test across every installed voice; skipped when no
    /// voices are installed (e.g. CI).
    #[test]
    fn synthesize_with_installed_voices() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let voices_dir = manifest_dir.join("../engine/voices");
        let Ok(entries) = std::fs::read_dir(&voices_dir) else {
            eprintln!("skipping: no voices directory");
            return;
        };
        let models: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| file_stem_of_model(p).is_some())
            .collect();
        if models.is_empty() {
            eprintln!("skipping: no voice models installed");
            return;
        }
        for model in models {
            let voice = load_voice(&model)
                .unwrap_or_else(|e| panic!("voice load failed for {}: {e}", model.display()));
            let text = if voice_info(&file_stem_of_model(&model).unwrap(), &model)
                .lang
                .starts_with("es")
            {
                "Hola mundo."
            } else {
                "Hello world."
            };
            let (audio, sample_rate) =
                synthesize_with_config(&mut voice.engine.lock().unwrap(), &voice, text, 1.0)
                    .unwrap_or_else(|e| panic!("synthesis failed for {}: {e}", model.display()));
            assert!(sample_rate > 0, "{}: bad sample rate", model.display());
            assert!(!audio.is_empty(), "{}: no audio produced", model.display());
        }
    }
}
