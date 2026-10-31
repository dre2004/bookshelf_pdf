import { useCallback, useEffect, useRef, useState } from "react";

/** The page text fed to the engine is exactly `PageTextData.chars`, so
 *  boundary events' charIndex maps straight onto the page's character
 *  geometry — that's what powers the spoken-word highlight. */
export interface SpokenRange {
  page: number;
  start: number;
  end: number;
}

export function speechSupported(): boolean {
  return typeof window !== "undefined" && "speechSynthesis" in window;
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
  voices: SpeechSynthesisVoice[];
  speaking: boolean;
  paused: boolean;
  page: number | null;
  spokenRange: SpokenRange | null;
  voiceURI: string;
  setVoiceURI: (uri: string) => void;
  rate: number;
  setRate: (rate: number) => void;
  continuous: boolean;
  setContinuous: (value: boolean) => void;
  play: (fromPage?: number) => void;
  pause: () => void;
  resume: () => void;
  stop: () => void;
}

export function useNarration({
  docId,
  page,
  pageCount,
  getText,
  onAdvancePage,
}: UseNarrationArgs): NarrationController {
  const supported = speechSupported();

  const [voices, setVoices] = useState<SpeechSynthesisVoice[]>([]);
  const [speaking, setSpeaking] = useState(false);
  const [paused, setPaused] = useState(false);
  const [spokenRange, setSpokenRange] = useState<SpokenRange | null>(null);
  const [narratingPage, setNarratingPage] = useState<number | null>(null);

  const [voiceURI, setVoiceURIState] = useState("");
  const [rate, setRateState] = useState(1);
  const [continuous, setContinuousState] = useState(true);

  // Live refs so in-flight utterances pick up setting changes.
  const voiceURIRef = useRef(voiceURI);
  const rateRef = useRef(rate);
  const continuousRef = useRef(continuous);
  const stoppedRef = useRef(false);
  const lastBoundaryRef = useRef(0);
  const speakingPageRef = useRef<number | null>(null);

  useEffect(() => {
    voiceURIRef.current = voiceURI;
  }, [voiceURI]);
  useEffect(() => {
    rateRef.current = rate;
  }, [rate]);
  useEffect(() => {
    continuousRef.current = continuous;
  }, [continuous]);

  // Voice list loads asynchronously in most engines.
  useEffect(() => {
    if (!supported) return;
    const synth = window.speechSynthesis;
    const load = () => {
      const list = sortVoices(synth.getVoices());
      setVoices(list);
      setVoiceURIState((current) => {
        if (current && list.some((v) => v.voiceURI === current)) return current;
        const preferred =
          list.find((v) => v.localService && v.lang.startsWith(navigator.language)) ??
          list.find((v) => v.lang.startsWith(navigator.language)) ??
          list.find((v) => v.localService) ??
          list[0];
        return preferred?.voiceURI ?? "";
      });
    };
    load();
    synth.addEventListener("voiceschanged", load);
    return () => synth.removeEventListener("voiceschanged", load);
  }, [supported]);

  const stop = useCallback(() => {
    if (!supported) return;
    stoppedRef.current = true;
    window.speechSynthesis.cancel();
    speakingPageRef.current = null;
    lastBoundaryRef.current = 0;
    setSpeaking(false);
    setPaused(false);
    setNarratingPage(null);
    setSpokenRange(null);
  }, [supported]);

  const speakPage = useCallback(
    (target: number) => {
      if (!supported) return;
      stoppedRef.current = false;
      window.speechSynthesis.cancel();
      speakingPageRef.current = target;
      setNarratingPage(target);
      setSpeaking(true);
      setPaused(false);
      lastBoundaryRef.current = 0;
      setSpokenRange(null);

      void getText(target)
        .then((text) => {
          if (stoppedRef.current || speakingPageRef.current !== target) return;
          if (!text.trim()) {
            // Blank page: skip straight to the next one.
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
    [supported, getText, pageCount, onAdvancePage, stop],
  );

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
    if (!supported || !speaking) return;
    window.speechSynthesis.pause();
    setPaused(true);
  }, [supported, speaking]);

  const resume = useCallback(() => {
    if (!supported || !speaking) return;
    window.speechSynthesis.resume();
    setPaused(false);
  }, [supported, speaking]);

  return {
    supported,
    voices,
    speaking,
    paused,
    page: narratingPage,
    spokenRange,
    voiceURI,
    setVoiceURI: setVoiceURIState,
    rate,
    setRate: setRateState,
    continuous,
    setContinuous: setContinuousState,
    play,
    pause,
    resume,
    stop,
  };
}
