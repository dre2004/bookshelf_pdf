//! One-click download of the default on-device voices from the
//! rhasspy/piper-voices collection into the app's voices directory.

use std::io::Read;

use serde::Serialize;
use tauri::Emitter;

/// Default set installed by the in-app downloader. Voice models carry their
/// own licenses from the piper-voices project.
pub const DEFAULT_VOICE_SPECS: &[(&str, &str, &str)] = &[
    // (locale, name, quality) — verified against the repo's voices.json
    ("es_AR", "daniela", "high"),
    ("en_US", "amy", "medium"),
    ("en_GB", "alba", "medium"),
];

const HF_BASE: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/main";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum VoiceDownloadEvent {
    Started {
        voices: Vec<String>,
        total_bytes: u64,
    },
    Progress {
        downloaded_bytes: u64,
    },
    VoiceDone {
        voice: String,
    },
    Done {
        installed: Vec<String>,
    },
    Failed {
        reason: String,
    },
}

/// Downloads the default voice set into `voices_dir`, emitting
/// `voice-download` events for progress. Runs on the caller's thread —
/// commands spawn a worker for it.
/// Progress/status reported by [`download_default_voices_core`].
pub enum DownloadUpdate {
    Plan(Vec<String>, u64),
    Bytes(u64),
    VoiceDone(String),
}

/// Downloads the default voice set into `voices_dir`, reporting progress
/// through `update`. Network core only — no Tauri types, so it is directly
/// testable. Returns the installed voice stems.
pub fn download_default_voices_core(
    voices_dir: &std::path::Path,
    mut update: impl FnMut(DownloadUpdate),
) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(voices_dir).map_err(|e| format!("voices dir: {e}"))?;

    let mut plans = Vec::new();
    let mut total_bytes = 0u64;
    for &(lang, name, quality) in DEFAULT_VOICE_SPECS {
        let stem = format!("{lang}-{name}-{quality}");
        let model = voices_dir.join(format!("{stem}.onnx"));
        let config = voices_dir.join(format!("{stem}.onnx.json"));
        if model.is_file() && config.is_file() {
            continue;
        }
        // Repo layout: <language>/<locale>/<name>/<quality>/ — the language
        // prefix is the locale's primary subtag (es_AR -> es).
        let language = lang.split('_').next().unwrap_or(lang);
        for (ext, path) in [("onnx", model), ("onnx.json", config)] {
            let url = format!("{HF_BASE}/{language}/{lang}/{name}/{quality}/{stem}.{ext}");
            // Size is only for the progress bar; unknown is fine.
            let size = content_length(&url).unwrap_or(0);
            total_bytes += size;
            plans.push((stem.clone(), url, path, size));
        }
    }

    if plans.is_empty() {
        return Ok(Vec::new());
    }
    let voices: Vec<String> = {
        let mut seen: Vec<String> = plans.iter().map(|(stem, ..)| stem.clone()).collect();
        seen.dedup();
        seen
    };
    update(DownloadUpdate::Plan(voices, total_bytes));

    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(600)))
            .build(),
    );
    let mut downloaded = 0u64;
    let mut installed: Vec<String> = Vec::new();
    let mut last_reported = 0u64;
    for (stem, url, path, size) in plans {
        download_file(
            &agent,
            &url,
            &path,
            size,
            &mut downloaded,
            &mut last_reported,
            &mut |bytes| {
                update(DownloadUpdate::Bytes(bytes));
            },
        )
        .map_err(|e| {
            let _ = std::fs::remove_file(&path);
            format!("downloading {url}: {e}")
        })?;
        update(DownloadUpdate::VoiceDone(stem.clone()));
        if !installed.contains(&stem) {
            installed.push(stem);
        }
    }
    Ok(installed)
}

/// Tauri wrapper: forwards core updates as `voice-download` events on a
/// worker thread; errors also arrive as events.
pub fn download_default_voices(
    app: &tauri::AppHandle,
    voices_dir: &std::path::Path,
) -> Result<(), String> {
    let app = app.clone();
    let voices_dir = voices_dir.to_path_buf();
    std::thread::spawn(move || {
        let result = download_default_voices_core(&voices_dir, |update| {
            let event = match update {
                DownloadUpdate::Plan(voices, total_bytes) => VoiceDownloadEvent::Started {
                    voices,
                    total_bytes,
                },
                DownloadUpdate::Bytes(downloaded_bytes) => {
                    VoiceDownloadEvent::Progress { downloaded_bytes }
                }
                DownloadUpdate::VoiceDone(voice) => VoiceDownloadEvent::VoiceDone { voice },
            };
            let _ = app.emit("voice-download", event);
        });
        match result {
            Ok(installed) => {
                let _ = app.emit("voice-download", VoiceDownloadEvent::Done { installed });
            }
            Err(reason) => {
                let _ = app.emit("voice-download", VoiceDownloadEvent::Failed { reason });
            }
        }
    });
    Ok(())
}

fn content_length(url: &str) -> Result<u64, String> {
    let agent = ureq::Agent::new_with_defaults();
    let response = agent
        .head(url)
        .call()
        .map_err(|e| format!("checking {url}: {e}"))?;
    response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| format!("no content length for {url}"))
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn download_file(
    agent: &ureq::Agent,
    url: &str,
    path: &std::path::Path,
    expected_size: u64,
    downloaded: &mut u64,
    last_reported: &mut u64,
    report: &mut dyn FnMut(u64),
) -> Result<(), String> {
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
        use std::io::Write;
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
mod network_tests {
    use super::*;

    /// Hits the real registry with the same ureq call path as the
    /// downloader. Skipped when offline (CI).
    #[test]
    fn fetch_voice_config_over_https() {
        let url = format!("{HF_BASE}/es/es_AR/daniela/high/es_AR-daniela-high.onnx.json");
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_global(Some(std::time::Duration::from_secs(30)))
                .build(),
        );
        let mut body = String::new();
        let outcome = agent.get(&url).call().and_then(|response| {
            response
                .into_body()
                .into_reader()
                .take(200)
                .read_to_string(&mut body)
                .map_err(|e| e.into())
        });
        match outcome {
            Ok(_) => assert!(body.contains("phoneme_id_map"), "unexpected body: {body}"),
            Err(e) => {
                eprintln!("skipping (network unavailable): {e}");
            }
        }
    }

    /// Full download of the default set into a temp dir — the same core
    /// the app uses. Skips (passes) when the network is unavailable.
    #[test]
    fn download_core_installs_default_voices() {
        let dir = std::env::temp_dir().join(format!("bs-voice-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut last_bytes = 0u64;
        let result = download_default_voices_core(&dir, |update| {
            if let DownloadUpdate::Bytes(bytes) = update {
                last_bytes = bytes;
            }
        });
        match result {
            Ok(installed) => {
                assert_eq!(
                    installed,
                    vec![
                        "es_AR-daniela-high",
                        "en_US-amy-medium",
                        "en_GB-alba-medium"
                    ]
                );
                for stem in &installed {
                    let model = dir.join(format!("{stem}.onnx"));
                    let config = dir.join(format!("{stem}.onnx.json"));
                    assert!(model.is_file(), "{} missing", model.display());
                    assert!(config.is_file(), "{} missing", config.display());
                    let size = std::fs::metadata(&model).unwrap().len();
                    assert!(size > 1_000_000, "{stem} suspiciously small: {size}");
                }
                assert!(
                    last_bytes > 200_000_000,
                    "progress bytes look wrong: {last_bytes}"
                );
            }
            Err(e) => {
                eprintln!("skipping (network unavailable): {e}");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The .onnx redirects to the xet CDN; some CDNs reject default
    /// library user-agents. Reads the first chunk of the real model file.
    #[test]
    fn fetch_model_head_over_https() {
        let url = format!("{HF_BASE}/es/es_AR/daniela/high/es_AR-daniela-high.onnx");
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_global(Some(std::time::Duration::from_secs(60)))
                .build(),
        );
        let mut buffer = vec![0u8; 64 * 1024];
        let outcome = agent
            .get(&url)
            .call()
            .map_err(|e| e.to_string())
            .and_then(|response| {
                let mut reader = response.into_body().into_reader();
                use std::io::Read;
                reader.read(&mut buffer).map_err(|e| e.to_string())
            });
        match outcome {
            Ok(read) => assert!(read > 1024, "only {read} bytes"),
            Err(e) => eprintln!("skipping (network unavailable): {e}"),
        }
    }

    #[test]
    fn head_request_reports_size() {
        let url = format!("{HF_BASE}/es/es_AR/daniela/high/es_AR-daniela-high.onnx.json");
        match content_length(&url) {
            Ok(size) => assert!(size > 1000, "size was {size}"),
            Err(e) => eprintln!("skipping (network unavailable): {e}"),
        }
    }
}
