import type { ReactNode } from "react";
import type { NarrationController } from "./narration";

/** `87.3 MB`-style byte formatting for the progress readout. */
export function formatMB(bytes: number): string {
  return `${(bytes / 1_000_000).toFixed(1)} MB`;
}

interface Props {
  narration: NarrationController;
  /** Modal variant: how to close the card. Inline (settings) omits it. */
  onDismiss?: () => void;
}

/**
 * The one-time on-device narration download: what it is, live progress
 * with sizes, and an explicit ready state. Rendered as a modal on first
 * boot and from the Narrate button, and inline in settings.
 */
export default function NarrationSetup({ narration, onDismiss }: Props) {
  const { setup } = narration;
  if (setup.phase === "idle" && narration.packInstalled) {
    return null;
  }

  const voiceLabel = (id: string): string => {
    const match = narration.options.find((option) => option.uri === `kokoro:${id}`);
    return match?.label ?? id;
  };

  let body: ReactNode;
  if (setup.phase === "idle") {
    body = (
      <>
        <p className="setup-copy">
          Read-aloud narration runs entirely on your machine — nothing is
          sent anywhere. It needs a one-time download: the speech engine
          (about 87 MB) plus nine voices (about 5 MB).
        </p>
        <div className="setup-actions">
          {onDismiss && (
            <button className="btn" onClick={onDismiss}>
              Not now
            </button>
          )}
          <button className="btn primary" onClick={setup.start}>
            ⬇ Download and set up
          </button>
        </div>
      </>
    );
  } else if (setup.phase === "engine") {
    const percent =
      setup.totalBytes > 0
        ? Math.min(100, Math.round((setup.downloadedBytes / setup.totalBytes) * 100))
        : null;
    body = (
      <>
        <p className="setup-status">
          Downloading the narration engine
          {setup.totalBytes > 0 && (
            <>
              {" — "}
              {formatMB(setup.downloadedBytes)} of {formatMB(setup.totalBytes)}
              {percent != null && ` (${percent}%)`}
            </>
          )}
        </p>
        <div
          className="setup-progress"
          role="progressbar"
          aria-valuenow={percent ?? undefined}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-label="Narration engine download"
        >
          <div
            className="setup-progress-fill"
            style={{ width: `${percent ?? 100}%` }}
            data-unknown={percent == null ? "true" : undefined}
          />
        </div>
        <p className="setup-hint">You can keep reading while it downloads.</p>
      </>
    );
  } else if (setup.phase === "voices") {
    body = (
      <>
        <p className="setup-status">
          Fetching voices — {Math.min(setup.voicesFetched, setup.voicesTotal)} /{" "}
          {setup.voicesTotal}
          {setup.voice && ` · ${voiceLabel(setup.voice)}`}
        </p>
        <div
          className="setup-progress"
          role="progressbar"
          aria-valuenow={setup.voicesFetched}
          aria-valuemin={0}
          aria-valuemax={setup.voicesTotal}
          aria-label="Voice downloads"
        >
          <div
            className="setup-progress-fill"
            style={{
              width: `${Math.min(100, (setup.voicesFetched / setup.voicesTotal) * 100)}%`,
            }}
          />
        </div>
      </>
    );
  } else if (setup.phase === "done") {
    body = (
      <>
        <p className="setup-status">Narration ready ✓</p>
        <p className="setup-hint">The Narrate button now reads your books aloud.</p>
        {onDismiss && (
          <div className="setup-actions">
            <button className="btn primary" onClick={onDismiss}>
              Done
            </button>
          </div>
        )}
      </>
    );
  } else {
    body = (
      <>
        <p className="setup-status">Download failed</p>
        <p className="setup-error">{setup.error ?? "Something went wrong."}</p>
        <div className="setup-actions">
          {onDismiss && (
            <button className="btn" onClick={onDismiss}>
              Close
            </button>
          )}
          <button className="btn primary" onClick={setup.start}>
            Try again
          </button>
        </div>
      </>
    );
  }

  return (
    <div className="narration-setup">
      <h3>On-device narration</h3>
      {body}
    </div>
  );
}
