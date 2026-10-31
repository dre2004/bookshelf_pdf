import type { NarrationController } from "./narration";

interface Props {
  narration: NarrationController;
  currentPage: number;
  pageCount: number;
  onPageChange: (page: number) => void;
}

export default function NarrationBar({
  narration,
  currentPage,
  pageCount,
  onPageChange,
}: Props) {
  if (!narration.speaking) {
    return null;
  }

  const sampleVoice = narration.options.find(
    (option) => option.uri === narration.voiceURI,
  );
  const voiceName = sampleVoice?.label ?? "Narrating";
  // Rough remaining estimate: pages left × a conservative 45 s/page spoken.
  const minutesLeft = Math.round(((pageCount - currentPage) * 45) / 60);

  return (
    <div className="narration-bar">
      <button
        className="btn icon"
        onClick={narration.paused ? narration.resume : narration.pause}
        title={narration.paused ? "Resume (click)" : "Pause"}
      >
        {narration.paused ? "▶" : "⏸"}
      </button>
      <button className="btn icon" onClick={narration.stop} title="Stop narration">
        ⏹
      </button>
      <span className="narration-voice">{voiceName}</span>
      <span className="muted">
        page {(narration.page ?? currentPage) + 1} / {pageCount} · ~{minutesLeft} min left
      </span>
      <input
        className="narration-speed"
        type="range"
        min={0.5}
        max={2}
        step={0.05}
        value={narration.rate}
        onChange={(e) => narration.setRate(Number(e.target.value))}
        title="Narration speed"
      />
      <span className="muted">{narration.rate.toFixed(2)}×</span>
      <button
        className="btn icon"
        onClick={() => onPageChange((narration.page ?? currentPage) - 1)}
        title="Previous page"
      >
        ‹
      </button>
      <button
        className="btn icon"
        onClick={() => onPageChange((narration.page ?? currentPage) + 1)}
        title="Next page"
      >
        ›
      </button>
    </div>
  );
}
