import type { NarrationController } from "./narration";

interface Props {
  narration: NarrationController;
  currentPage: number;
  onClose: () => void;
}

export default function NarrationPanel({ narration, currentPage, onClose }: Props) {
  const { supported, options, speaking, paused } = narration;
  const onDevice = options.filter((option) => option.engine === "piper");
  const system = options.filter((option) => option.engine === "speech");

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
          <label className="narration-field">
            Voice
            <select
              value={narration.voiceURI}
              onChange={(e) => narration.setVoiceURI(e.target.value)}
              disabled={speaking}
            >
              {onDevice.length > 0 && (
                <optgroup label="On-device (offline)">{onDevice.map((option) => (
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
