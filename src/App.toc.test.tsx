// @vitest-environment jsdom
// Regression test for the "sidebar links don't work after reopening the
// same book from the library" bug. Root cause was two-fold: re-entering
// the open book skipped position restore (view remounts at the top while
// the page state keeps its old value), and a chapter click targeting that
// stale state page was a silent React no-op. Exercises the full user path:
// open -> library -> reopen -> click the chapter the state already names.
import { describe, expect, it, vi, beforeAll, beforeEach } from "vitest";
import { render, screen, act, waitFor } from "@testing-library/react";
import React from "react";

const pageDims = Array.from({ length: 40 }, () => ({ width: 595, height: 842 }));
const doc = {
  docId: 7,
  pageCount: 40,
  pages: pageDims,
};

const api = vi.hoisted(() => ({
  bookState: vi.fn().mockResolvedValue({ lastPage: 2, lastOffset: 0.5 }),
  getOutline: vi.fn().mockResolvedValue([
    { title: "Chapter Two", page: 5, children: [] },
  ]),
}));

vi.mock("./api", () => ({
  openDocument: vi.fn().mockResolvedValue(doc),
  closeDocument: vi.fn().mockResolvedValue(undefined),
  getPageDims: vi.fn().mockResolvedValue(pageDims),
  getPageText: vi.fn().mockResolvedValue({ chars: "Hello", boxes: [1, 2, 3, 4], width: 595, height: 842 }),
  cachedPageText: vi.fn().mockResolvedValue({ chars: "Hello", boxes: [1, 2, 3, 4], width: 595, height: 842 }),
  pageImageUrl: vi.fn(() => "http://pdfimg.localhost/page/7/0/800"),
  coverUrl: vi.fn(() => "http://pdfimg.localhost/cover/1"),
  bookRegister: vi.fn().mockResolvedValue(1),
  bookState: api.bookState,
  bookProgressSave: vi.fn().mockResolvedValue(undefined),
  libraryList: vi.fn().mockResolvedValue([
    {
      bookId: 1,
      path: "C:\\books\\test.pdf",
      title: "test.pdf",
      pageCount: 40,
      lastPage: 2,
      lastOffset: 0.5,
      progressPct: 5,
      lastOpenedAt: 1,
      hasCover: true,
    },
  ]),
  libraryRemove: vi.fn().mockResolvedValue(undefined),
  settingsAll: vi.fn().mockResolvedValue({}),
  settingsSet: vi.fn().mockResolvedValue(undefined),
  getStartupFile: vi.fn().mockResolvedValue(null),
  getOutline: api.getOutline,
  searchDocument: vi.fn().mockResolvedValue([]),
  bookmarksAdd: vi.fn().mockResolvedValue({ id: 1, page: 0, offset: 0, title: "t", createdAt: 0 }),
  bookmarksList: vi.fn().mockResolvedValue([]),
  bookmarksRemove: vi.fn().mockResolvedValue(undefined),
  highlightsAdd: vi.fn().mockResolvedValue({}),
  highlightsList: vi.fn().mockResolvedValue([]),
  highlightsRemove: vi.fn().mockResolvedValue(undefined),
  highlightsUpdate: vi.fn().mockResolvedValue(undefined),
  ttsVoices: vi.fn().mockResolvedValue([]),
  ttsSpeak: vi.fn().mockResolvedValue(undefined),
  ttsPause: vi.fn().mockResolvedValue(undefined),
  ttsResume: vi.fn().mockResolvedValue(undefined),
  ttsStop: vi.fn().mockResolvedValue(undefined),
  ttsInstallKokoro: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn().mockResolvedValue(null),
  ask: vi.fn().mockResolvedValue(true),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({
    setTitle: vi.fn().mockResolvedValue(undefined),
  })),
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
  vi.stubGlobal("ResizeObserver", FakeObserver);
  vi.stubGlobal("IntersectionObserver", FakeObserver);
  vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: false, addEventListener() {}, removeEventListener() {} })));
  Element.prototype.scrollIntoView = vi.fn();
});

beforeEach(() => {
  vi.clearAllMocks();
  api.bookState.mockResolvedValue({ lastPage: 2, lastOffset: 0.5 });
});

describe("reopening the same book from the library", () => {
  it("restores the latest position and still scrolls on a same-page chapter click", async () => {
    const { default: App } = await import("./App");
    let container: HTMLElement | undefined;
    await act(async () => {
      container = render(React.createElement(App)).container;
    });
    const scrollSpy = Element.prototype.scrollIntoView as unknown as ReturnType<typeof vi.fn>;

    // Open the book.
    await screen.findAllByText("test.pdf");
    await act(async () => {
      container!.querySelector<HTMLElement>(".shelf-cover-btn")!.click();
    });
    await waitFor(() => expect(container!.querySelector(".page-frame")).toBeTruthy());
    const pageInput = () => container!.querySelector<HTMLInputElement>(".page-input")!;
    await waitFor(() => expect(pageInput().value).toBe("3")); // lastPage 2 (0-based)

    // Back to the library, saved position moves on to page 5 meanwhile.
    await act(async () => {
      [...container!.querySelectorAll<HTMLElement>("button")]
        .find((b) => (b.title || "").includes("Back to library"))!
        .click();
    });
    await waitFor(() => expect(container!.querySelector(".library")).toBeTruthy());
    api.bookState.mockResolvedValue({ lastPage: 5, lastOffset: 0.1 });

    // Reopen the same book: the re-entry branch must fetch state again...
    await act(async () => {
      container!.querySelector<HTMLElement>(".shelf-cover-btn")!.click();
    });
    await waitFor(() => expect(container!.querySelector(".page-frame")).toBeTruthy());
    // ...and land on the latest saved page, not a view/state desync.
    await waitFor(() => expect(pageInput().value).toBe("6"));
    expect(api.bookState.mock.calls.length).toBeGreaterThanOrEqual(2);

    // Open the sidebar and click the chapter pointing at the page the
    // state already names (5) — the exact previously-silent click.
    scrollSpy.mockClear();
    await act(async () => {
      [...container!.querySelectorAll<HTMLElement>("button")]
        .find((b) => (b.title || "").includes("Toggle sidebar"))!
        .click();
    });
    const chapter = await waitFor(() => {
      const item = [...container!.querySelectorAll<HTMLElement>(".outline-item")]
        .find((b) => b.textContent?.includes("Chapter Two"));
      expect(item).toBeTruthy();
      return item!;
    });
    await act(async () => {
      chapter.click();
    });
    await waitFor(() => expect(scrollSpy).toHaveBeenCalled());
  });
});
