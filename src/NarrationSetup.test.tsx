// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import NarrationSetup, { formatMB } from "./NarrationSetup";
import type { NarrationController, NarrationSetupPhase } from "./narration";

afterEach(cleanup);

function controller(
  phase: NarrationSetupPhase,
  overrides?: Partial<NarrationController["setup"]>,
  packInstalled: boolean | null = false,
): NarrationController {
  return {
    supported: true,
    options: [
      { uri: "kokoro:af_heart", label: "Heart (American, female)", engine: "kokoro" },
    ],
    speaking: false,
    paused: false,
    page: null,
    spokenRange: null,
    voiceURI: "kokoro:af_heart",
    setVoiceURI: () => {},
    rate: 1,
    setRate: () => {},
    codeHandling: "skip",
    setCodeHandling: () => {},
    continuous: true,
    setContinuous: () => {},
    play: () => {},
    pause: () => {},
    resume: () => {},
    stop: () => {},
    packInstalled,
    needsSetup: packInstalled === false,
    error: null,
    clearError: () => {},
    setup: {
      phase,
      downloadedBytes: 0,
      totalBytes: 0,
      voicesFetched: 0,
      voicesTotal: 9,
      voice: null,
      error: null,
      start: () => {},
      ...overrides,
    },
  } as NarrationController;
}

describe("formatMB", () => {
  it("formats bytes as one-decimal megabytes", () => {
    expect(formatMB(87_300_000)).toBe("87.3 MB");
    expect(formatMB(0)).toBe("0.0 MB");
  });
});

describe("NarrationSetup", () => {
  it("renders nothing once the pack is installed and idle", () => {
    const { container } = render(<NarrationSetup narration={controller("idle", {}, true)} />);
    expect(container.innerHTML).toBe("");
  });

  it("offers the download with sizes and dismiss when the pack is missing", () => {
    const start = vi.fn();
    const onDismiss = vi.fn();
    render(
      <NarrationSetup narration={controller("idle", { start })} onDismiss={onDismiss} />,
    );
    expect(screen.getByText(/92 MB in total/)).toBeTruthy();
    expect(screen.getByText(/87 MB/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Download and set up/ }));
    expect(start).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: /Not now/ }));
    expect(onDismiss).toHaveBeenCalled();
  });

  it("shows byte progress and percent while the engine downloads", () => {
    render(
      <NarrationSetup
        narration={controller("engine", { downloadedBytes: 34_100_000, totalBytes: 87_300_000 })}
      />,
    );
    expect(screen.getByText(/34\.1 MB of 87\.3 MB/)).toBeTruthy();
    expect(screen.getByText(/\(39%\)/)).toBeTruthy();
  });

  it("counts voices as they fetch", () => {
    render(
      <NarrationSetup
        narration={controller("voices", { voicesFetched: 3, voice: "af_heart" })}
      />,
    );
    expect(screen.getByText(/3 \/ 9/)).toBeTruthy();
    expect(screen.getByText(/Heart \(American, female\)/)).toBeTruthy();
  });

  it("announces readiness when done", () => {
    render(<NarrationSetup narration={controller("done")} />);
    expect(screen.getByText(/Narration ready/)).toBeTruthy();
  });

  it("surfaces the failure reason with a retry", () => {
    const start = vi.fn();
    render(
      <NarrationSetup
        narration={controller("failed", { error: "network unreachable", start })}
      />,
    );
    expect(screen.getByText(/network unreachable/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Try again/ }));
    expect(start).toHaveBeenCalled();
  });
});
