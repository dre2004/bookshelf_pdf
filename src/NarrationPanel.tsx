import type { NarrationController } from "./narration";

interface Props {
  narration: NarrationController;
  currentPage: number;
  onClose: () => void;
}

export default function NarrationPanel({ narration, currentPage, onClose }: Props) {
  const { supported, options, speaking, paused } = narration;
  const kokoro = options.filter((option) => option.engine === "kokoro");
  const piper = options.filter((option) => option.engine === "piper");
  const system = options.filter((option) => option.engine === "speech");
  const kokoroPackReady = kokoro.length > 0;

  return (
    <div className="narration-panel">
      <div className="narration-header">
        <span className="narration-title">🔊 Narration</span>
        <button className="btn icon" onClick={onClose} title="Close">
          ✕
        </button>
      </div>

      {!supported ? (
        <p className="muted narration-unsupported">
          No narration engines available: this web view lacks speech synthesis and no on-device
          voices are installed.
        </p>
      ) : (
        <>
          {!kokoroPackReady && (
            <div className="voice-download">
              <button
                className="btn primary"
                onClick={narration.download.startKokoro}
                disabled={narration.download.kokoroActive}
              >
                {narration.download.kokoroActive
                  ? `Downloading Kokoro voices… ${narration.download.kokoroProgress}%`
                  : "⬇ Download natural neural voices (~115 MB)"}
              </button>
              <p className="muted narration-hint">
                High-quality offline neural voices (Heart, Bella and more) — recommended over the
                classic Piper set.
              </p>
            </div>
          )}
          {kokoroPackReady && piper.length === 0 && (
            <div className="voice-download">
              <button
                className="btn primary"
                onClick={narration.download.start}
                disabled={narration.download.active}
              >
                {narration.download.active
                  ? narration.download.knownTotal
                    ? `Downloading voices… ${Math.round(narration.download.progress * 100)}%`
                    : `Downloading voices… ${narration.download.downloadedMb.toFixed(0)} MB`
                  : "⬇ Download on-device voices (~240 MB)"}
              </button>
              {narration.download.active && (
                <div className="progress-track">
                  <div
                    className="progress-fill"
                    style={{ width: `${narration.download.progress * 100}%` }}
                  />
                </div>
              )}
              {narration.download.error && (
                <p className="muted narration-unsupported">{narration.download.error}</p>
              )}
              <p className="muted narration-hint">
                Offline neural voices (Daniela + English) downloaded once into your app data —
                then narration works entirely on your machine.
              </p>
            </div>
          )}
          <label className="narration-field">
            Voice
            <select
              value={narration.voiceURI}
              onChange={(e) => narration.setVoiceURI(e.target.value)}
              disabled={speaking}
            >
              {kokoro.length > 0 && (
                <optgroup label="Natural neural (Kokoro)">{kokoro.map((option) => (
                  <option key={option.uri} value={option.uri}>
                    {option.label}
                  </option>
                ))}</optgroup>
              )}
              {piper.length > 0 && (
                <optgroup label="Piper voices">{piper.map((option) => (
                  <option key={option.uri} value={option.uri}>
                    {option.label}
                  </option>
                ))}</optgroup>
              )}
              {system.length > 0 && (
                <optgroup label="System">{system.map((option) => (
                  <option key={option.uri} value={option.uri}>
                    {option.label}
                  </option>
                ))}</optgroup>
              )}
              {options.length === 0 && <option value="">Loading voices…</option>}
            </select>
          </label>

          <label className="narration-field">
            Speed&nbsp;&nbsp;{narration.rate.toFixed(2)}×
            <input
              type="range"
              min={0.5}
              max={2}
              step={0.05}
              value={narration.rate}
              onChange={(e) => narration.setRate(Number(e.target.value))}
            />
          </label>

          <label className="narration-check">
            <input
              type="checkbox"
              checked={narration.continuous}
              onChange={(e) => narration.setContinuous(e.target.checked)}
            />
            Auto-advance pages
          </label>

          <div className="narration-controls">
            {speaking && !paused ? (
              <button className="btn" onClick={narration.pause}>
                ⏸ Pause
              </button>
            ) : speaking && paused ? (
              <button className="btn" onClick={narration.resume}>
                ▶ Resume
              </button>
            ) : (
              <button className="btn primary" onClick={() => narration.play(currentPage)}>
                ▶ Read from page {currentPage + 1}
              </button>
            )}
            <button className="btn" onClick={narration.stop} disabled={!speaking}>
              ⏹ Stop
            </button>
          </div>

          {speaking && (
            <p className="muted narration-status">
              Reading page {(narration.page ?? currentPage) + 1}
            </p>
          )}
        </>
      )}
    </div>
  );
}
