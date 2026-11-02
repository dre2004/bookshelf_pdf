import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { cachedPageText, getPageLinks, pageImageUrl } from "./api";
import type { SpokenRange } from "./narration";
import { rangeBoxes, type OverlayBox } from "./textOverlay";
import { HIGHLIGHT_COLORS, type HighlightColor, type PageLink, type PageTextData } from "./types";
import type { DocumentInfo, Highlight, PageInfo, ReadMode, SearchMatch, Theme, ZoomMode } from "./types";

const PAD = 32;
const A4: PageInfo = { width: 595, height: 842 };

function dimsOf(dims: PageInfo[], index: number): PageInfo {
  return dims[index] ?? A4;
}

/** Round render width up to a 32px step so resize/zoom reuses cached renders. */
function renderWidth(cssWidth: number): number {
  const dpr = window.devicePixelRatio || 1;
  return Math.max(128, Math.ceil((cssWidth * dpr) / 32) * 32);
}

function useElementSize<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => {
      const rect = entry.contentRect;
      setSize({ width: rect.width, height: rect.height });
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  return [ref, size] as const;
}

/** Invisible, selectable copy of the page text aligned with the rendered image. */
function TextLayer({
  text,
  cssWidth,
  cssHeight,
}: {
  text: PageTextData;
  cssWidth: number;
  cssHeight: number;
}) {
  const spans = useMemo(() => {
    const sx = cssWidth / text.width;
    const sy = cssHeight / text.height;
    const out: ReactNode[] = [];
    for (let i = 0; i < text.chars.length; i++) {
      const w = text.boxes[i * 4 + 2] ?? 0;
      const h = text.boxes[i * 4 + 3] ?? 0;
      if (w <= 0 || h <= 0) continue;
      const x = (text.boxes[i * 4] ?? 0) * sx;
      const y = (text.height - ((text.boxes[i * 4 + 1] ?? 0) + h)) * sy;
      const style: CSSProperties = {
        left: `${x}px`,
        top: `${y}px`,
        width: `${w * sx}px`,
        height: `${h * sy}px`,
        fontSize: `${h * sy}px`,
        lineHeight: `${h * sy}px`,
      };
      out.push(
        <span key={i} data-i={i} style={style}>
          {text.chars[i] === " " ? "\u00a0" : text.chars[i]}
        </span>,
      );
    }
    return out;
  }, [text, cssWidth, cssHeight]);

  return <>{spans}</>;
}

function OverlayBoxes({ boxes, className }: { boxes: OverlayBox[]; className: string }) {
  return (
    <div className={`overlay ${className}`} aria-hidden="true">
      {boxes.map((box, i) => (
        <div
          key={i}
          className="overlay-box"
          style={{
            left: `${box.left}px`,
            top: `${box.top}px`,
            width: `${box.width}px`,
            height: `${box.height}px`,
          }}
        />
      ))}
    </div>
  );
}

/** Clickable in-page links (TOC entries, cross references, URIs), positioned
 *  with the same PDF-point → CSS transform the text layer uses. Sits above
 *  the text layer so link areas always activate; the rest of the page keeps
 *  its normal selection behavior. */
function LinkLayer({
  links,
  pageWidth,
  pageHeight,
  cssWidth,
  cssHeight,
  onJump,
}: {
  links: PageLink[];
  pageWidth: number;
  pageHeight: number;
  cssWidth: number;
  cssHeight: number;
  onJump: (page: number) => void;
}) {
  const boxes = useMemo(() => {
    if (!pageWidth || !pageHeight) return [];
    const sx = cssWidth / pageWidth;
    const sy = cssHeight / pageHeight;
    return links.map((link, i) => {
      const [x, y, w, h] = link.rect;
      return {
        key: i,
        style: {
          left: `${x * sx}px`,
          top: `${(pageHeight - (y + h)) * sy}px`,
          width: `${w * sx}px`,
          height: `${h * sy}px`,
        } as CSSProperties,
        link,
      };
    });
  }, [links, pageWidth, pageHeight, cssWidth, cssHeight]);

  return (
    <div className="link-layer">
      {boxes.map(({ key, style, link }) => (
        <a
          key={key}
          className="page-link"
          style={style}
          href={link.uri ?? "#"}
          target={link.uri ? "_blank" : undefined}
          rel={link.uri ? "noreferrer" : undefined}
          title={link.uri ?? `Go to page ${(link.destPage ?? 0) + 1}`}
          onClick={(e) => {
            if (link.destPage != null) {
              e.preventDefault();
              onJump(link.destPage);
            } else if (link.uri) {
              e.preventDefault();
              void openUrl(link.uri).catch(() => {});
            }
          }}
        />
      ))}
    </div>
  );
}

interface SelectionMenuState {
  x: number;
  y: number;
  page: number;
  start: number;
  end: number;
  text: string;
}

interface PageImageProps {
  docId: number;
  index: number;
  cssWidth: number;
  cssHeight: number;
  highlights: Highlight[];
  searchHits: SearchMatch[];
  activeHitKey: string | null;
  /** Character range currently being spoken on this page, if any. */
  spokenRange: { start: number; end: number } | null;
  onAddHighlight: (
    page: number,
    start: number,
    end: number,
    text: string,
    color: HighlightColor,
  ) => void;
  /** Jumps to a page (in-page link navigation). */
  onJumpPage: (page: number) => void;
}

function PageImage({
  docId,
  index,
  cssWidth,
  cssHeight,
  highlights,
  searchHits,
  activeHitKey,
  spokenRange,
  onAddHighlight,
  onJumpPage,
}: PageImageProps) {
  // Load status is keyed by URL (see the note at the <img> below).
  const [status, setStatus] = useState<{
    url: string;
    state: "loading" | "loaded" | "failed";
  } | null>(null);
  const url = pageImageUrl(docId, index, renderWidth(cssWidth));
  const state = status?.url === url ? status.state : "loading";

  const [text, setText] = useState<PageTextData | null>(null);
  useEffect(() => {
    let cancelled = false;
    cachedPageText(docId, index)
      .then((data) => {
        if (!cancelled) setText(data);
      })
      .catch(() => {
        /* page stays without a text layer; image still renders */
      });
    return () => {
      cancelled = true;
    };
  }, [docId, index]);

  const [links, setLinks] = useState<PageLink[]>([]);
  useEffect(() => {
    let cancelled = false;
    getPageLinks(docId, index)
      .then((data) => {
        if (!cancelled) setLinks(data);
      })
      .catch(() => {
        /* a page without link data just has no clickable links */
      });
    return () => {
      cancelled = true;
    };
  }, [docId, index]);

  const [menu, setMenu] = useState<SelectionMenuState | null>(null);

  const highlightBoxes = useMemo(() => {
    if (!text) return new Map<string, OverlayBox[]>();
    const byColor = new Map<string, OverlayBox[]>();
    for (const hl of highlights) {
      const boxes = rangeBoxes(text, hl.startChar, hl.endChar, cssWidth, cssHeight);
      byColor.set(
        `${hl.id}`,
        boxes,
      );
    }
    return byColor;
  }, [text, highlights, cssWidth, cssHeight]);

  const searchBoxes = useMemo(() => {
    if (!text) return { all: [] as OverlayBox[], active: [] as OverlayBox[] };
    const all: OverlayBox[] = [];
    const active: OverlayBox[] = [];
    for (const hit of searchHits) {
      const boxes = rangeBoxes(text, hit.start, hit.end, cssWidth, cssHeight);
      all.push(...boxes);
      if (activeHitKey === `${hit.page}:${hit.start}:${hit.end}`) active.push(...boxes);
    }
    return { all, active };
  }, [text, searchHits, activeHitKey, cssWidth, cssHeight]);

  const spokenBoxes = useMemo(() => {
    if (!text || !spokenRange) return [] as OverlayBox[];
    return rangeBoxes(text, spokenRange.start, spokenRange.end, cssWidth, cssHeight);
  }, [text, spokenRange, cssWidth, cssHeight]);

  const handleTextMouseUp = (e: React.MouseEvent) => {
    const selection = window.getSelection();
    if (!selection || selection.isCollapsed || !text) {
      setMenu(null);
      return;
    }
    const anchor = selection.anchorNode?.parentElement?.closest("[data-i]");
    const focus = selection.focusNode?.parentElement?.closest("[data-i]");
    const anchorIndex = anchor?.getAttribute("data-i");
    const focusIndex = focus?.getAttribute("data-i");
    if (anchorIndex == null || focusIndex == null) {
      setMenu(null);
      return;
    }
    const a = Number(anchorIndex);
    const f = Number(focusIndex);
    if (a === f) {
      setMenu(null);
      return;
    }
    const start = Math.min(a, f);
    const end = Math.max(a, f) + 1;
    const frame = (e.currentTarget as HTMLElement).closest(".page-frame") as HTMLElement;
    const rect = frame.getBoundingClientRect();
    setMenu({
      x: e.clientX - rect.left,
      y: e.clientY - rect.top - 44,
      page: index,
      start,
      end,
      text: selection.toString(),
    });
  };

  return (
    <div className="page-frame" style={{ width: cssWidth, height: cssHeight }}>
      {state === "loading" && <div className="page-spinner" aria-label="Rendering page" />}
      {state === "failed" && (
        <div className="page-error">
          <p>Failed to render page {index + 1}.</p>
          <p className="muted">Try zooming or reopening the document.</p>
        </div>
      )}
      {/* Load status is keyed by URL: events for a previous URL can never
          corrupt the current one, so cached images that fire `load` at any
          point relative to commits are handled by onLoad alone. Do NOT add a
          ref callback here — setState from an inline ref re-attaches it every
          render and loops (that blanked 0.2.0). */}
      <img
        key={url}
        src={url}
        width={Math.round(cssWidth)}
        height={Math.round(cssHeight)}
        draggable={false}
        alt=""
        decoding="async"
        className={`page-img ${state === "loaded" ? "visible" : ""}`}
        onLoad={() => setStatus({ url, state: "loaded" })}
        onError={() => setStatus({ url, state: "failed" })}
      />

      {[...highlightBoxes.entries()].map(([hlId, boxes]) => {
        const color = highlights.find((h) => `${h.id}` === hlId)?.color ?? "yellow";
        return <OverlayBoxes key={hlId} boxes={boxes} className={`hl hl-${color}`} />;
      })}
      <OverlayBoxes boxes={searchBoxes.all} className="search-hit" />
      <OverlayBoxes boxes={searchBoxes.active} className="search-hit active" />
      <OverlayBoxes boxes={spokenBoxes} className="narration-hit" />

      {text && (
        <div className="text-layer" onMouseUp={handleTextMouseUp}>
          <TextLayer text={text} cssWidth={cssWidth} cssHeight={cssHeight} />
        </div>
      )}

      {text && links.length > 0 && (
        <LinkLayer
          links={links}
          pageWidth={text.width}
          pageHeight={text.height}
          cssWidth={cssWidth}
          cssHeight={cssHeight}
          onJump={onJumpPage}
        />
      )}

      {menu && (
        <div className="selection-menu" style={{ left: menu.x, top: menu.y }} onMouseDown={(e) => e.preventDefault()}>
          {HIGHLIGHT_COLORS.map((color) => (
            <button
              key={color}
              className={`swatch swatch-${color}`}
              title={`Highlight ${color}`}
              onClick={() => {
                onAddHighlight(menu.page, menu.start, menu.end, menu.text, color);
                window.getSelection()?.removeAllRanges();
                setMenu(null);
              }}
            />
          ))}
          <button
            className="btn small"
            onClick={() => {
              void navigator.clipboard.writeText(menu.text);
              setMenu(null);
            }}
          >
            Copy
          </button>
        </div>
      )}
    </div>
  );
}

interface LazyPageProps extends PageImageProps {
  container: HTMLElement | null;
  registerEl: (index: number, el: HTMLDivElement | null) => void;
}

/** Deferred page in scroll mode: only renders once near the viewport. */
function LazyPage({ container, registerEl, index, ...image }: LazyPageProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [near, setNear] = useState(false);

  useEffect(() => {
    registerEl(index, ref.current);
    return () => registerEl(index, null);
  }, [index, registerEl]);

  useEffect(() => {
    if (near || !ref.current || !container) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) setNear(true);
      },
      { root: container, rootMargin: "1600px 0px" },
    );
    observer.observe(ref.current);
    return () => observer.disconnect();
  }, [container, near]);

  return (
    <div ref={ref} className="lazy-page">
      {near ? (
        <PageImage {...image} index={index} />
      ) : (
        <div
          className="page-frame placeholder"
          style={{ width: image.cssWidth, height: image.cssHeight }}
        />
      )}
    </div>
  );
}

export interface ReaderProps {
  doc: DocumentInfo;
  dims: PageInfo[];
  page: number;
  /** Increments on every sidebar/search jump, so a jump to the page the
   *  state already names still scrolls (React skips same-value updates). */
  jumpSerial: number;
  onPageChange: (page: number) => void;
  /** Called as reading position moves (page + position within the page 0..1). */
  onProgress: (page: number, offset: number) => void;
  /** Position to restore when this document first mounts. */
  resume: { page: number; offset: number } | null;
  mode: ReadMode;
  zoom: ZoomMode;
  theme: Theme;
  /** Two-page spread in paged mode. */
  spread: boolean;
  highlights: Highlight[];
  searchHits: SearchMatch[];
  activeHitKey: string | null;
  /** Character range being spoken, with its page (for the highlight). */
  spokenRange: SpokenRange | null;
  onAddHighlight: (
    page: number,
    start: number,
    end: number,
    text: string,
    color: HighlightColor,
  ) => void;
  /** Jumps to a page, recording history (in-page link navigation). */
  onJumpPage: (page: number) => void;
  /** Clicking a search marker on the scrollbar jumps to that hit. */
  onJumpToHit?: (index: number) => void;
}

/** Position of each search hit as a fraction of the scrollable content,
 *  for the scrollbar markers. -1 when the geometry isn't known yet.
 *  `pageMetrics` maps a page to its (offsetTop, height) in the scroll
 *  content; `hitYFractions` maps the hit's array index to its vertical
 *  fraction within its page (0 = top). Exported for unit testing. */
export function markerFractions(
  hits: SearchMatch[],
  pageMetrics: Map<number, { top: number; height: number }>,
  hitYFractions: Map<number, number>,
  scrollHeight: number,
): number[] {
  return hits.map((hit, index) => {
    const metrics = pageMetrics.get(hit.page);
    const yFraction = hitYFractions.get(index);
    if (!metrics || yFraction === undefined || scrollHeight <= 0) return -1;
    return (metrics.top + yFraction * metrics.height) / scrollHeight;
  });
}

export default function Reader({
  doc,
  dims,
  page,
  jumpSerial,
  onPageChange,
  onProgress,
  resume,
  mode,
  zoom,
  theme,
  spread,
  highlights,
  searchHits,
  activeHitKey,
  spokenRange,
  onAddHighlight,
  onJumpPage,
  onJumpToHit,
}: ReaderProps) {
  const [containerRef, viewport] = useElementSize<HTMLDivElement>();
  const [containerEl, setContainerEl] = useState<HTMLElement | null>(null);
  const pageEls = useRef(new Map<number, HTMLDivElement>());
  const lastReported = useRef(page);

  const fitDims = mode === "paged" ? dimsOf(dims, page) : dimsOf(dims, 0);

  const hitsByPage = useMemo(() => {
    const map = new Map<number, SearchMatch[]>();
    for (const hit of searchHits) {
      const list = map.get(hit.page);
      if (list) list.push(hit);
      else map.set(hit.page, [hit]);
    }
    return map;
  }, [searchHits]);

  const highlightsByPage = useMemo(() => {
    const map = new Map<number, Highlight[]>();
    for (const hl of highlights) {
      const list = map.get(hl.page);
      if (list) list.push(hl);
      else map.set(hl.page, [hl]);
    }
    return map;
  }, [highlights]);

  const scale = useMemo(() => {
    if (!viewport.width || !viewport.height) return 1;
    const columns = mode === "paged" && spread ? 2 : 1;
    const gap = columns === 2 ? 16 : 0;
    const availW = Math.max(80, viewport.width - PAD * 2 - gap);
    const availH = Math.max(80, viewport.height - PAD * 2);
    const fitWidthScale = availW / (fitDims.width * columns);
    switch (zoom.kind) {
      case "fitWidth":
        return fitWidthScale;
      case "fitPage":
        return Math.min(fitWidthScale, availH / fitDims.height);
      case "percent":
        return (zoom.value / 100) * fitWidthScale;
    }
  }, [viewport, zoom, fitDims.width, fitDims.height, mode, spread]);

  const registerEl = useRef((index: number, el: HTMLDivElement | null) => {
    if (el) pageEls.current.set(index, el);
    else pageEls.current.delete(index);
  }).current;

  // Restore the saved reading position once per document, waiting briefly
  // for the target page element to mount (page dims stream in async).
  const restoredDoc = useRef(-1);
  useEffect(() => {
    if (mode !== "scroll" || !resume) return;
    if (restoredDoc.current === doc.docId) return;
    restoredDoc.current = doc.docId;
    if (resume.page <= 0 && resume.offset <= 0) return;

    let tries = 0;
    // Bare setInterval (not window.*): this callback must survive a jsdom
    // test-environment teardown without throwing about `window`, and the
    // cleanup below runs on unmount — but a fired-after-teardown callback
    // is harmless when it can still clear itself.
    const timer = setInterval(() => {
      const el = pageEls.current.get(resume.page);
      if (el) {
        clearInterval(timer);
        const container = containerRef.current;
        if (container) {
          container.scrollTop = el.offsetTop - 12 + resume.offset * el.offsetHeight;
          lastReported.current = resume.page;
        }
      } else if (++tries > 60) {
        clearInterval(timer);
      }
    }, 50);
    return () => clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, doc.docId]);

  // Follow external page changes (toolbar, keyboard) by scrolling the page
  // into view; scroll-driven changes are suppressed via lastReported. A
  // jump (jumpSerial bump) always scrolls, even to the page the state
  // already names — the view can legitimately be elsewhere after a remount.
  const appliedJumpSerial = useRef(jumpSerial);
  useEffect(() => {
    if (mode !== "scroll") {
      lastReported.current = page;
      appliedJumpSerial.current = jumpSerial;
      return;
    }
    const isJump = jumpSerial !== appliedJumpSerial.current;
    appliedJumpSerial.current = jumpSerial;
    if (!isJump && page === lastReported.current) return;
    lastReported.current = page;
    pageEls.current.get(page)?.scrollIntoView({ block: "start", behavior: "auto" });
  }, [page, mode, jumpSerial]);

  const lastProgressSent = useRef(0);

  const handleScroll = () => {
    const container = containerRef.current;
    if (!container) return;
    const probe = container.scrollTop + container.clientHeight * 0.35;
    let current = 0;
    let currentTop = 0;
    let currentHeight = 1;
    for (const [index, el] of pageEls.current) {
      const top = el.offsetTop;
      if (probe >= top && probe < top + el.offsetHeight) {
        current = index;
        currentTop = top;
        currentHeight = el.offsetHeight;
        break;
      }
      if (top <= probe) {
        current = index;
        currentTop = top;
        currentHeight = el.offsetHeight;
      }
    }
    if (current !== lastReported.current) {
      lastReported.current = current;
      onPageChange(current);
    }
    const now = performance.now();
    if (now - lastProgressSent.current > 400) {
      lastProgressSent.current = now;
      onProgress(current, Math.min(1, Math.max(0, (probe - currentTop) / currentHeight)));
    }
  };

  // Warm the render cache for neighboring pages in paged mode so page
  // turns are instant; scroll mode already prefetches via IntersectionObserver.
  useEffect(() => {
    if (mode !== "paged") return;
    const neighbors = spread ? [page + 2, page + 3, page - 2, page + 1] : [page + 1, page + 2, page - 1];
    for (const neighbor of neighbors) {
      if (neighbor < 0 || neighbor >= doc.pageCount) continue;
      const d = dimsOf(dims, neighbor);
      const img = new Image();
      img.src = pageImageUrl(doc.docId, neighbor, renderWidth(d.width * scale));
    }
  }, [page, mode, spread, doc.docId, doc.pageCount, scale, dims]);

  const pageImageProps = (index: number) => ({
    docId: doc.docId,
    index,
    cssWidth: dimsOf(dims, index).width * scale,
    cssHeight: dimsOf(dims, index).height * scale,
    highlights: highlightsByPage.get(index) ?? [],
    searchHits: hitsByPage.get(index) ?? [],
    activeHitKey,
    spokenRange: spokenRange?.page === index
      ? { start: spokenRange.start, end: spokenRange.end }
      : null,
    onAddHighlight,
    onJumpPage,
  });

  if (mode === "paged") {
    const secondPage = spread && page + 1 < doc.pageCount ? page + 1 : null;
    return (
      <div className={`reader theme-${theme}`} ref={containerRef}>
        <div className="paged-stage">
          <PageImage {...pageImageProps(page)} />
          {secondPage !== null && <PageImage {...pageImageProps(secondPage)} />}
          {page > 0 && (
            <button
              className="nav-btn prev"
              onClick={() => onPageChange(page - (spread ? 2 : 1))}
              aria-label="Previous page"
            >
              ‹
            </button>
          )}
          {(secondPage !== null || page < doc.pageCount - 1) && (
            <button
              className="nav-btn next"
              onClick={() => onPageChange(page + (spread ? 2 : 1))}
              aria-label="Next page"
            >
              ›
            </button>
          )}
        </div>
      </div>
    );
  }

  return (
    <div
      className={`reader scroll-mode theme-${theme}`}
      ref={(el) => {
        containerRef.current = el;
        setContainerEl(el);
      }}
      onScroll={handleScroll}
    >
      <div className="scroll-pages">
        {Array.from({ length: doc.pageCount }, (_, i) => (
          <LazyPage key={i} {...pageImageProps(i)} container={containerEl} registerEl={registerEl} />
        ))}
      </div>
      {searchHits.length > 0 && (
        <SearchMarkers
          hits={searchHits}
          activeHitKey={activeHitKey}
          pageEls={pageEls.current}
          container={containerEl}
          dims={dims}
          docId={doc.docId}
          onJumpToHit={onJumpToHit}
        />
      )}
    </div>
  );
}

/** Scrollbar match markers: a strip along the right edge of the reader
 *  with one tick per search hit, positioned by the hit's fraction of the
 *  scrollable content. Clicking a tick jumps to that hit. */
function SearchMarkers({
  hits,
  activeHitKey,
  pageEls,
  container,
  dims,
  docId,
  onJumpToHit,
}: {
  hits: SearchMatch[];
  activeHitKey: string | null;
  pageEls: Map<number, HTMLDivElement>;
  container: HTMLElement | null;
  dims: PageInfo[];
  docId: number;
  onJumpToHit?: (index: number) => void;
}) {
  const [fractions, setFractions] = useState<number[]>([]);

  // Hit positions need two geometries: the hit's y inside its page (from
  // the page's character boxes) and the page's offset inside the scroll
  // content. Both settle asynchronously (text fetch, dims stream), so
  // recompute whenever inputs move; unknown positions stay -1.
  useEffect(() => {
    let cancelled = false;
    const compute = async () => {
      const hitYFractions = new Map<number, number>();
      const pages = [...new Set(hits.map((h) => h.page))];
      for (const pageIndex of pages) {
        try {
          const text = await cachedPageText(docId, pageIndex);
          if (cancelled) return;
          const height = Math.max(1, text.height);
          hits.forEach((hit, index) => {
            if (hit.page !== pageIndex) return;
            // boxes[i*4..] = (x, y-bottom-origin, w, h) in PDF points.
            const y = text.boxes[hit.start * 4 + 1] ?? 0;
            const h = text.boxes[hit.start * 4 + 3] ?? 0;
            const fromTop = 1 - (y + h) / height;
            hitYFractions.set(index, Math.min(1, Math.max(0, fromTop)));
          });
        } catch {
          /* no text layer for that page: those hits stay unplaced */
        }
      }
      const pageMetrics = new Map<number, { top: number; height: number }>();
      for (const pageIndex of pages) {
        const el = pageEls.get(pageIndex);
        if (el) pageMetrics.set(pageIndex, { top: el.offsetTop, height: el.offsetHeight });
      }
      const scrollHeight = container?.scrollHeight ?? 0;
      if (!cancelled) setFractions(markerFractions(hits, pageMetrics, hitYFractions, scrollHeight));
    };
    void compute();
    return () => {
      cancelled = true;
    };
  }, [hits, pageEls, container, dims, docId]);

  return (
    <div className="search-markers" role="toolbar" aria-label="Search match positions">
      {hits.map((hit, index) => {
        const fraction = fractions[index];
        if (fraction === undefined || fraction < 0) return null;
        const active = activeHitKey === `${hit.page}:${hit.start}:${hit.end}`;
        return (
          <button
            key={`${hit.page}:${hit.start}:${hit.end}`}
            className={`search-marker ${active ? "active" : ""}`}
            style={{ top: `${fraction * 100}%` }}
            onClick={() => onJumpToHit?.(index)}
            title={`Page ${hit.page + 1}`}
            aria-label={`Go to match on page ${hit.page + 1}`}
          />
        );
      })}
    </div>
  );
}
