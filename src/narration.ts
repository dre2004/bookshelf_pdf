import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  type CodeHandling,
  type TtsVoice,
  ttsDownloadVoices,
  ttsInstallKokoro,
  ttsPackInstalled,
  ttsPause,
  ttsResume,
  ttsSpeak,
  ttsStop,
  ttsVoices,
} from "./api";

/** The page text fed to the engine is exactly `PageTextData.chars`, so
 *  progress events' char ranges map straight onto the page's character
 *  geometry — that's what powers the spoken-word highlight. */
export interface SpokenRange {
  page: number;
  start: number;
  end: number;
}

/** Sentence spoken by the settings screen's sample button. Matches the
 *  Rust preview path (`tts_preview_voice`). */
export const SAMPLE_SENTENCE = "This is a sample of how this voice reads your books.";

export interface NarrationVoiceOption {
  /** Stable selection key; `kokoro:<id>` for on-device voices, the system
   *  voice's own URI otherwise. */
  uri: string;
  label: string;
  engine: "kokoro" | "speech";
}

/** Local voices first, then by name — they tend to respond faster. */
export function sortVoices(voices: SpeechSynthesisVoice[]): SpeechSynthesisVoice[] {
  return [...voices].sort((a, b) => {
    if (a.localService !== b.localService) return a.localService ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
}

interface UseNarrationArgs {
  docId: number;
  page: number;
  pageCount: number;
  /** Full character text of a page (aligned with its char boxes). */
  getText: (page: number) => Promise<string>;
  /** Called when auto-advance moves to the next page. */
  onAdvancePage: (page: number) => void;
}

/** Phases of the one-time on-device narration download. */
export type NarrationSetupPhase = "idle" | "engine" | "voices" | "done" | "failed";

export interface NarrationController {
  supported: boolean;
  /** All selectable voices: on-device (Kokoro) first, then system voices. */
  options: NarrationVoiceOption[];
  speaking: boolean;
  paused: boolean;
  page: number | null;
  spokenRange: SpokenRange | null;
  voiceURI: string;
  setVoiceURI: (uri: string) => void;
  rate: number;
  setRate: (rate: number) => void;
  codeHandling: CodeHandling;
  setCodeHandling: (value: CodeHandling) => void;
  continuous: boolean;
  setContinuous: (value: boolean) => void;
  play: (fromPage?: number) => void;
  pause: () => void;
  resume: () => void;
  stop: () => void;
  /** `null` while the check is in flight; narration is gated only on a
   *  confirmed `false`. */
  packInstalled: boolean | null;
  /** True when narration cannot run yet: the engine pack is missing and
   *  the selection is (or would default to) an on-device voice. */
  needsSetup: boolean;
  /** Why playback stopped unexpectedly, if it did. Cleared on replay. */
  error: string | null;
  clearError: () => void;
  /** State of the one-time on-device engine + voices download. */
  setup: {
    phase: NarrationSetupPhase;
    downloadedBytes: number;
    totalBytes: number;
    voicesFetched: number;
    voicesTotal: number;
    /** Voice currently downloading, if any. */
    voice: string | null;
    error: string | null;
    start: () => void;
  };
}

const KOKORO_PREFIX = "kokoro:";

function isOnDeviceVoice(uri: string): boolean {
  return uri.startsWith(KOKORO_PREFIX);
}

export function useNarration({
  docId,
  page,
  pageCount,
  getText,
  onAdvancePage,
}: UseNarrationArgs): NarrationController {
  const supported = speechSupported();

  const [systemVoices, setSystemVoices] = useState<SpeechSynthesisVoice[]>([]);
  const [engineVoices, setEngineVoices] = useState<TtsVoice[]>([]);
  const [speaking, setSpeaking] = useState(false);
  const [paused, setPaused] = useState(false);
  const [spokenRange, setSpokenRange] = useState<SpokenRange | null>(null);
  const [narratingPage, setNarratingPage] = useState<number | null>(null);

  const [voiceURI, setVoiceURIState] = useState("");
  const [rate, setRateState] = useState(1);
  const [codeHandling, setCodeHandling] = useState<CodeHandling>("skip");
  const [continuous, setContinuousState] = useState(true);

  // True while the selection is an automatic default rather than a user
  // choice — lets the on-device list (which loads slower than system
  // voices) win the default once it arrives.
  const autoPicked = useRef(false);

  // Live refs so in-flight playback picks up setting changes.
  const voiceURIRef = useRef(voiceURI);
  const rateRef = useRef(rate);
  const codeHandlingRef = useRef(codeHandling);
  const continuousRef = useRef(continuous);
  const stoppedRef = useRef(false);
  const lastBoundaryRef = useRef(0);
  // Current chunk while narrating, so speed changes can resume in place.
  const currentChunkRef = useRef<{ page: number; start: number; end: number } | null>(null);
  const speakingPageRef = useRef<number | null>(null);

  useEffect(() => {
    voiceURIRef.current = voiceURI;
  }, [voiceURI]);
  useEffect(() => {
    rateRef.current = rate;
  }, [rate]);
  useEffect(() => {
    codeHandlingRef.current = codeHandling;
  }, [codeHandling]);
  useEffect(() => {
    continuousRef.current = continuous;
  }, [continuous]);

  const options: NarrationVoiceOption[] = [
    ...engineVoices.map((voice) => ({
      uri: `${KOKORO_PREFIX}${voice.id}`,
      label: voice.label,
      engine: "kokoro" as const,
    })),
    ...(supported
      ? systemVoices.map((voice) => ({
          uri: voice.voiceURI,
          label: `${voice.name} (${voice.lang})`,
          engine: "speech" as const,
        }))
      : []),
  ];

  /** Default pick: Heart when listed, else the usual system heuristic. */
  const defaultVoiceURI = useCallback(
    (system: SpeechSynthesisVoice[], engine: TtsVoice[]): string => {
      const heart = engine.find((voice) => voice.id === "af_heart");
      if (heart) return `${KOKORO_PREFIX}${heart.id}`;
      if (engine[0]) return `${KOKORO_PREFIX}${engine[0].id}`;
      const list = sortVoices(system);
      const preferred =
        list.find((v) => v.localService && v.lang.startsWith(navigator.language)) ??
        list.find((v) => v.lang.startsWith(navigator.language)) ??
        list.find((v) => v.localService) ??
        list[0];
      return preferred?.voiceURI ?? "";
    },
    [],
  );

  // System voice list loads asynchronously in most engines.
  useEffect(() => {
    if (!supported) return;
    const synth = window.speechSynthesis;
    const load = () => {
      const list = sortVoices(synth.getVoices());
      setSystemVoices(list);
      setVoiceURIState((current) => {
        // A user choice always wins; an on-device default holds until the
        // engine list settles it.
        if (current && (!autoPicked.current || isOnDeviceVoice(current))) return current;
        const next = defaultVoiceURI(list, []);
        if (next) autoPicked.current = true;
        return next || current;
      });
    };
    load();
    synth.addEventListener("voiceschanged", load);
    return () => synth.removeEventListener("voiceschanged", load);
  }, [supported, defaultVoiceURI]);

  // On-device voices come from the Rust engine; arriving last, they get the
  // final say over any automatic default (Heart wins when listed).
  const refreshEngineVoices = useCallback(() => {
    ttsVoices()
      .then((voices) => {
        setEngineVoices(voices);
        setVoiceURIState((current) => {
          if (current && !autoPicked.current) return current;
          const next = defaultVoiceURI(systemVoices, voices);
          if (next) autoPicked.current = true;
          return next || current;
        });
      })
      .catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [defaultVoiceURI]);

  useEffect(() => {
    refreshEngineVoices();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [docId, refreshEngineVoices]);

  // ---- On-device narration setup (engine pack + voices) ----
  const [phase, setPhase] = useState<NarrationSetupPhase>("idle");
  const [downloadedBytes, setDownloadedBytes] = useState(0);
  const [totalBytes, setTotalBytes] = useState(0);
  const [voicesFetched, setVoicesFetched] = useState(0);
  const [setupVoice, setSetupVoice] = useState<string | null>(null);
  const [setupError, setSetupError] = useState<string | null>(null);
  const [packInstalled, setPackInstalled] = useState<boolean | null>(null);
  // Mirrors `phase` for the event listener, which must branch without a
  // re-render (and must not run side effects inside a state updater).
  const phaseRef = useRef<NarrationSetupPhase>("idle");
  const setPhaseBoth = useCallback((next: NarrationSetupPhase) => {
    phaseRef.current = next;
    setPhase(next);
  }, []);
  const VOICES_TOTAL = 9; // the curated on-device catalog

  const checkPackInstalled = useCallback(() => {
    ttsPackInstalled()
      .then((installed) => setPackInstalled(installed))
      .catch(() => {});
  }, []);

  useEffect(() => {
    checkPackInstalled();
  }, [checkPackInstalled]);

  useEffect(() => {
    const unlisten = listen<{
      kind:
        | "started"
        | "progress"
        | "done"
        | "failed"
        | "voiceStarted"
        | "voiceDone";
      totalBytes?: number;
      downloadedBytes?: number;
      voice?: string;
      reason?: string;
    }>("kokoro-download", (event) => {
      const payload = event.payload;
      switch (payload.kind) {
        case "started":
          setPhaseBoth("engine");
          setDownloadedBytes(0);
          setTotalBytes(payload.totalBytes ?? 0);
          setSetupError(null);
          break;
        case "progress":
          if (payload.totalBytes) setTotalBytes(payload.totalBytes);
          if (payload.downloadedBytes) setDownloadedBytes(payload.downloadedBytes);
          break;
        case "done":
          // Engine pack settled (downloaded or already present): fetch
          // the voices; the next done event completes the setup.
          if (phaseRef.current === "engine") {
            setPhaseBoth("voices");
            ttsDownloadVoices().catch((e: unknown) => {
              setSetupError(String(e));
              setPhaseBoth("failed");
            });
          } else if (phaseRef.current === "voices") {
            setSetupVoice(null);
            setPackInstalled(true);
            setPhaseBoth("done");
            refreshEngineVoices();
          }
          break;
        case "voiceStarted":
          setSetupVoice(payload.voice ?? null);
          break;
        case "voiceDone":
          setSetupVoice(null);
          setVoicesFetched((count) => count + 1);
          break;
        case "failed":
          setSetupError(payload.reason ?? "download failed");
          setPhaseBoth("failed");
          checkPackInstalled();
          break;
      }
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [refreshEngineVoices, checkPackInstalled, setPhaseBoth]);

  const startSetup = useCallback(() => {
    setSetupError(null);
    setVoicesFetched(0);
    setSetupVoice(null);
    setDownloadedBytes(0);
    setTotalBytes(0);
    setPhaseBoth("engine");
    ttsInstallKokoro().catch((e) => {
      setSetupError(String(e));
      setPhaseBoth("failed");
    });
  }, [setPhaseBoth]);

  const [speakError, setSpeakError] = useState<string | null>(null);
  const clearError = useCallback(() => setSpeakError(null), []);

  const setVoiceURI = useCallback((uri: string) => {
    autoPicked.current = false;
    setVoiceURIState(uri);
  }, []);

  const stop = useCallback(() => {
    stoppedRef.current = true;
    if (isOnDeviceVoice(voiceURIRef.current) || speakingPageRef.current !== null) {
      void ttsStop().catch(() => {});
    }
    if (supported) window.speechSynthesis.cancel();
    speakingPageRef.current = null;
    lastBoundaryRef.current = 0;
    setSpeaking(false);
    setPaused(false);
    setNarratingPage(null);
    setSpokenRange(null);
  }, [supported]);

  /** Stops playback and leaves a persistent reason instead of silently
   *  ripping the narration bar away. */
  const fail = useCallback(
    (reason: unknown) => {
      stop();
      setSpeakError(
        typeof reason === "string" && reason
          ? reason
          : String(reason ?? "narration stopped"),
      );
    },
    [stop],
  );

  const speakPage = useCallback(
    (target: number) => {
      stoppedRef.current = false;
      lastBoundaryRef.current = 0;
      setNarratingPage(target);
      setSpeaking(true);
      setPaused(false);
      setSpokenRange(null);

      const selection = voiceURIRef.current;
      if (isOnDeviceVoice(selection)) {
        speakingPageRef.current = target;
        void getText(target)
          .then((text) =>
            ttsSpeak({
              text,
              page: target,
              voiceId: selection,
              rate: rateRef.current,
              docId,
              codeHandling: codeHandlingRef.current,
            }),
          )
          .catch((e) => fail(e));
        return;
      }

      if (!supported) return;
      speakingPageRef.current = target;
      window.speechSynthesis.cancel();
      void getText(target)
        .then((text) => {
          if (stoppedRef.current || speakingPageRef.current !== target) return;
          if (!text.trim()) {
            if (continuousRef.current && target + 1 < pageCount) {
              onAdvancePage(target + 1);
              speakPage(target + 1);
            } else {
              stop();
            }
            return;
          }
          const utterance = new SpeechSynthesisUtterance(text);
          const voice = window.speechSynthesis
            .getVoices()
            .find((v) => v.voiceURI === voiceURIRef.current);
          if (voice) {
            utterance.voice = voice;
            utterance.lang = voice.lang;
          }
          utterance.rate = rateRef.current;
          utterance.onboundary = (event) => {
            if (typeof event.charIndex === "number") {
              const start = lastBoundaryRef.current;
              const end = event.charIndex;
              if (end > start) setSpokenRange({ page: target, start, end });
              lastBoundaryRef.current = end;
            }
          };
          utterance.onend = () => {
            if (stoppedRef.current || speakingPageRef.current !== target) return;
            if (continuousRef.current && target + 1 < pageCount) {
              onAdvancePage(target + 1);
              speakPage(target + 1);
            } else {
              stop();
            }
          };
          utterance.onerror = () => {
            if (speakingPageRef.current === target) {
              fail("the system voice failed to speak — pick a Bookshelf voice in Settings");
            }
          };
          window.speechSynthesis.speak(utterance);
        })
        .catch((e) => fail(e));
    },
    [getText, pageCount, onAdvancePage, stop, fail, supported],
  );

  // On-device engine events: progress → highlight, pageDone → advance.
  useEffect(() => {
    const unlisten = listen<{
      kind: "progress" | "pageDone" | "error";
      page?: number;
      start?: number;
      end?: number;
      reason?: string;
    }>("tts-event", (event) => {
      const payload = event.payload;
      if (payload.kind === "progress" && payload.page != null) {
        currentChunkRef.current = {
          page: payload.page,
          start: payload.start ?? 0,
          end: payload.end ?? 0,
        };
        setSpokenRange({ page: payload.page, start: payload.start ?? 0, end: payload.end ?? 0 });
      } else if (payload.kind === "error") {
        fail(payload.reason ?? "the narration engine stopped unexpectedly");
      } else if (payload.kind === "pageDone") {
        const done = payload.page ?? narratingPageRef.current;
        if (done == null) return;
        if (continuousRef.current && done + 1 < pageCount) {
          onAdvancePage(done + 1);
          speakPage(done + 1);
        } else {
          stop();
        }
      }
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [speakPage, stop, fail, onAdvancePage, pageCount]);

  // Keep a ref of the narrating page for late pageDone events.
  const narratingPageRef = useRef<number | null>(null);
  useEffect(() => {
    narratingPageRef.current = narratingPage;
  }, [narratingPage]);

  // Changing documents stops narration.
  useEffect(() => {
    stop();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [docId]);

  const play = useCallback(
    (fromPage?: number) => {
      clearError();
      speakPage(fromPage ?? page);
    },
    [clearError, speakPage, page],
  );

  const pause = useCallback(() => {
    if (!speaking) return;
    if (isOnDeviceVoice(voiceURIRef.current)) {
      void ttsPause().catch(() => {});
    } else if (supported) {
      window.speechSynthesis.pause();
    }
    setPaused(true);
  }, [speaking, supported]);

  const resume = useCallback(() => {
    if (!speaking) return;
    if (isOnDeviceVoice(voiceURIRef.current)) {
      void ttsResume().catch(() => {});
    } else if (supported) {
      window.speechSynthesis.resume();
    }
    setPaused(false);
  }, [speaking, supported]);

  const setRate = useCallback(
    (nextRate: number) => {
      setRateState(nextRate);
      rateRef.current = nextRate;
      // Apply immediately when narrating: re-speak the current selection
      // at the new speed. The chunk restart keeps position and highlight.
      const chunk = currentChunkRef.current;
      if (chunk && speakingPageRef.current !== null && !stoppedRef.current) {
        const target = chunk.page;
        void getText(target)
          .then((text) =>
            ttsSpeak({
              text,
              page: target,
              voiceId: voiceURIRef.current,
              rate: nextRate,
              docId,
              codeHandling: codeHandlingRef.current,
            }),
          )
          .catch((e) => fail(e));
      }
    },
    [docId, fail, getText, stop],
  );

  return {
    supported: supported || engineVoices.length > 0,
    options,
    speaking,
    paused,
    page: narratingPage,
    spokenRange,
    voiceURI,
    setVoiceURI,
    rate,
    setRate,
    codeHandling,
    setCodeHandling,
    continuous,
    setContinuous: setContinuousState,
    play,
    pause,
    resume,
    stop,
    packInstalled,
    needsSetup:
      packInstalled === false && (voiceURI === "" || isOnDeviceVoice(voiceURI)),
    error: speakError,
    clearError,
    setup: {
      phase,
      downloadedBytes,
      totalBytes,
      voicesFetched,
      voicesTotal: VOICES_TOTAL,
      voice: setupVoice,
      error: setupError,
      start: startSetup,
    },
  };
}

export function speechSupported(): boolean {
  return typeof window !== "undefined" && "speechSynthesis" in window;
}
