import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { pageImageUrl } from "./api";
import type { DocumentInfo, PageInfo, ReadMode, Theme, ZoomMode } from "./types";

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

interface PageImageProps {
  docId: number;
  index: number;
  cssWidth: number;
  cssHeight: number;
}

function PageImage({ docId, index, cssWidth, cssHeight }: PageImageProps) {
  const [state, setState] = useState<"loading" | "loaded" | "failed">("loading");
  const url = pageImageUrl(docId, index, renderWidth(cssWidth));
  useEffect(() => {
    setState("loading");
  }, [url]);

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
        src={url}
        width={Math.round(cssWidth)}
        height={Math.round(cssHeight)}
        draggable={false}
        alt=""
        className={`page-img ${state === "loaded" ? "visible" : ""}`}
        onLoad={() => setState("loaded")}
        onError={() => setState("failed")}
      />
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

interface ReaderProps {
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
}: ReaderProps) {
  const [containerRef, viewport] = useElementSize<HTMLDivElement>();
  const [containerEl, setContainerEl] = useState<HTMLElement | null>(null);
  const pageEls = useRef(new Map<number, HTMLDivElement>());
  const lastReported = useRef(page);

  const fitDims = mode === "paged" ? dimsOf(dims, page) : dimsOf(dims, 0);

  const scale = useMemo(() => {
    if (!viewport.width || !viewport.height) return 1;
    const availW = Math.max(80, viewport.width - PAD * 2);
    const availH = Math.max(80, viewport.height - PAD * 2);
    const fitWidthScale = availW / fitDims.width;
    switch (zoom.kind) {
      case "fitWidth":
        return fitWidthScale;
      case "fitPage":
        return Math.min(fitWidthScale, availH / fitDims.height);
      case "percent":
        return (zoom.value / 100) * fitWidthScale;
    }
  }, [viewport, zoom, fitDims.width, fitDims.height]);

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
    for (const neighbor of [page + 1, page + 2, page - 1]) {
      if (neighbor < 0 || neighbor >= doc.pageCount) continue;
      const d = dimsOf(dims, neighbor);
      const img = new Image();
      img.src = pageImageUrl(doc.docId, neighbor, renderWidth(d.width * scale));
    }
  }, [page, mode, doc.docId, doc.pageCount, scale, dims]);

  if (mode === "paged") {
    const d = dimsOf(dims, page);
    return (
      <div className={`reader theme-${theme}`} ref={containerRef}>
        <div className="paged-stage">
          <PageImage
            docId={doc.docId}
            index={page}
            cssWidth={d.width * scale}
            cssHeight={d.height * scale}
          />
          {page > 0 && (
            <button
              className="nav-btn prev"
              onClick={() => onPageChange(page - 1)}
              aria-label="Previous page"
            >
              ‹
            </button>
          )}
          {page < doc.pageCount - 1 && (
            <button
              className="nav-btn next"
              onClick={() => onPageChange(page + 1)}
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
        {Array.from({ length: doc.pageCount }, (_, i) => {
          const d = dimsOf(dims, i);
          return (
            <LazyPage
              key={i}
              index={i}
              docId={doc.docId}
              cssWidth={d.width * scale}
              cssHeight={d.height * scale}
              container={containerEl}
              registerEl={registerEl}
            />
          );
        })}
      </div>
    </div>
  );
}
