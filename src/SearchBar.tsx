import { useEffect, useRef } from "react";

interface Props {
  query: string;
  onQueryChange: (query: string) => void;
  resultCount: number;
  activeIndex: number;
  onNext: () => void;
  onPrev: () => void;
  onClose: () => void;
  searching: boolean;
}

export default function SearchBar({
  query,
  onQueryChange,
  resultCount,
  activeIndex,
  onNext,
  onPrev,
  onClose,
  searching,
}: Props) {
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  return (
    <div className="search-bar">
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
        {searching ? "…" : resultCount > 0 ? `${activeIndex + 1}/${resultCount}` : query ? "0" : ""}
      </span>
      <button className="btn icon" onClick={onPrev} disabled={resultCount === 0} title="Previous match (Shift+Enter)">
        ‹
      </button>
      <button className="btn icon" onClick={onNext} disabled={resultCount === 0} title="Next match (Enter)">
        ›
      </button>
      <button className="btn icon" onClick={onClose} title="Close (Esc)">
        ✕
      </button>
    </div>
  );
}
