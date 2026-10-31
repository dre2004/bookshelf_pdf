import { useEffect, useRef, useState } from "react";
import { cachedPageText, pageImageUrl } from "./api";
import { snippetAround } from "./textOverlay";
import {
  HIGHLIGHT_COLORS,
  type Bookmark,
  type DocumentInfo,
  type Highlight,
  type OutlineNode,
  type PageInfo,
} from "./types";

export type SidebarTab = "outline" | "pages" | "bookmarks" | "highlights";

function OutlineTree({
  nodes,
  depth,
  currentPage,
  onJump,
}: {
  nodes: OutlineNode[];
  depth: number;
  currentPage: number;
  onJump: (page: number) => void;
}) {
  return (
    <ul className="outline-list">
      {nodes.map((node, i) => (
        <li key={`${depth}-${i}`}>
          <button
            className={`outline-item ${node.page === currentPage ? "current" : ""}`}
            style={{ paddingLeft: 10 + depth * 14 }}
            disabled={node.page < 0}
            title={node.page >= 0 ? `Page ${node.page + 1}` : "No page target"}
            onClick={() => node.page >= 0 && onJump(node.page)}
          >
            <span className="outline-title">{node.title}</span>
            {node.page >= 0 && <span className="outline-page">{node.page + 1}</span>}
          </button>
          {node.children.length > 0 && (
            <OutlineTree
              nodes={node.children}
              depth={depth + 1}
              currentPage={currentPage}
              onJump={onJump}
            />
          )}
        </li>
      ))}
    </ul>
  );
}

function Thumbnail({
  docId,
  index,
  aspect,
  container,
  current,
  onJump,
}: {
  docId: number;
  index: number;
  aspect: number;
  container: HTMLElement | null;
  current: boolean;
  onJump: (page: number) => void;
}) {
  const ref = useRef<HTMLButtonElement>(null);
  const [src, setSrc] = useState<string | null>(null);

  useEffect(() => {
    if (src || !ref.current || !container) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) setSrc(pageImageUrl(docId, index, 140));
      },
      { root: container, rootMargin: "600px 0px" },
    );
    observer.observe(ref.current);
    return () => observer.disconnect();
  }, [container, src, docId, index]);

  return (
    <button
      ref={ref}
      className={`thumbnail ${current ? "current" : ""}`}
      onClick={() => onJump(index)}
      title={`Page ${index + 1}`}
    >
      {src ? (
        <img src={src} alt="" draggable={false} />
      ) : (
        <div className="thumbnail-placeholder" style={{ aspectRatio: `1 / ${aspect}` }} />
      )}
      <span>{index + 1}</span>
    </button>
  );
}

/** Highlight text enriched with page snippets once the page text arrives. */
function HighlightRow({ hl, docId }: { hl: Highlight; docId: number }) {
  const [snippet, setSnippet] = useState(hl.text);
  useEffect(() => {
    let cancelled = false;
    cachedPageText(docId, hl.page)
      .then((text) => {
        if (!cancelled) setSnippet(snippetAround(text, hl.startChar, hl.endChar));
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [docId, hl.page, hl.startChar, hl.endChar]);

  return <span className={`hl-chip hl-${hl.color}`}>{snippet}</span>;
}

interface Props {
  tab: SidebarTab;
  onTabChange: (tab: SidebarTab) => void;
  doc: DocumentInfo | null;
  dims: PageInfo[];
  page: number;
  onJumpPage: (page: number, offset?: number) => void;
  outline: OutlineNode[];
  bookmarks: Bookmark[];
  onAddBookmark: () => void;
  onRemoveBookmark: (bookmark: Bookmark) => void;
  highlights: Highlight[];
  onRemoveHighlight: (highlight: Highlight) => void;
  onHighlightColorChange: (highlight: Highlight, color: string) => void;
  onHighlightNoteChange: (highlight: Highlight, note: string) => void;
}

const TABS: { id: SidebarTab; label: string }[] = [
  { id: "outline", label: "Contents" },
  { id: "pages", label: "Pages" },
  { id: "bookmarks", label: "Marks" },
  { id: "highlights", label: "Notes" },
];

export default function Sidebar({
  tab,
  onTabChange,
  doc,
  dims,
  page,
  onJumpPage,
  outline,
  bookmarks,
  onAddBookmark,
  onRemoveBookmark,
  highlights,
  onRemoveHighlight,
  onHighlightColorChange,
  onHighlightNoteChange,
}: Props) {
  const [scrollEl, setScrollEl] = useState<HTMLElement | null>(null);

  /** Jump to the nearest bookmark before/after the current page, wrapping. */
  const jumpBookmark = (direction: 1 | -1) => {
    const pages = bookmarks.map((b) => b.page);
    if (pages.length === 0) return;
    const target =
      direction === 1
        ? pages.find((p) => p > page) ?? pages[0]
        : [...pages].reverse().find((p) => p < page) ?? pages[pages.length - 1];
    onJumpPage(target);
  };

  return (
    <aside className="sidebar">
      <div className="sidebar-tabs" role="tablist">
        {TABS.map((t) => (
          <button
            key={t.id}
            role="tab"
            aria-selected={tab === t.id}
            className={`sidebar-tab ${tab === t.id ? "active" : ""}`}
            onClick={() => onTabChange(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>

      <div
        className="sidebar-body"
        ref={(el) => {
          setScrollEl(el);
        }}
      >
        {!doc || tab === "outline" ? (
          outline.length > 0 ? (
            <OutlineTree
              nodes={outline}
              depth={0}
              currentPage={page}
              onJump={(p) => onJumpPage(p)}
            />
          ) : (
            <p className="sidebar-empty muted">This document has no table of contents.</p>
          )
        ) : tab === "pages" ? (
          <div className="thumbnail-grid">
            {Array.from({ length: doc.pageCount }, (_, i) => {
              const d = dims[i] ?? dims[0] ?? { width: 595, height: 842 };
              return (
                <Thumbnail
                  key={i}
                  docId={doc.docId}
                  index={i}
                  aspect={d.height / d.width}
                  container={scrollEl}
                  current={i === page}
                  onJump={(p) => onJumpPage(p)}
                />
              );
            })}
          </div>
        ) : tab === "bookmarks" ? (
          <>
            <div className="bookmark-nav">
              <button
                className="btn small"
                disabled={bookmarks.length === 0}
                onClick={() => jumpBookmark(-1)}
                title="Previous bookmark"
              >
                ‹ Prev mark
              </button>
              <button
                className="btn small"
                disabled={bookmarks.length === 0}
                onClick={() => jumpBookmark(1)}
                title="Next bookmark"
              >
                Next mark ›
              </button>
            </div>
            <button className="btn primary sidebar-action" onClick={onAddBookmark}>
              + Bookmark page {page + 1}
            </button>
            {bookmarks.length === 0 ? (
              <p className="sidebar-empty muted">
                No bookmarks yet. Press Ctrl+B or use the button above to mark your place.
              </p>
            ) : (
              <ul className="bookmark-list">
                {bookmarks.map((b) => (
                  <li key={b.id} className="bookmark-row">
                    <button className="bookmark-jump" onClick={() => onJumpPage(b.page, b.offset)}>
                      <span className="bookmark-title">{b.title}</span>
                      <span className="muted">page {b.page + 1}</span>
                    </button>
                    <button
                      className="icon-remove"
                      title="Delete bookmark"
                      onClick={() => onRemoveBookmark(b)}
                    >
                      ✕
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </>
        ) : (
          <>
            {highlights.length === 0 ? (
              <p className="sidebar-empty muted">
                Select text on a page and pick a color to highlight it. Notes can be attached
                below each highlight.
              </p>
            ) : (
              <ul className="highlight-list">
                {highlights.map((hl) => (
                  <li key={hl.id} className="highlight-row">
                    <button className="highlight-jump" onClick={() => onJumpPage(hl.page)}>
                      <HighlightRow hl={hl} docId={doc.docId} />
                    </button>
                    <div className="highlight-tools">
                      {HIGHLIGHT_COLORS.map((color) => (
                        <button
                          key={color}
                          className={`swatch swatch-${color} ${hl.color === color ? "active" : ""}`}
                          title={`Set ${color}`}
                          onClick={() => onHighlightColorChange(hl, color)}
                        />
                      ))}
                      <button
                        className="icon-remove"
                        title="Delete highlight"
                        onClick={() => onRemoveHighlight(hl)}
                      >
                        ✕
                      </button>
                    </div>
                    <textarea
                      className="highlight-note"
                      placeholder="Add a note…"
                      defaultValue={hl.note}
                      key={hl.id}
                      onBlur={(e) => {
                        if (e.target.value !== hl.note) onHighlightNoteChange(hl, e.target.value);
                      }}
                    />
                  </li>
                ))}
              </ul>
            )}
          </>
        )}
      </div>
    </aside>
  );
}
