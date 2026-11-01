import { useCallback, useEffect, useRef, useState } from "react";
import { ask, open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import Toolbar from "./Toolbar";
import Reader from "./Reader";
import Library from "./Library";
import Sidebar, { type SidebarTab } from "./Sidebar";
import Resizer from "./Resizer";
import SearchPanel from "./SearchPanel";
import SettingsScreen from "./SettingsScreen";
import NarrationBar from "./NarrationBar";
import { useNarration } from "./narration";
import {
  bookProgressSave,
  bookRegister,
  bookState,
  bookmarksAdd,
  bookmarksList,
  bookmarksRemove,
  cachedPageText,
  closeDocument,
  getPageDims,
  getOutline,
  getStartupFile,
  highlightsAdd,
  highlightsList,
  highlightsRemove,
  highlightsUpdate,
  libraryList,
  libraryRemove,
  openDocument,
  searchDocument,
  settingsAll,
  settingsSet,
  toApiError,
} from "./api";
import type {
  BookSummary,
  Bookmark,
  DocumentInfo,
  Highlight,
  HighlightColor,
  OutlineNode,
  PageInfo,
  ReadMode,
  SearchMatch,
  Theme,
  ZoomMode,
} from "./types";
import { settingsFromStorage } from "./settings";
import "./App.css";

type View = "library" | "reader";

interface CurrentBook {
  bookId: number;
  path: string;
  title: string;
  doc: DocumentInfo;
}

const DIM_BATCH = 100;
const SAVE_DEBOUNCE_MS = 700;
const SEARCH_DEBOUNCE_MS = 250;
/** Mirrors the backend's per-query cap (search_document). */
const SEARCH_RESULT_CAP = 2000;

function describeError(e: unknown): string {
  const api = toApiError(e);
  if (api) {
    const label = api.code.replace(/_/g, " ");
    return api.reason ? `${label}: ${api.reason}` : label;
  }
  return String(e);
}

function hitKey(hit: SearchMatch): string {
  return `${hit.page}:${hit.start}:${hit.end}`;
}

export default function App() {
  const [view, setView] = useState<View>("library");
  const [library, setLibrary] = useState<BookSummary[]>([]);
  const [ready, setReady] = useState(false);

  const [current, setCurrent] = useState<CurrentBook | null>(null);
  const [dims, setDims] = useState<PageInfo[]>([]);
  const [page, setPage] = useState(0);
  const [resume, setResume] = useState<{ page: number; offset: number } | null>(null);

  const [mode, setMode] = useState<ReadMode>("scroll");
  const [zoom, setZoom] = useState<ZoomMode>({ kind: "fitWidth" });
  const [theme, setTheme] = useState<Theme>("day");
  const [spread, setSpread] = useState(false);
  const settingsReady = useRef(false);

  const [sidebarTab, setSidebarTab] = useState<SidebarTab | null>(null);
  const [outline, setOutline] = useState<OutlineNode[]>([]);
  const [bookmarks, setBookmarks] = useState<Bookmark[]>([]);
  const [highlights, setHighlights] = useState<Highlight[]>([]);

  // Back/forward navigation stacks (pages+offsets you jumped away from).
  const [history, setHistory] = useState<{ page: number; offset: number }[]>([]);
  const [future, setFuture] = useState<{ page: number; offset: number }[]>([]);

  const [search, setSearch] = useState<{
    open: boolean;
    query: string;
    results: SearchMatch[];
    active: number;
    searching: boolean;
    caseSensitive: boolean;
    wholeWords: boolean;
  }>({
    open: false,
    query: "",
    results: [],
    active: 0,
    searching: false,
    caseSensitive: false,
    wholeWords: false,
  });

  const [settingsOpen, setSettingsOpen] = useState(false);
  const [sidebarWidth, setSidebarWidth] = useState(240);
  const [narrationVoice, setNarrationVoice] = useState("");
  const [narrationRate, setNarrationRate] = useState(1);
  const [narrationContinuous, setNarrationContinuous] = useState(true);
  const [narrationCodeHandling, setNarrationCodeHandling] = useState<"skip" | "announce">("skip");

  const [error, setError] = useState<string | null>(null);
  const [passwordFor, setPasswordFor] = useState<{ path: string; incorrect: boolean } | null>(null);
  const [password, setPassword] = useState("");

  const currentRef = useRef<CurrentBook | null>(null);
  const saveTimer = useRef<number | null>(null);
  const lastProgress = useRef<{ bookId: number; page: number; offset: number } | null>(null);
  const searchTimer = useRef<number | null>(null);
  const searchSeq = useRef(0);
  // Boot/listeners run before openPath exists as a stable callback.
  const openPathRef = useRef<((path: string) => Promise<void>) | null>(null);

  // ---- boot: hydrate persisted settings and the library ----
  useEffect(() => {
    const boot = async () => {
      try {
        const stored = await settingsAll();
        const hydrated = settingsFromStorage(stored, {
          mode: "scroll",
          theme: "day",
          zoom: { kind: "fitWidth" },
          spread: false,
        });
        setMode(hydrated.mode);
        setTheme(hydrated.theme);
        setZoom(hydrated.zoom);
        setSpread(hydrated.spread);
        try {
          const voice = JSON.parse(stored.narrationVoice ?? '""');
          // The Piper engine is gone; a stale piper: selection falls back
          // to the automatic default pick (and re-persists).
          if (typeof voice === "string" && !voice.startsWith("piper:")) {
            setNarrationVoice(voice);
          }
        } catch {
          /* keep default voice */
        }
        try {
          const rate = JSON.parse(stored.narrationRate ?? "1");
          if (typeof rate === "number" && rate >= 0.5 && rate <= 2) setNarrationRate(rate);
        } catch {
          /* keep default rate */
        }
        try {
          const width = JSON.parse(stored.sidebarWidth ?? "240");
          if (typeof width === "number" && width >= 180 && width <= 480) setSidebarWidth(width);
        } catch {
          /* default width */
        }
        try {
          const continuous = JSON.parse(stored.narrationContinuous ?? "true");
          if (typeof continuous === "boolean") setNarrationContinuous(continuous);
        } catch {
          /* keep default */
        }
        try {
          const handling = JSON.parse(stored.narrationCodeHandling ?? '"skip"');
          if (handling === "skip" || handling === "announce") setNarrationCodeHandling(handling);
        } catch {
          /* keep default */
        }
        settingsReady.current = true;
      } catch {
        settingsReady.current = true;
      }
      try {
        setLibrary(await libraryList());
      } catch {
        /* library stays empty; reading still works */
      }
      setReady(true);
      // A PDF double-clicked while the app was closed arrives via argv.
      try {
        const startup = await getStartupFile();
        if (startup) void openPathRef.current?.(startup);
      } catch {
        /* no startup file */
      }
    };
    void boot();
  }, []);

  // Files opened while the app is already running (single-instance forward).
  useEffect(() => {
    const unlisten = listen<string>("open-file", (event) => {
      if (event.payload) void openPathRef.current?.(event.payload);
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  const persist = useCallback((key: string, value: unknown) => {
    if (!settingsReady.current) return;
    void settingsSet(key, JSON.stringify(value)).catch(() => {});
  }, []);

  const resizeSidebar = useCallback(
    (width: number) => {
      const clamped = Math.min(480, Math.max(180, Math.round(width)));
      setSidebarWidth(clamped);
      persist("sidebarWidth", clamped);
    },
    [persist],
  );

  const changeMode = useCallback(
    (m: ReadMode) => {
      setMode(m);
      persist("readMode", m);
    },
    [persist],
  );
  const changeTheme = useCallback(
    (t: Theme) => {
      setTheme(t);
      persist("theme", t);
    },
    [persist],
  );
  const changeZoom = useCallback(
    (z: ZoomMode) => {
      setZoom(z);
      persist("zoom", z);
    },
    [persist],
  );

  const refreshLibrary = useCallback(async () => {
    try {
      setLibrary(await libraryList());
    } catch {
      /* keep previous list */
    }
  }, []);

  // ---- progress saving ----
  const flushProgress = useCallback(() => {
    const progress = lastProgress.current;
    if (progress) {
      lastProgress.current = null;
      void bookProgressSave(progress.bookId, progress.page, progress.offset).catch(() => {});
    }
    if (saveTimer.current !== null) {
      window.clearTimeout(saveTimer.current);
      saveTimer.current = null;
    }
  }, []);

  // Always-current reading position (survives flushes) — the source for
  // back/forward history entries.
  const positionRef = useRef<{ page: number; offset: number }>({ page: 0, offset: 0 });

  const scheduleSave = useCallback(
    (bookId: number, nextPage: number, offset: number) => {
      positionRef.current = { page: nextPage, offset };
      if (bookId < 0) return;
      lastProgress.current = { bookId, page: nextPage, offset };
      if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(flushProgress, SAVE_DEBOUNCE_MS);
    },
    [flushProgress],
  );

  useEffect(() => {
    const flush = () => flushProgress();
    window.addEventListener("visibilitychange", flush);
    window.addEventListener("beforeunload", flush);
    return () => {
      window.removeEventListener("visibilitychange", flush);
      window.removeEventListener("beforeunload", flush);
    };
  }, [flushProgress]);

  // ---- opening books ----
  const openPath = useCallback(
    async (path: string, pwd?: string) => {
      if (currentRef.current?.path === path) {
        // Re-entering the open book: the Reader remounted with scrollTop 0
        // when the library opened, so the latest saved position must be
        // restored again — otherwise the view desyncs from the page state
        // and same-page chapter clicks die silently.
        const book = currentRef.current;
        if (book.bookId >= 0) {
          try {
            const state = await bookState(book.bookId);
            if (state && (state.lastPage > 0 || state.lastOffset > 0)) {
              const page = Math.min(state.lastPage, book.doc.pageCount - 1);
              setPage(page);
              positionRef.current = { page, offset: state.lastOffset };
              setResume({ page, offset: state.lastOffset });
            } else {
              setPage(0);
              positionRef.current = { page: 0, offset: 0 };
              setResume(null);
            }
          } catch {
            /* stay on the current page state */
          }
        }
        setView("reader");
        return;
      }
      setError(null);
      try {
        const doc = await openDocument(path, pwd);
        const title = path.split(/[\\/]/).pop() ?? path;
        let bookId = -1;
        try {
          bookId = await bookRegister(path, title, doc.pageCount, doc.docId);
        } catch {
          /* library persistence unavailable — read without saving progress */
        }

        let restore: { page: number; offset: number } | null = null;
        if (bookId >= 0) {
          try {
            const state = await bookState(bookId);
            if (state && (state.lastPage > 0 || state.lastOffset > 0)) {
              restore = {
                page: Math.min(state.lastPage, doc.pageCount - 1),
                offset: state.lastOffset,
              };
            }
          } catch {
            /* start from the beginning */
          }
        }

        const previous = currentRef.current;
        currentRef.current = { bookId, path, title, doc };
        setCurrent(currentRef.current);
        setDims(doc.pages);
        setPage(restore?.page ?? 0);
        setResume(restore);
        positionRef.current = { page: restore?.page ?? 0, offset: restore?.offset ?? 0 };
        setHistory([]);
        setFuture([]);
        setPasswordFor(null);
        setPassword("");
        setView("reader");
        setOutline([]);
        setBookmarks([]);
        setHighlights([]);
        setSearch((s) => ({ ...s, open: false, query: "", results: [], active: 0 }));

        if (previous) void closeDocument(previous.doc.docId).catch(() => {});
        void refreshLibrary();

        if (bookId >= 0) {
          getOutline(doc.docId)
            .then(setOutline)
            .catch(() => {});
          bookmarksList(bookId)
            .then(setBookmarks)
            .catch(() => {});
          highlightsList(bookId)
            .then(setHighlights)
            .catch(() => {});
        }
      } catch (e) {
        const api = toApiError(e);
        if (api && (api.code === "password_required" || api.code === "password_incorrect")) {
          setPasswordFor({ path, incorrect: api.code === "password_incorrect" });
          setPassword("");
        } else {
          setError(describeError(e));
        }
      }
    },
    [refreshLibrary],
  );

  const pickFile = useCallback(async () => {
    const picked = await openFileDialog({
      multiple: false,
      directory: false,
      filters: [{ name: "PDF documents", extensions: ["pdf"] }],
    });
    if (typeof picked === "string") void openPath(picked);
  }, [openPath]);

  useEffect(() => {
    openPathRef.current = openPath;
  }, [openPath]);

  // Window title carries the open book's name; the toolbar stays lean.
  useEffect(() => {
    const title =
      current && view === "reader" ? `Bookshelf PDF - ${current.title}` : "Bookshelf PDF";
    void getCurrentWindow().setTitle(title).catch(() => {});
  }, [current, view]);

  const goLibrary = useCallback(() => {
    flushProgress();
    setView("library");
    setSearch((s) => ({ ...s, open: false }));
    void refreshLibrary();
  }, [flushProgress, refreshLibrary]);

  const removeBook = useCallback(
    async (book: BookSummary) => {
      const confirmed = await ask(
        `Remove "${book.title}" from the library? The PDF file itself is not deleted.`,
        { title: "Remove from library", kind: "warning" },
      );
      if (!confirmed) return;
      try {
        await libraryRemove(book.bookId);
        await refreshLibrary();
      } catch (e) {
        setError(describeError(e));
      }
    },
    [refreshLibrary],
  );

  // ---- reading interactions ----
  const changePage = useCallback(
    (nextPage: number) => {
      const book = currentRef.current;
      if (!book) return;
      const clamped = Math.min(book.doc.pageCount - 1, Math.max(0, nextPage));
      setPage(clamped);
      if (mode === "paged") scheduleSave(book.bookId, clamped, 0);
    },
    [mode, scheduleSave],
  );

  const handleProgress = useCallback(
    (progressPage: number, offset: number) => {
      const book = currentRef.current;
      if (!book) return;
      scheduleSave(book.bookId, progressPage, offset);
    },
    [scheduleSave],
  );

  // ---- narration (read aloud) ----
  const narration = useNarration({
    docId: current?.doc.docId ?? -1,
    page,
    pageCount: current?.doc.pageCount ?? 0,
    getText: (target) => {
      const book = currentRef.current;
      if (!book) return Promise.reject(new Error("no book open"));
      return cachedPageText(book.doc.docId, target).then((t) => t.chars);
    },
    onAdvancePage: changePage,
  });

  // Hydrate narration settings once persisted settings are loaded.
  useEffect(() => {
    if (!ready) return;
    if (narrationVoice) narration.setVoiceURI(narrationVoice);
    narration.setRate(narrationRate);
    narration.setContinuous(narrationContinuous);
    narration.setCodeHandling(narrationCodeHandling);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready]);

  // Persist narration choices as they change.
  useEffect(() => {
    if (narration.voiceURI) persist("narrationVoice", narration.voiceURI);
  }, [narration.voiceURI, persist]);
  useEffect(() => {
    persist("narrationRate", narration.rate);
  persist("narrationCodeHandling", narration.codeHandling);
  }, [narration.rate, persist]);
  useEffect(() => {
    persist("narrationContinuous", narration.continuous);
  }, [narration.continuous, persist]);

  /** Jump to a page (and fractional position) from sidebar/search actions.
   *  Jumps are recorded so Back can return to where you were reading. The
   *  serial bumps on every jump so the Reader scrolls even when the target
   *  equals the current page state (which otherwise no-ops in React). */
  const [jumpSerial, setJumpSerial] = useState(0);
  const jumpTo = useCallback(
    (target: number, offset = 0, opts?: { push?: boolean }) => {
      if (opts?.push !== false) {
        setHistory((h) => [...h.slice(-49), positionRef.current]);
        setFuture([]);
      }
      setJumpSerial((s) => s + 1);
      changePage(target);
      if (mode === "scroll" && offset > 0) {
        // The reader applies offsets only on document restore; approximate a
        // within-page jump by scrolling after the page settles.
        window.setTimeout(() => {
          const container = document.querySelector<HTMLElement>(".reader.scroll-mode");
          const pages = container?.querySelectorAll<HTMLElement>(".lazy-page");
          const el = pages?.[target];
          if (container && el) {
            container.scrollTop = el.offsetTop - 12 + offset * el.offsetHeight;
          }
        }, 60);
      }
    },
    [changePage, mode],
  );

  const goBack = useCallback(() => {
    if (history.length === 0) return;
    const target = history[history.length - 1];
    setHistory((h) => h.slice(0, -1));
    setFuture((f) => [...f.slice(-49), positionRef.current]);
    jumpTo(target.page, target.offset, { push: false });
  }, [history, jumpTo]);

  const goForward = useCallback(() => {
    if (future.length === 0) return;
    const target = future[future.length - 1];
    setFuture((f) => f.slice(0, -1));
    setHistory((h) => [...h.slice(-49), positionRef.current]);
    jumpTo(target.page, target.offset, { push: false });
  }, [future, jumpTo]);

  // ---- bookmarks ----
  const addBookmarkNow = useCallback(async () => {
    const book = currentRef.current;
    if (!book || book.bookId < 0) return;
    const offset = positionRef.current.page === page ? positionRef.current.offset : 0;
    try {
      await bookmarksAdd(book.bookId, page, offset, `Page ${page + 1}`);
      setBookmarks(await bookmarksList(book.bookId));
      setSidebarTab("bookmarks");
    } catch (e) {
      setError(describeError(e));
    }
  }, [page]);

  const removeBookmark = useCallback(
    async (bookmark: Bookmark) => {
      const book = currentRef.current;
      if (!book) return;
      try {
        await bookmarksRemove(bookmark.id);
        setBookmarks(await bookmarksList(book.bookId));
      } catch (e) {
        setError(describeError(e));
      }
    },
    [],
  );

  // ---- highlights ----
  const addHighlight = useCallback(
    async (hlPage: number, start: number, end: number, text: string, color: HighlightColor) => {
      const book = currentRef.current;
      if (!book || book.bookId < 0) return;
      try {
        await highlightsAdd({
          bookId: book.bookId,
          page: hlPage,
          startChar: start,
          endChar: end,
          text,
          color,
          note: "",
        });
        setHighlights(await highlightsList(book.bookId));
      } catch (e) {
        setError(describeError(e));
      }
    },
    [],
  );

  const removeHighlight = useCallback(async (highlight: Highlight) => {
    const book = currentRef.current;
    if (!book) return;
    try {
      await highlightsRemove(highlight.id);
      setHighlights(await highlightsList(book.bookId));
    } catch (e) {
      setError(describeError(e));
    }
  }, []);

  const setHighlightColor = useCallback(
    async (highlight: Highlight, color: string) => {
      const book = currentRef.current;
      if (!book) return;
      try {
        await highlightsUpdate(highlight.id, color);
        setHighlights(await highlightsList(book.bookId));
      } catch (e) {
        setError(describeError(e));
      }
    },
    [],
  );

  const setHighlightNote = useCallback(
    async (highlight: Highlight, note: string) => {
      const book = currentRef.current;
      if (!book) return;
      try {
        await highlightsUpdate(highlight.id, undefined, note);
        setHighlights(await highlightsList(book.bookId));
      } catch (e) {
        setError(describeError(e));
      }
    },
    [],
  );

  // ---- search ----
  const runSearch = useCallback(
    (query: string, caseSensitive: boolean, wholeWords: boolean) => {
      const book = currentRef.current;
      if (searchTimer.current !== null) window.clearTimeout(searchTimer.current);
      if (!book || !query) {
        setSearch((s) => ({ ...s, results: [], active: 0, searching: false }));
        return;
      }
      setSearch((s) => ({ ...s, searching: true }));
      const seq = ++searchSeq.current;
      searchTimer.current = window.setTimeout(() => {
        searchDocument(book.doc.docId, query, caseSensitive, wholeWords)
          .then((results) => {
            if (searchSeq.current !== seq) return;
            setSearch((s) => ({ ...s, results, active: 0, searching: false }));
            if (results.length > 0) jumpTo(results[0].page);
          })
          .catch(() => {
            if (searchSeq.current === seq) {
              setSearch((s) => ({ ...s, results: [], active: 0, searching: false }));
            }
          });
      }, SEARCH_DEBOUNCE_MS);
    },
    [jumpTo],
  );

  // Toggling an option re-runs the search with the new flags.
  useEffect(() => {
    if (search.open && search.query) {
      runSearch(search.query, search.caseSensitive, search.wholeWords);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search.caseSensitive, search.wholeWords, search.open]);

  const stepSearch = useCallback(
    (direction: 1 | -1) => {
      setSearch((s) => {
        if (s.results.length === 0) return s;
        const next = (s.active + direction + s.results.length) % s.results.length;
        jumpTo(s.results[next].page);
        return { ...s, active: next };
      });
    },
    [jumpTo],
  );

  const selectSearchResult = useCallback(
    (index: number) => {
      setSearch((s) => (s.results[index] ? { ...s, active: index } : s));
      const hit = search.results[index];
      if (hit) jumpTo(hit.page);
    },
    [jumpTo, search.results],
  );

  // Snippets for the result list: the text around each match, built from
  // the cached page text (one fetch per page containing hits).
  const [snippets, setSnippets] = useState<Map<number, [string, string, string]>>(new Map());
  useEffect(() => {
    let cancelled = false;
    const book = currentRef.current;
    if (!book || search.results.length === 0) {
      setSnippets(new Map());
      return;
    }
    const byPage = new Map<number, { index: number; hit: SearchMatch }[]>();
    search.results.forEach((hit, index) => {
      const list = byPage.get(hit.page);
      if (list) list.push({ index, hit });
      else byPage.set(hit.page, [{ index, hit }]);
    });
    const build = async () => {
      const out = new Map<number, [string, string, string]>();
      for (const [pageIndex, hits] of byPage) {
        try {
          const text = await cachedPageText(book.doc.docId, pageIndex);
          if (cancelled) return;
          for (const { index, hit } of hits) {
            const chars = text.chars;
            const before = chars
              .slice(Math.max(0, hit.start - 32), hit.start)
              .replace(/\s+/g, " ");
            const matched = chars.slice(hit.start, hit.end).replace(/\s+/g, " ");
            const after = chars.slice(hit.end, hit.end + 32).replace(/\s+/g, " ");
            out.set(index, [before, matched, after]);
          }
        } catch {
          /* the row renders without a snippet */
        }
      }
      if (!cancelled) setSnippets(out);
    };
    void build();
    return () => {
      cancelled = true;
    };
  }, [search.results]);

  // Fetch remaining page dimensions in the background for long documents.
  useEffect(() => {
    if (!current) return;
    let cancelled = false;
    const doc = current.doc;
    const fill = async () => {
      let from = doc.pages.length;
      while (!cancelled && from < doc.pageCount) {
        const to = Math.min(from + DIM_BATCH, doc.pageCount);
        try {
          const batch = await getPageDims(doc.docId, from, to);
          if (cancelled || batch.length === 0) return;
          setDims((prev) => (prev.length === from ? [...prev, ...batch] : prev));
        } catch {
          return;
        }
        from = to;
      }
    };
    void fill();
    return () => {
      cancelled = true;
    };
  }, [current]);

  // Keyboard shortcuts (reader view only)
  useEffect(() => {
    if (!current || passwordFor || view !== "reader") return;
    const handler = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      const typing = target.tagName === "INPUT" || target.tagName === "TEXTAREA";
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        setSearch((s) => ({ ...s, open: true }));
        return;
      }
      if (typing) return;
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "b") {
        e.preventDefault();
        void addBookmarkNow();
        return;
      }
      const last = current.doc.pageCount - 1;
      const step = mode === "paged" && spread ? 2 : 1;
      if (e.altKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
        e.preventDefault();
        if (e.key === "ArrowLeft") goBack();
        else goForward();
        return;
      }
      switch (e.key) {
        case "Escape":
          if (search.open) setSearch((s) => ({ ...s, open: false }));
          else goLibrary();
          break;
        case "PageDown":
        case "ArrowRight":
        case " ":
          setPage((p) => {
            const next = Math.min(last, p + (mode === "paged" ? step : 1));
            if (next !== p && mode === "paged") scheduleSave(current.bookId, next, 0);
            return next;
          });
          e.preventDefault();
          break;
        case "PageUp":
        case "ArrowLeft":
          setPage((p) => {
            const next = Math.max(0, p - (mode === "paged" ? step : 1));
            if (next !== p && mode === "paged") scheduleSave(current.bookId, next, 0);
            return next;
          });
          e.preventDefault();
          break;
        case "Home":
          setPage(0);
          break;
        case "End":
          setPage(last);
          break;
        case "+":
        case "=":
          changeZoom(
            zoom.kind === "percent"
              ? { kind: "percent", value: Math.min(600, zoom.value * 1.25) }
              : { kind: "percent", value: 125 },
          );
          break;
        case "-":
          changeZoom(
            zoom.kind === "percent"
              ? { kind: "percent", value: Math.max(25, zoom.value / 1.25) }
              : zoom,
          );
          break;
        case "o":
          if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            void pickFile();
          }
          break;
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [
    current,
    passwordFor,
    view,
    mode,
    zoom,
    spread,
    search.open,
    goLibrary,
    goBack,
    goForward,
    pickFile,
    scheduleSave,
    changeZoom,
    addBookmarkNow,
  ]);

  if (!ready) {
    return <div className="boot-screen" />;
  }

  const activeHit = search.open && search.results.length > 0
    ? hitKey(search.results[search.active])
    : null;

  return (
    <div className="app">
      {view === "reader" ? (
        <Toolbar
          pageCount={current?.doc.pageCount ?? 0}
          page={page}
          onPageChange={changePage}
          zoom={zoom}
          onZoomChange={changeZoom}
          onOpen={pickFile}
          onLibrary={goLibrary}
          onSettings={() => setSettingsOpen(true)}
          disabled={!current}
          sidebarOpen={sidebarTab !== null}
          onToggleSidebar={() => setSidebarTab((t) => (t === null ? "outline" : null))}
          onSearch={() => setSearch((s) => ({ ...s, open: true }))}
          onNarration={() => narration.play(page)}
          narrationActive={narration.speaking}
          onAddBookmark={() => void addBookmarkNow()}
          onBack={goBack}
          onForward={goForward}
          canBack={history.length > 0}
          canForward={future.length > 0}
        />
      ) : (
        <header className="toolbar library-bar">
          <span className="library-bar-title">Bookshelf PDF</span>
        </header>
      )}

      {error && (
        <div className="banner error" role="alert">
          <span>{error}</span>
          <button className="btn small" onClick={() => setError(null)}>
            Dismiss
          </button>
        </div>
      )}

      {view === "reader" && current ? (
        <div className="reader-row">
          {sidebarTab !== null && (
            <div className="sidebar-wrap" style={{ width: sidebarWidth }}>
              <Sidebar
              tab={sidebarTab}
              onTabChange={setSidebarTab}
              doc={current.doc}
              dims={dims}
              page={page}
              onJumpPage={jumpTo}
              outline={outline}
              bookmarks={bookmarks}
              onAddBookmark={() => void addBookmarkNow()}
              onRemoveBookmark={(b) => void removeBookmark(b)}
              highlights={highlights}
              onRemoveHighlight={(h) => void removeHighlight(h)}
              onHighlightColorChange={(h, c) => void setHighlightColor(h, c)}
                onHighlightNoteChange={(h, n) => void setHighlightNote(h, n)}
              />
            </div>
          )}
          {sidebarTab !== null && <Resizer onResize={resizeSidebar} />}
          <div className="reader-stack">
            <Reader
              doc={current.doc}
              mode={mode}
              dims={dims}
              page={page}
              jumpSerial={jumpSerial}
              onPageChange={changePage}
              onProgress={handleProgress}
              resume={resume}
              zoom={zoom}
              theme={theme}
              spread={spread}
              highlights={highlights}
              searchHits={search.open ? search.results : []}
              activeHitKey={activeHit}
              spokenRange={narration.spokenRange}
              onAddHighlight={(p, s2, e2, text, color) => void addHighlight(p, s2, e2, text, color)}
              onJumpPage={(target) => jumpTo(target)}
              onJumpToHit={selectSearchResult}
            />
          </div>
          {search.open && (
            <aside className="search-dock">
              <SearchPanel
                query={search.query}
                onQueryChange={(query) => {
                  setSearch((s) => ({ ...s, query }));
                  runSearch(query, search.caseSensitive, search.wholeWords);
                }}
                caseSensitive={search.caseSensitive}
                wholeWords={search.wholeWords}
                onToggleCase={() => setSearch((s) => ({ ...s, caseSensitive: !s.caseSensitive }))}
                onToggleWholeWords={() => setSearch((s) => ({ ...s, wholeWords: !s.wholeWords }))}
                rows={search.results.map((match, index) => ({
                  match,
                  snippet: snippets.get(index) ?? null,
                }))}
                activeIndex={search.active}
                searching={search.searching}
                truncated={search.results.length >= SEARCH_RESULT_CAP}
                onSelect={selectSearchResult}
                onNext={() => stepSearch(1)}
                onPrev={() => stepSearch(-1)}
                onClose={() => setSearch((s) => ({ ...s, open: false }))}
              />
            </aside>
          )}
        </div>
      ) : (
        <Library
          books={library}
          onOpenBook={(book) => void openPath(book.path)}
          onOpenFile={pickFile}
          onRemoveBook={(book) => void removeBook(book)}
        />
      )}

      {settingsOpen && (
        <SettingsScreen
          mode={mode}
          onClose={() => setSettingsOpen(false)}
          onModeChange={changeMode}
          theme={theme}
          onThemeChange={changeTheme}
          narration={narration}
        />
      )}

      {view === "reader" && current && (
        <NarrationBar
          narration={narration}
          currentPage={page}
          pageCount={current.doc.pageCount}
        />
      )}

      {passwordFor && (
        <div className="modal-backdrop">
          <form
            className="modal"
            onSubmit={(e) => {
              e.preventDefault();
              void openPath(passwordFor.path, password);
            }}
          >
            <h2>{passwordFor.incorrect ? "Wrong password" : "Password required"}</h2>
            <p className="muted">{passwordFor.path}</p>
            <input
              type="password"
              autoFocus
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder="Document password"
            />
            <div className="modal-actions">
              <button
                type="button"
                className="btn"
                onClick={() => {
                  setPasswordFor(null);
                  setPassword("");
                }}
              >
                Cancel
              </button>
              <button type="submit" className="btn primary" disabled={!password}>
                Unlock
              </button>
            </div>
          </form>
        </div>
      )}
    </div>
  );
}
