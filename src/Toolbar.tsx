import { useEffect, useState } from "react";
import type { ReadMode, Theme, ZoomMode } from "./types";
import { stepZoom, zoomLabel } from "./zoom";

interface Props {
  fileName: string | null;
  pageCount: number;
  page: number;
  onPageChange: (page: number) => void;
  mode: ReadMode;
  onModeChange: (mode: ReadMode) => void;
  zoom: ZoomMode;
  onZoomChange: (zoom: ZoomMode) => void;
  theme: Theme;
  onThemeChange: (theme: Theme) => void;
  onOpen: () => void;
  onLibrary: () => void;
  disabled: boolean;
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  onSearch: () => void;
  onAddBookmark: () => void;
  onBack: () => void;
  onForward: () => void;
  canBack: boolean;
  canForward: boolean;
  spread: boolean;
  onSpreadChange: (spread: boolean) => void;
}

export default function Toolbar({
  fileName,
  pageCount,
  page,
  onPageChange,
  mode,
  onModeChange,
  zoom,
  onZoomChange,
  theme,
  onThemeChange,
  onOpen,
  onLibrary,
  disabled,
  sidebarOpen,
  onToggleSidebar,
  onSearch,
  onAddBookmark,
  onBack,
  onForward,
  canBack,
  canForward,
  spread,
  onSpreadChange,
}: Props) {
  const [pageInput, setPageInput] = useState(String(page + 1));

  useEffect(() => {
    setPageInput(String(page + 1));
  }, [page]);

  const commitPage = (raw: string) => {
    const parsed = Number.parseInt(raw, 10);
    if (!Number.isNaN(parsed)) {
      onPageChange(Math.min(pageCount - 1, Math.max(0, parsed - 1)));
    } else {
      setPageInput(String(page + 1));
    }
  };

  return (
    <header className="toolbar">
      <div className="toolbar-group">
        <button
          className={`btn icon wide ${sidebarOpen ? "active" : ""}`}
          disabled={disabled}
          onClick={onToggleSidebar}
          title="Toggle sidebar (contents, pages, bookmarks, notes)"
        >
          ☰ Sidebar
        </button>
        <button className="btn" onClick={onLibrary} title="Back to library (Esc)">
          ‹ Library
        </button>
        <button className="btn" onClick={onOpen} title="Open a PDF (Ctrl+O)">
          Open
        </button>
        {fileName && <span className="file-name" title={fileName}>{fileName}</span>}
      </div>

      <div className="toolbar-group">
        <button className="btn" disabled={disabled} onClick={onSearch} title="Search in document (Ctrl+F)">
          Search
        </button>
        <button className="btn" disabled={disabled} onClick={onAddBookmark} title="Bookmark this position (Ctrl+B)">
          ⚑ Bookmark
        </button>
      </div>

      <div className="toolbar-group">
        <button
          className="btn icon"
          disabled={!canBack}
          onClick={onBack}
          title="Back to where you were reading (Alt+←)"
        >
          ←
        </button>
        <button
          className="btn icon"
          disabled={!canForward}
          onClick={onForward}
          title="Forward (Alt+→)"
        >
          →
        </button>
        <button className="btn icon" disabled={disabled} onClick={() => onPageChange(page - 1)} title="Previous page (Page Up / ←)">
          ‹
        </button>
        <input
          className="page-input"
          value={pageInput}
          disabled={disabled}
          onChange={(e) => setPageInput(e.target.value)}
          onBlur={(e) => commitPage(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          }}
          aria-label="Page number"
        />
        <span className="page-count">/ {pageCount || "–"}</span>
        <button className="btn icon" disabled={disabled} onClick={() => onPageChange(page + 1)} title="Next page (Page Down / →)">
          ›
        </button>
      </div>

      <div className="toolbar-group">
        <button className="btn icon" disabled={disabled} onClick={() => onZoomChange(stepZoom(zoom, -1))} title="Zoom out (−)">
          −
        </button>
        <button
          className="btn zoom-mode"
          disabled={disabled}
          onClick={() =>
            onZoomChange(zoom.kind === "fitWidth" ? { kind: "fitPage" } : { kind: "fitWidth" })
          }
          title="Toggle fit width / fit page"
        >
          {zoomLabel(zoom)}
        </button>
        <button className="btn icon" disabled={disabled} onClick={() => onZoomChange(stepZoom(zoom, 1))} title="Zoom in (+)">
          +
        </button>
      </div>

      <div className="toolbar-group">
        <button
          className={`btn ${mode === "paged" ? "active" : ""}`}
          disabled={disabled}
          onClick={() => onModeChange("paged")}
          title="Paged mode — one page at a time"
        >
          Paged
        </button>
        <button
          className={`btn ${mode === "scroll" ? "active" : ""}`}
          disabled={disabled}
          onClick={() => onModeChange("scroll")}
          title="Continuous scrolling mode"
        >
          Scroll
        </button>
        {mode === "paged" && (
          <button
            className={`btn ${spread ? "active" : ""}`}
            disabled={disabled}
            onClick={() => onSpreadChange(!spread)}
            title="Two-page spread"
          >
            ⧉
          </button>
        )}
      </div>

      <div className="toolbar-group">
        {(["day", "sepia", "night"] as Theme[]).map((t) => (
          <button
            key={t}
            className={`btn theme-${t} ${theme === t ? "active" : ""}`}
            onClick={() => onThemeChange(t)}
            title={`${t} theme`}
          >
            {t === "day" ? "☀" : t === "sepia" ? "◐" : "☾"}
          </button>
        ))}
      </div>
    </header>
  );
}
