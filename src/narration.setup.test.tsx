// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import { useNarration, type NarrationController } from "./narration";

// Captured event handlers per channel so tests can drive the download
// and playback state machines exactly as the Rust backend's events would.
type Handler = (event: { payload: Record<string, unknown> }) => void;
const handlers = new Map<string, Handler>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((channel: string, handler: Handler) => {
    handlers.set(channel, handler);
    return Promise.resolve(() => {});
  }),
}));

const installKokoro = vi.fn().mockResolvedValue(undefined);
const downloadVoices = vi.fn().mockResolvedValue(undefined);
vi.mock("./api", () => ({
  ttsVoices: vi.fn().mockResolvedValue([]),
  ttsPackInstalled: vi.fn().mockResolvedValue(false),
  ttsInstallKokoro: vi.fn(() => installKokoro()),
  ttsDownloadVoices: vi.fn(() => downloadVoices()),
  ttsSpeak: vi.fn().mockResolvedValue(undefined),
  ttsPause: vi.fn().mockResolvedValue(undefined),
  ttsResume: vi.fn().mockResolvedValue(undefined),
  ttsStop: vi.fn().mockResolvedValue(undefined),
}));

afterEach(() => {
  cleanup();
  handlers.clear();
  vi.clearAllMocks();
});

function Probe({ onReady }: { onReady: (n: NarrationController) => void }) {
  const narration = useNarration({
    docId: -1,
    page: 0,
    pageCount: 0,
    getText: () => Promise.resolve(""),
    onAdvancePage: () => {},
  });
  onReady(narration);
  return null;
}

function mount(): { current: NarrationController | null } {
  const controller: { current: NarrationController | null } = { current: null };
  render(<Probe onReady={(n) => (controller.current = n)} />);
  return controller;
}

const emit = (channel: string, payload: Record<string, unknown>) =>
  act(async () => {
    handlers.get(channel)?.({ payload });
  });

describe("narration setup state machine", () => {
  it("gates narration while the pack is confirmed missing", async () => {
    const controller = mount();
    await act(async () => {});
    expect(controller.current?.packInstalled).toBe(false);
    expect(controller.current?.needsSetup).toBe(true);
  });

  it("walks engine → voices → done and un-gates narration", async () => {
    const controller = mount();
    await act(async () => {});
    act(() => controller.current?.setup.start());
    expect(controller.current?.setup.phase).toBe("engine");
    expect(installKokoro).toHaveBeenCalled();

    await emit("kokoro-download", { kind: "started", totalBytes: 87_300_000 });
    expect(controller.current?.setup.totalBytes).toBe(87_300_000);

    await emit("kokoro-download", { kind: "progress", downloadedBytes: 34_100_000 });
    expect(controller.current?.setup.downloadedBytes).toBe(34_100_000);

    await emit("kokoro-download", { kind: "done" });
    expect(controller.current?.setup.phase).toBe("voices");
    expect(downloadVoices).toHaveBeenCalled();

    await emit("kokoro-download", { kind: "voiceStarted", voice: "af_heart" });
    expect(controller.current?.setup.voice).toBe("af_heart");
    await emit("kokoro-download", { kind: "voiceDone", voice: "af_heart" });
    expect(controller.current?.setup.voicesFetched).toBe(1);

    await emit("kokoro-download", { kind: "done" });
    expect(controller.current?.setup.phase).toBe("done");
    expect(controller.current?.packInstalled).toBe(true);
    expect(controller.current?.needsSetup).toBe(false);
  });

  it("lands in failed with the reason on a download error", async () => {
    const controller = mount();
    await act(async () => {});
    act(() => controller.current?.setup.start());
    await emit("kokoro-download", { kind: "failed", reason: "connection reset" });
    expect(controller.current?.setup.phase).toBe("failed");
    expect(controller.current?.setup.error).toBe("connection reset");
  });

  it("keeps the engine's error reason instead of failing silently", async () => {
    const controller = mount();
    await act(async () => {});
    await emit("tts-event", {
      kind: "error",
      reason: "the Kokoro voice pack is not installed",
    });
    expect(controller.current?.speaking).toBe(false);
    expect(controller.current?.error).toBe("the Kokoro voice pack is not installed");
    act(() => controller.current?.clearError());
    expect(controller.current?.error).toBeNull();
  });
});
