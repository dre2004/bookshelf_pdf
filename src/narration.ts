import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  type CodeHandling,
  type TtsVoice,
  ttsInstallKokoro,
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
  /** State of the on-device engine pack download offered in settings. */
  download: {
    active: boolean;
    progress: number; // 0..1 when the total size is known
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

  // ---- Kokoro engine pack download ----
  const [downloadActive, setDownloadActive] = useState(false);
  const [downloadProgress, setDownloadProgress] = useState(0);
  const [downloadError, setDownloadError] = useState<string | null>(null);

  useEffect(() => {
    const unlisten = listen<{
      kind: "started" | "progress" | "done" | "failed";
      totalBytes?: number;
      downloadedBytes?: number;
      reason?: string;
    }>("kokoro-download", (event) => {
      const payload = event.payload;
      if (payload.kind === "started") {
        setDownloadActive(true);
        setDownloadError(null);
        setDownloadProgress(0);
      } else if (payload.kind === "progress") {
        const total = payload.totalBytes ?? 0;
        const downloaded = payload.downloadedBytes ?? 0;
        if (total > 0) setDownloadProgress(Math.min(1, downloaded / total));
      } else if (payload.kind === "done") {
        setDownloadActive(false);
        setDownloadProgress(1);
        refreshEngineVoices();
      } else if (payload.kind === "failed") {
        setDownloadActive(false);
        setDownloadError(payload.reason ?? "download failed");
      }
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [refreshEngineVoices]);

  const startDownload = useCallback(() => {
    setDownloadError(null);
    setDownloadActive(true);
    setDownloadProgress(0);
    ttsInstallKokoro().catch((e) => {
      setDownloadActive(false);
      setDownloadError(String(e));
    });
  }, []);

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
          .catch(() => stop());
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
            if (speakingPageRef.current === target) stop();
          };
          window.speechSynthesis.speak(utterance);
        })
        .catch(() => stop());
    },
    [getText, pageCount, onAdvancePage, stop, supported],
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
        stop();
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
  }, [speakPage, stop, onAdvancePage, pageCount]);

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
      speakPage(fromPage ?? page);
    },
    [speakPage, page],
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
          .catch(() => stop());
      }
    },
    [docId, getText, stop],
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
    download: {
      active: downloadActive,
      progress: downloadProgress,
      error: downloadError,
      start: startDownload,
    },
  };
}

export function speechSupported(): boolean {
  return typeof window !== "undefined" && "speechSynthesis" in window;
}
