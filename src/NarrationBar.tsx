import type { NarrationController } from "./narration";

/** ±15% per step, clamped to 0.50-2.00x, rounded to 2dp. */
export function stepped(rate: number, direction: 1 | -1): number {
  const next = direction === 1 ? rate * 1.15 : rate / 1.15;
  return Math.min(2, Math.max(0.5, Math.round(next * 100) / 100));
}

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
