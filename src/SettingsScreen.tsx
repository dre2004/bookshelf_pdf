import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ttsPackInstalled, ttsPreviewVoice } from "./api";
import type { ReadMode, Theme } from "./types";
import { stepped } from "./NarrationBar";
import { SAMPLE_SENTENCE, speechSupported, type NarrationController } from "./narration";

interface Props {
  onClose: () => void;
  mode: ReadMode;
  onModeChange: (mode: ReadMode) => void;
  theme: Theme;
  onThemeChange: (theme: Theme) => void;
  narration: NarrationController;
}

export default function SettingsScreen({
  onClose,
  mode,
  onModeChange,
  theme,
  onThemeChange,
  narration,
}: Props) {
  const [packInstalled, setPackInstalled] = useState(true);
  const [sampling, setSampling] = useState(false);
  const [sampleError, setSampleError] = useState<string | null>(null);

  useEffect(() => {
    void ttsPackInstalled().then(setPackInstalled).catch(() => setPackInstalled(true));
  }, []);

  useEffect(() => {
    const unlisten = listen<{ kind: string }>("kokoro-download", (event) => {
      if (event.payload.kind === "done") {
        void ttsPackInstalled().then(setPackInstalled).catch(() => {});
      }
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  const onDevice = narration.options.filter((option) => option.engine === "kokoro");
  const system = narration.options.filter((option) => option.engine === "speech");
  const selection = narration.voiceURI || onDevice[0]?.uri || system[0]?.uri || "";

  /** Plays the sample sentence in the selected voice: the on-device engine
   *  synthesizes it (downloading the pack first when missing), system
   *  voices go through the browser's speech synthesis. */
  const playSample = async () => {
    if (!selection) return;
    setSampleError(null);
    setSampling(true);
    try {
      if (selection.startsWith("kokoro:")) {
        await ttsPreviewVoice(selection.slice("kokoro:".length), narration.rate);
      } else if (speechSupported()) {
        const utterance = new SpeechSynthesisUtterance(SAMPLE_SENTENCE);
        const voice = window.speechSynthesis.getVoices().find((v) => v.voiceURI === selection);
        if (voice) {
          utterance.voice = voice;
          utterance.lang = voice.lang;
        }
        utterance.rate = narration.rate;
        window.speechSynthesis.cancel();
        window.speechSynthesis.speak(utterance);
      }
    } catch (e) {
      setSampleError(String(e));
    } finally {
      setSampling(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="settings-screen" onClick={(e) => e.stopPropagation()}>
        <div className="settings-header">
          <h2>Settings</h2>
          <button className="btn icon" onClick={onClose} title="Close">
            ✕
          </button>
        </div>

        <section className="settings-section">
          <h3>Reading</h3>
          <div className="settings-row">
            <span>Page layout</span>
            <div className="segmented">
              <button
                className={mode === "scroll" ? "active" : ""}
                onClick={() => onModeChange("scroll")}
              >
                Continuous scroll
              </button>
              <button
                className={mode === "paged" ? "active" : ""}
                onClick={() => onModeChange("paged")}
              >
                Paged
              </button>
            </div>
          </div>
          <div className="settings-row">
            <span>Theme</span>
            <div className="segmented">
              {(["day", "sepia", "night"] as Theme[]).map((t) => (
                <button key={t} className={theme === t ? "active" : ""} onClick={() => onThemeChange(t)}>
                  {t}
                </button>
              ))}
            </div>
          </div>
        </section>

        <section className="settings-section">
          <h3>Narration</h3>
          <div className="settings-row">
            <span>Voice</span>
            <div className="voice-picker">
              <select
                value={selection}
                onChange={(e) => narration.setVoiceURI(e.target.value)}
                aria-label="Narration voice"
              >
                {onDevice.length > 0 && (
                  <optgroup label="Bookshelf (on-device)">{onDevice.map((option) => (
                    <option key={option.uri} value={option.uri}>
                      {option.label}
                    </option>
                  ))}</optgroup>
                )}
                {system.length > 0 && (
                  <optgroup label="System voices">{system.map((option) => (
                    <option key={option.uri} value={option.uri}>
                      {option.label}
                    </option>
                  ))}</optgroup>
                )}
                {onDevice.length + system.length === 0 && <option value="">Loading voices…</option>}
              </select>
              <button
                className="btn icon"
                onClick={() => void playSample()}
                disabled={sampling || !selection}
                title="Play a sample in the selected voice"
              >
                {sampling ? "…" : "▶"}
              </button>
            </div>
          </div>

          <div className="settings-row">
            <span>Speed</span>
            <div className="speed-stepper">
              <button
                className="btn icon"
                onClick={() => narration.setRate(stepped(narration.rate, -1))}
                title="Slower (-15%)"
              >
                −
              </button>
              <span className="speed-value">{narration.rate.toFixed(2)}×</span>
              <button
                className="btn icon"
                onClick={() => narration.setRate(stepped(narration.rate, 1))}
                title="Faster (+15%)"
              >
                +
              </button>
            </div>
          </div>

          <div className="settings-row">
            <span>Code blocks</span>
            <select
              className="code-handling"
              value={narration.codeHandling}
              onChange={(e) =>
                narration.setCodeHandling(e.target.value as "skip" | "announce")
              }
            >
              <option value="skip">Skip silently</option>
              <option value="announce">Say "Code listing"</option>
            </select>
          </div>

          {!packInstalled && (
            <div className="voice-download">
              <button
                className="btn primary"
                onClick={narration.download.start}
                disabled={narration.download.active}
              >
                {narration.download.active
                  ? `Downloading engine… ${Math.round(narration.download.progress * 100)}%`
                  : "⬇ Download narration engine (~87 MB)"}
              </button>
            </div>
          )}
          {sampleError && <p className="muted narration-unsupported">{sampleError}</p>}
        </section>
      </div>
    </div>
  );
}
