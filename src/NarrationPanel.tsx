import type { NarrationController } from "./narration";

interface Props {
  narration: NarrationController;
  currentPage: number;
  onClose: () => void;
}

export default function NarrationPanel({ narration, currentPage, onClose }: Props) {
  const { supported, voices, speaking, paused } = narration;

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
          Speech synthesis isn't available in this platform's web view. Narration is supported on
          Windows.
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
              {voices.length === 0 && <option value="">Loading voices…</option>}
              {voices.map((voice) => (
                <option key={voice.voiceURI} value={voice.voiceURI}>
                  {voice.name} ({voice.lang})
                </option>
              ))}
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
