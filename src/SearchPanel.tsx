import { useEffect, useRef } from "react";
import type { SearchMatch } from "./types";

/** One result row: the match plus its context snippet, when the page
 *  text has loaded (before/hit/after). */
export interface SearchRow {
  match: SearchMatch;
  snippet: [string, string, string] | null;
}

interface Props {
  query: string;
  onQueryChange: (query: string) => void;
  caseSensitive: boolean;
  wholeWords: boolean;
  onToggleCase: () => void;
  onToggleWholeWords: () => void;
  rows: SearchRow[];
  activeIndex: number;
  searching: boolean;
  /** The backend caps results; surface it when the cap was hit. */
  truncated: boolean;
  onSelect: (index: number) => void;
  onNext: () => void;
  onPrev: () => void;
  onClose: () => void;
}

export default function SearchPanel({
  query,
  onQueryChange,
  caseSensitive,
  wholeWords,
  onToggleCase,
  onToggleWholeWords,
  rows,
  activeIndex,
  searching,
  truncated,
  onSelect,
  onNext,
  onPrev,
  onClose,
}: Props) {
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // Keep the active row visible as the user cycles with Enter.
  useEffect(() => {
    listRef.current
      ?.querySelector(`[data-index="${activeIndex}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [activeIndex]);

  return (
    <div className="search-panel">
      <div className="search-panel-head">
        <input
          ref={inputRef}
          value={query}
          placeholder="Search in document…"
          onChange={(e) => onQueryChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.shiftKey ? onPrev() : onNext();
            } else if (e.key === "Escape") {
              onClose();
            }
            e.stopPropagation();
          }}
        />
        <span className="search-count muted">
          {searching
            ? "…"
            : rows.length > 0
              ? `${activeIndex + 1}/${rows.length}`
              : query
                ? "0"
                : ""}
        </span>
        <button className="btn icon" onClick={onPrev} disabled={rows.length === 0} title="Previous match (Shift+Enter)">
          ‹
        </button>
        <button className="btn icon" onClick={onNext} disabled={rows.length === 0} title="Next match (Enter)">
          ›
        </button>
        <button className="btn icon" onClick={onClose} title="Close (Esc)">
          ✕
        </button>
      </div>
      <div className="search-panel-toggles">
        <button
          className={`btn small toggle ${caseSensitive ? "on" : ""}`}
          onClick={onToggleCase}
          title="Match case"
          aria-pressed={caseSensitive}
        >
          Aa
        </button>
        <button
          className={`btn small toggle ${wholeWords ? "on" : ""}`}
          onClick={onToggleWholeWords}
          title="Whole words"
          aria-pressed={wholeWords}
        >
          ab|
        </button>
      </div>
      <div className="search-panel-results" ref={listRef}>
        {rows.length === 0 && (
          <p className="muted search-empty">
            {query ? (searching ? "Searching…" : "No matches.") : "Type to search the document."}
          </p>
        )}
        {rows.map((row, index) => (
          <button
            key={`${row.match.page}:${row.match.start}:${row.match.end}`}
            data-index={index}
            className={`search-result ${index === activeIndex ? "active" : ""}`}
            onClick={() => onSelect(index)}
          >
            <span className="search-result-page muted">p. {row.match.page + 1}</span>
            {row.snippet ? (
              <span className="search-result-snippet">
                <span className="muted">…{row.snippet[0]}</span>
                <mark>{row.snippet[1]}</mark>
                <span className="muted">{row.snippet[2]}…</span>
              </span>
            ) : (
              <span className="muted">…</span>
            )}
          </button>
        ))}
        {truncated && rows.length > 0 && (
          <p className="muted search-truncated">
            Result limit reached — refine the query to see more.
          </p>
        )}
      </div>
    </div>
  );
}
