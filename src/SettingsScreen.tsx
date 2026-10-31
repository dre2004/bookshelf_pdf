import { useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ttsPackInstalled, ttsPreviewVoice, ttsVoices } from "./api";
import type { ReadMode, Theme } from "./types";
import { stepped } from "./NarrationBar";
import type { NarrationController } from "./narration";

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
  const [voices, setVoices] = useState<{ id: string; label: string }[]>([]);
  const [packInstalled, setPackInstalled] = useState(true);
  const [sampling, setSampling] = useState<string | null>(null);
  const [sampleError, setSampleError] = useState<string | null>(null);
  const [downloading, setDownloading] = useState(false);

  useEffect(() => {
    void ttsVoices().then(setVoices).catch(() => {});
    void ttsPackInstalled().then(setPackInstalled).catch(() => setPackInstalled(true));
  }, []);

  useEffect(() => {
    const unlisten = listen<{ kind: string; voice?: string }>("kokoro-download", (event) => {
      if (event.payload.kind === "started" || event.payload.kind === "done") {
        setDownloading(event.payload.kind === "started");
      }
      if (event.payload.kind === "done") {
        void ttsPackInstalled().then(setPackInstalled).catch(() => {});
      }
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  const grouped = useMemo(() => {
    const groups = new Map<string, { id: string; label: string }[]>();
    for (const voice of voices) {
      // label format "Name (accent, gender)"
      const match = voice.label.match(/^(.*) \((.*)\)$/);
      const group = match ? match[2] : "Voices";
      if (!groups.has(group)) groups.set(group, []);
      groups.get(group)!.push(voice);
    }
    return [...groups.entries()].sort(([a], [b]) => a.localeCompare(b));
  }, [voices]);

  const startPackDownload = () => narration.download.startKokoro();

  const sample = async (voice: string) => {
    setSampleError(null);
    setSampling(voice);
    try {
      await ttsPreviewVoice(voice, narration.rate);
    } catch (e) {
      setSampleError(String(e));
    } finally {
      setSampling(null);
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

          {!packInstalled && (
            <div className="voice-download">
              <button
                className="btn primary"
                onClick={startPackDownload}
                disabled={downloading || narration.download.kokoroActive}
              >
                {downloading || narration.download.kokoroActive
                  ? `Downloading engine… ${Math.round(narration.download.kokoroProgress * 100)}%`
                  : "⬇ Download narration engine (~87 MB)"}
              </button>
            </div>
          )}

          <div className="voice-catalog">
            {grouped.map(([group, items]) => (
              <div key={group} className="voice-group">
                <h4>{group}</h4>
                {items.map((voice) => {
                  const selected = narration.voiceURI === `kokoro:${voice.id}`;
                  return (
                    <div key={voice.id} className={`voice-row ${selected ? "selected" : ""}`}>
                      <button className="voice-select" onClick={() => narration.setVoiceURI(`kokoro:${voice.id}`)}>
                        <span className="voice-name">{voice.label}</span>
                        {selected && <span className="voice-selected-mark">✓</span>}
                      </button>
                      <button
                        className="btn icon voice-sample"
                        disabled={!packInstalled || sampling !== null}
                        onClick={() => void sample(voice.id)}
                        title="Play a sample"
                      >
                        {sampling === voice.id ? "…" : "▶"}
                      </button>
                    </div>
                  );
                })}
              </div>
            ))}
          </div>
          {sampleError && <p className="muted narration-unsupported">{sampleError}</p>}
        </section>
      </div>
    </div>
  );
}
