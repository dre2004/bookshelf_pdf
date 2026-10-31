import { useCallback, useEffect, useRef, useState } from "react";
import { ask, open as openFileDialog } from "@tauri-apps/plugin-dialog";
import Toolbar from "./Toolbar";
import Reader from "./Reader";
import Library from "./Library";
import {
  bookProgressSave,
  bookRegister,
  bookState,
  closeDocument,
  getPageDims,
  libraryList,
  libraryRemove,
  openDocument,
  settingsAll,
  settingsSet,
  toApiError,
} from "./api";
import type {
  BookSummary,
  DocumentInfo,
  PageInfo,
  ReadMode,
  Theme,
  ZoomMode,
} from "./types";
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

function describeError(e: unknown): string {
  const api = toApiError(e);
  if (api) {
    const label = api.code.replace(/_/g, " ");
    return api.reason ? `${label}: ${api.reason}` : label;
  }
  return String(e);
}

function parseStored<T>(raw: string | undefined, guard: (v: unknown) => T | null, fallback: T): T {
  if (!raw) return fallback;
  try {
    return guard(JSON.parse(raw)) ?? fallback;
  } catch {
    return fallback;
  }
}

function isReadMode(v: unknown): ReadMode | null {
  return v === "paged" || v === "scroll" ? v : null;
}

function isTheme(v: unknown): Theme | null {
  return v === "day" || v === "sepia" || v === "night" ? v : null;
}

function isZoom(v: unknown): ZoomMode | null {
  if (typeof v !== "object" || v === null) return null;
  const z = v as { kind?: unknown; value?: unknown };
  if (z.kind === "fitWidth" || z.kind === "fitPage") return { kind: z.kind };
  if (z.kind === "percent" && typeof z.value === "number" && z.value >= 25 && z.value <= 600) {
    return { kind: "percent", value: z.value };
  }
  return null;
}

export default function App() {
  const [view, setView] = useState<View>("library");
  const [library, setLibrary] = useState<BookSummary[]>([]);
  const [ready, setReady] = useState(false);

  const [current, setCurrent] = useState<CurrentBook | null>(null);
  const [dims, setDims] = useState<PageInfo[]>([]);
  const [page, setPage] = useState(0);
  const [resume, setResume] = useState<{ page: number; offset: number } | null>(null);

  const [mode, setMode] = useState<ReadMode>("paged");
  const [zoom, setZoom] = useState<ZoomMode>({ kind: "fitWidth" });
  const [theme, setTheme] = useState<Theme>("day");
  const settingsReady = useRef(false);

  const [error, setError] = useState<string | null>(null);
  const [passwordFor, setPasswordFor] = useState<{ path: string; incorrect: boolean } | null>(null);
  const [password, setPassword] = useState("");

  const currentRef = useRef<CurrentBook | null>(null);
  const saveTimer = useRef<number | null>(null);
  const lastProgress = useRef<{ bookId: number; page: number; offset: number } | null>(null);

  // ---- boot: hydrate persisted settings and the library ----
  useEffect(() => {
    const boot = async () => {
      try {
        const stored = await settingsAll();
        setMode(parseStored(stored.readMode, isReadMode, "paged"));
        setTheme(parseStored(stored.theme, isTheme, "day"));
        setZoom(parseStored(stored.zoom, isZoom, { kind: "fitWidth" }));
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
    };
    void boot();
  }, []);

  const persist = useCallback((key: string, value: unknown) => {
    if (!settingsReady.current) return;
    void settingsSet(key, JSON.stringify(value)).catch(() => {});
  }, []);

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

  const scheduleSave = useCallback(
    (bookId: number, nextPage: number, offset: number) => {
      if (bookId < 0) return;
      lastProgress.current = { bookId, page: nextPage, offset };
      if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(flushProgress, SAVE_DEBOUNCE_MS);
    },
    [flushProgress],
  );

  // Flush any pending progress when the app is hidden or closed.
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
        setPasswordFor(null);
        setPassword("");
        setView("reader");
        if (previous) void closeDocument(previous.doc.docId).catch(() => {});
        void refreshLibrary();
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

  const goLibrary = useCallback(() => {
    flushProgress();
    setView("library");
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

  // Keyboard navigation (reader view only)
  useEffect(() => {
    if (!current || passwordFor || view !== "reader") return;
    const handler = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      if (target.tagName === "INPUT" || target.tagName === "TEXTAREA") return;
      const last = current.doc.pageCount - 1;
      switch (e.key) {
        case "Escape":
          goLibrary();
          break;
        case "PageDown":
        case "ArrowRight":
        case " ":
          setPage((p) => {
            const next = Math.min(last, p + 1);
            if (next !== p && mode === "paged") scheduleSave(current.bookId, next, 0);
            return next;
          });
          e.preventDefault();
          break;
        case "PageUp":
        case "ArrowLeft":
          setPage((p) => {
            const next = Math.max(0, p - 1);
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
  }, [current, passwordFor, view, mode, zoom, changeZoom, goLibrary, pickFile, scheduleSave]);

  if (!ready) {
    return <div className="boot-screen" />;
  }

  return (
    <div className="app">
      {view === "reader" ? (
        <Toolbar
          fileName={current?.title ?? null}
          pageCount={current?.doc.pageCount ?? 0}
          page={page}
          onPageChange={changePage}
          mode={mode}
          onModeChange={changeMode}
          zoom={zoom}
          onZoomChange={changeZoom}
          theme={theme}
          onThemeChange={changeTheme}
          onOpen={pickFile}
          onLibrary={goLibrary}
          disabled={!current}
        />
      ) : (
        <header className="toolbar library-bar">
          <span className="library-bar-title">bookshelf_pdf</span>
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
        <Reader
          doc={current.doc}
          dims={dims}
          page={page}
          onPageChange={changePage}
          onProgress={handleProgress}
          resume={resume}
          mode={mode}
          zoom={zoom}
          theme={theme}
        />
      ) : (
        <Library
          books={library}
          onOpenBook={(book) => void openPath(book.path)}
          onOpenFile={pickFile}
          onRemoveBook={(book) => void removeBook(book)}
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
