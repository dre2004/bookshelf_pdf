import { useEffect, useState } from "react";
import type { ZoomMode } from "./types";
import { stepZoom, zoomLabel } from "./zoom";

interface Props {
  pageCount: number;
  page: number;
  onPageChange: (page: number) => void;
  zoom: ZoomMode;
  onZoomChange: (zoom: ZoomMode) => void;
  onOpen: () => void;
  onLibrary: () => void;
  onSettings: () => void;
  disabled: boolean;
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  onSearch: () => void;
  onNarration: () => void;
  onNarrationSetup: () => void;
  narrationActive: boolean;
  narrationSetupNeeded: boolean;
  onAddBookmark: () => void;
  onBack: () => void;
  onForward: () => void;
  canBack: boolean;
  canForward: boolean;
}

export default function Toolbar({
  pageCount,
  page,
  onPageChange,
  zoom,
  onZoomChange,
  onOpen,
  onLibrary,
  onSettings,
  disabled,
  sidebarOpen,
  onToggleSidebar,
  onSearch,
  onNarration,
  onNarrationSetup,
  narrationActive,
  narrationSetupNeeded,
  onAddBookmark,
  onBack,
  onForward,
  canBack,
  canForward,
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
      </div>

      <div className="toolbar-group">
        <button className="btn" disabled={disabled} onClick={onSearch} title="Search in document (Ctrl+F)">
          Search
        </button>
        {narrationSetupNeeded ? (
          <button
            className="btn"
            disabled={disabled}
            onClick={onNarrationSetup}
            title="Narration needs a one-time download (~87 MB) — set it up now"
          >
            🔊 Set up narration
          </button>
        ) : (
          <button
            className={`btn ${narrationActive ? "active" : ""}`}
            disabled={disabled}
            onClick={onNarration}
            title="Narration — read the book aloud"
          >
            🔊 Narrate
          </button>
        )}
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


      <div className="toolbar-spacer" />
      <div className="toolbar-group">
        <button className="btn icon" onClick={onSettings} title="Settings">
          ⚙
        </button>
      </div>
    </header>
  );
}
