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
pub mod kokoro;
pub mod voice_downloads;

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

use chunker::split_for_narration;
use kokoro::KokoroModel;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsVoiceInfo {
    /// Selection key: `piper:<stem>` or `kokoro:<voice>`.
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
    loaded: Mutex<HashMap<String, Arc<LoadedVoice>>>,
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

/// Silence appended after a chunk (in samples) so narration breathes:
/// a longer pause after heading-like chunks, a shorter one at paragraph
/// breaks. Returns zero elsewhere — sentence pauses come from prosody.
fn gap_samples(
    is_heading: bool,
    ends_paragraph: bool,
    is_last: bool,
    sample_rate: u32,
) -> Vec<f32> {
    if is_last {
        return Vec::new();
    }
    let milliseconds = if is_heading {
        // Professional audiobook standard: 2.0-2.5 s after a section
        // heading (narratorsroadmap.com standards for silence).
        HEADING_PAUSE_MS
    } else if ends_paragraph {
        // ~0.75-1.0 s standard between paragraphs.
        PARAGRAPH_PAUSE_MS
    } else {
        // ~0.5 s between sentences; the engine's own prosody handles
        // pauses inside a multi-sentence selection.
        SENTENCE_PAUSE_MS
    };
    vec![0.0; sample_rate as usize * milliseconds as usize / 1000]
}

const HEADING_PAUSE_MS: u32 = 2200;
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
    if starts_numbered || trimmed.ends_with(':') {
        return true;
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
            loaded: Mutex::new(HashMap::new()),
            kokoro: Mutex::new(None),
            playback: Mutex::new(PlaybackControl {
                stop: Arc::new(AtomicBool::new(false)),
                player: None,
                clock: Arc::new(Mutex::new(None)),
            }),
            device: Mutex::new(None),
        }
    }

    /// Voice lookup locations, resolved per call: the user's app-data
    /// directory (where downloads install) plus the repo's engine/voices
    /// for dev checkouts. The download happens after startup, so caching
    /// one dir at construction would hide freshly installed voices.
    fn voices_dirs(&self) -> Vec<PathBuf> {
        let mut candidates = Vec::new();
        if let Ok(data) = self.app.path().app_data_dir() {
            candidates.push(data.join("voices"));
        }
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join("engine").join("voices"));
            candidates.push(cwd.join("..").join("engine").join("voices"));
        }
        candidates.into_iter().filter(|dir| dir.is_dir()).collect()
    }

    /// Where downloads install to: the per-user voices directory, created
    /// on demand (distinct from the read-only lookup candidates).
    pub fn install_dir(&self) -> Result<PathBuf, String> {
        let dir = self
            .app
            .path()
            .app_data_dir()
            .map_err(|e| format!("cannot resolve app data dir: {e}"))?
            .join("voices");
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create voices dir: {e}"))?;
        Ok(dir)
    }

    /// The narration voice list: the full Kokoro English catalog (voices
    /// download on demand) plus system voices. Piper is retired from the
    /// catalog per user decision — its engine code remains dormant.
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

    fn voice(&self, id: &str) -> Result<Arc<LoadedVoice>, String> {
        if let Some(voice) = self.loaded.lock().unwrap().get(id) {
            return Ok(voice.clone());
        }
        let Some(model) = self
            .voices_dirs()
            .iter()
            .map(|dir| dir.join(format!("{id}.onnx")))
            .find(|model| model.is_file())
        else {
            return Err(format!("voice '{id}' is not installed"));
        };
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
    pub fn speak(
        &self,
        text: &str,
        page: i32,
        voice_id: &str,
        rate: f32,
        paragraph_breaks: Vec<usize>,
        outline_titles: Vec<String>,
    ) -> Result<(), String> {
        self.stop();

        let text = text.to_string();
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
        let last_chunk = chunks.last().copied().unwrap_or((0, 0));

        if let Some(kokoro_voice) = voice_id.strip_prefix("kokoro:").map(str::to_string) {
            let (dir, model) = self.kokoro_model()?;
            std::thread::spawn(move || {
                for (start, end) in chunks {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let chunk_text: String = text.chars().skip(start).take(end - start).collect();
                    let (samples, sample_rate) =
                        match model.synthesize(&dir, &chunk_text, &kokoro_voice, rate) {
                            Ok(audio) => audio,
                            Err(e) => {
                                let _ = app.emit("tts-event", TtsEvent::Error { reason: e });
                                return;
                            }
                        };
                    if samples.is_empty() {
                        continue;
                    }
                    let channels = rodio::ChannelCount::new(1).expect("nonzero channels");
                    let sample_rate =
                        rodio::SampleRate::new(sample_rate).expect("nonzero sample rate");
                    let mut samples = samples;
                    let chunk_is_heading = {
                        let chunk: String = text_chars[start..end].iter().collect();
                        is_heading_like(chunk.trim(), &outline_titles)
                    };
                    samples.extend(gap_samples(
                        chunk_is_heading,
                        paragraph_breaks.contains(&end.saturating_sub(1))
                            || paragraph_breaks.contains(&end),
                        (start, end) == last_chunk,
                        sample_rate.get(),
                    ));
                    player.append(rodio::buffer::SamplesBuffer::new(
                        channels,
                        sample_rate,
                        samples.clone(),
                    ));
                    // Highlight is scheduled at this chunk's projected play
                    // time; synthesis of the next chunk proceeds while the
                    // audio plays, so narration flows without gaps.
                    schedule(samples, sample_rate, page, Some((start, end)));
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

        let piper_id = voice_id
            .strip_prefix("piper:")
            .unwrap_or(voice_id)
            .to_string();
        let voice = self.voice(&piper_id)?;
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
                let mut samples: Vec<f32> = samples.iter().map(|&s| s as f32 / 32768.0).collect();
                let channels = rodio::ChannelCount::new(1).expect("nonzero channels");
                let sample_rate = rodio::SampleRate::new(sample_rate).expect("nonzero sample rate");
                let chunk_is_heading = {
                    let chunk: String = text_chars[start..end].iter().collect();
                    is_heading_like(chunk.trim(), &outline_titles)
                };
                samples.extend(gap_samples(
                    chunk_is_heading,
                    paragraph_breaks.contains(&end.saturating_sub(1))
                        || paragraph_breaks.contains(&end),
                    (start, end) == last_chunk,
                    sample_rate.get(),
                ));
                player.append(rodio::buffer::SamplesBuffer::new(
                    channels,
                    sample_rate,
                    samples.clone(),
                ));
                schedule(samples, sample_rate, page, Some((start, end)));
            }
            schedule(
                Vec::new(),
                std::num::NonZero::new(24_000).expect("rate"),
                page,
                None,
            );
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

#[cfg(test)]
mod pause_tests {
    use super::*;

    #[test]
    fn pause_durations_match_narration_standards() {
        // 2.2 s after headings, 0.8 s between paragraphs, 0.4 s between
        // sentence groups; never a trailing pause on the final chunk.
        assert_eq!(
            gap_samples(true, false, false, 24_000).len(),
            24_000 * 2200 / 1000
        );
        assert_eq!(
            gap_samples(false, true, false, 24_000).len(),
            24_000 * 800 / 1000
        );
        assert_eq!(
            gap_samples(false, false, false, 24_000).len(),
            24_000 * 400 / 1000
        );
        assert!(gap_samples(true, true, true, 24_000).is_empty());
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
