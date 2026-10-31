import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { cachedPageText, pageImageUrl } from "./api";
import { rangeBoxes, type OverlayBox } from "./textOverlay";
import { HIGHLIGHT_COLORS, type HighlightColor, type PageTextData } from "./types";
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
  onAddHighlight: (
    page: number,
    start: number,
    end: number,
    text: string,
    color: HighlightColor,
  ) => void;
}

function PageImage({
  docId,
  index,
  cssWidth,
  cssHeight,
  highlights,
  searchHits,
  activeHitKey,
  onAddHighlight,
}: PageImageProps) {
  // Status is keyed by URL: a cached image can fire `load` before React
  // commits, so a plain reset-on-url-change state machine races and can
  // leave the spinner stuck. Stale events for an old URL are ignored, and
  // the ref check below catches images that finished before this mounted.
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
      <img
        key={url}
        ref={(el) => {
          if (el?.complete && el.naturalWidth > 0) setStatus({ url, state: "loaded" });
        }}
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

      {text && (
        <div className="text-layer" onMouseUp={handleTextMouseUp}>
          <TextLayer text={text} cssWidth={cssWidth} cssHeight={cssHeight} />
        </div>
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
  onAddHighlight: (
    page: number,
    start: number,
    end: number,
    text: string,
    color: HighlightColor,
  ) => void;
}

export default function Reader({
  doc,
  dims,
  page,
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
  onAddHighlight,
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
    const timer = window.setInterval(() => {
      const el = pageEls.current.get(resume.page);
      if (el) {
        window.clearInterval(timer);
        const container = containerRef.current;
        if (container) {
          container.scrollTop = el.offsetTop - 12 + resume.offset * el.offsetHeight;
          lastReported.current = resume.page;
        }
      } else if (++tries > 60) {
        window.clearInterval(timer);
      }
    }, 50);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, doc.docId]);

  // Follow external page changes (toolbar, keyboard) by scrolling the page
  // into view; scroll-driven changes are suppressed via lastReported.
  useEffect(() => {
    if (mode !== "scroll") {
      lastReported.current = page;
      return;
    }
    if (page === lastReported.current) return;
    lastReported.current = page;
    pageEls.current.get(page)?.scrollIntoView({ block: "start", behavior: "auto" });
  }, [page, mode]);

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
    onAddHighlight,
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
    </div>
  );
}
