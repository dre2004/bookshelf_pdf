import { useEffect, useState } from "react";
import type { ReadMode, Theme, ZoomMode } from "./types";

const ZOOM_STEPS = [50, 75, 100, 125, 150, 175, 200, 300, 400, 600];

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
}

function zoomLabel(zoom: ZoomMode): string {
  switch (zoom.kind) {
    case "fitWidth":
      return "Fit width";
    case "fitPage":
      return "Fit page";
    case "percent":
      return `${zoom.value}%`;
  }
}

function stepZoom(zoom: ZoomMode, direction: 1 | -1): ZoomMode {
  // Fit modes sit at the 100% step so stepping up/down moves from there.
  const currentPercent = zoom.kind === "percent" ? zoom.value : 100;
  let index = ZOOM_STEPS.findIndex((s) => s >= currentPercent);
  if (index === -1) index = ZOOM_STEPS.length - 1;
  const next = Math.min(ZOOM_STEPS.length - 1, Math.max(0, index + direction));
  return { kind: "percent", value: ZOOM_STEPS[next] };
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
        <button className="btn" onClick={onLibrary} title="Back to library (Esc)">
          ‹ Library
        </button>
        <button className="btn" onClick={onOpen} title="Open a PDF (Ctrl+O)">
          Open
        </button>
        {fileName && <span className="file-name" title={fileName}>{fileName}</span>}
      </div>

      <div className="toolbar-group">
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
