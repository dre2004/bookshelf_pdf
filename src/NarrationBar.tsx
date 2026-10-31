import type { NarrationController } from "./narration";

/** The fixed narration speed ladder. */
export const SPEED_STEPS = [0.5, 0.75, 1.0, 1.25, 2.0, 2.5];

/** Moves to the next/previous ladder step from the current rate. */
export function stepped(rate: number, direction: 1 | -1): number {
  if (direction === 1) {
    return SPEED_STEPS.find((step) => step > rate + 0.001) ?? SPEED_STEPS[SPEED_STEPS.length - 1];
  }
  return [...SPEED_STEPS].reverse().find((step) => step < rate - 0.001) ?? SPEED_STEPS[0];
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
          title="Slower"
        >
          −
        </button>
        <button
          className="btn icon"
          onClick={() => narration.setRate(stepped(narration.rate, 1))}
          title="Faster"
        >
          +
        </button>
      </div>
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
