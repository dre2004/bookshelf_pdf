// @vitest-environment jsdom
// Regression test for the "sidebar link to the chapter I'm already on does
// nothing" bug: React skips same-value state updates, so a jump needs its
// own serial to make the Reader scroll.
import { describe, expect, it, vi, beforeAll } from "vitest";
import { render } from "@testing-library/react";
import Reader from "./Reader";
import type { DocumentInfo, PageInfo } from "./types";

vi.mock("./api", () => ({
  pageImageUrl: vi.fn(() => "http://pdfimg.localhost/page/1/0/800"),
  cachedPageText: vi.fn().mockResolvedValue({
    chars: "x",
    boxes: [1, 2, 3, 4],
    width: 595,
    height: 842,
  }),
  getPageLinks: vi.fn().mockResolvedValue([]),
}));

beforeAll(() => {
  class FakeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
    takeRecords() {
      return [];
    }
  }
  vi.stubGlobal("IntersectionObserver", FakeObserver);
  vi.stubGlobal("ResizeObserver", FakeObserver);
  vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: false, addEventListener() {}, removeEventListener() {} })));
  Element.prototype.scrollIntoView = vi.fn();
});

const dims: PageInfo[] = Array.from({ length: 10 }, () => ({ width: 595, height: 842 }));
const doc: DocumentInfo = { docId: 1, pageCount: 10, pages: dims };

function props(page: number, jumpSerial: number) {
  return {
    doc,
    dims,
    page,
    jumpSerial,
    onPageChange: vi.fn(),
    onProgress: vi.fn(),
    resume: null,
    mode: "scroll" as const,
    zoom: { kind: "fitWidth" } as const,
    theme: "day" as const,
    spread: false,
    highlights: [],
    searchHits: [],
    activeHitKey: null,
    spokenRange: null,
    onAddHighlight: vi.fn(),
    onJumpPage: vi.fn(),
  };
}

describe("Reader jump scrolling", () => {
  it("scrolls on a jump to the page the state already names", () => {
    const spy = Element.prototype.scrollIntoView as unknown as ReturnType<typeof vi.fn>;
    const { rerender } = render(<Reader {...props(3, 0)} />);
    // Mount with resume=null must not scroll on its own.
    expect(spy).not.toHaveBeenCalled();

    // Same page, new jump serial: must scroll (this was the silent no-op).
    rerender(<Reader {...props(3, 1)} />);
    expect(spy).toHaveBeenCalledTimes(1);

    // No page change and no serial bump: nothing new.
    rerender(<Reader {...props(3, 1)} />);
    expect(spy).toHaveBeenCalledTimes(1);

    // A page change still scrolls without a serial bump (toolbar/keyboard).
    rerender(<Reader {...props(4, 1)} />);
    expect(spy).toHaveBeenCalledTimes(2);
  });
});
